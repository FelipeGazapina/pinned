//! The hub's Postgres: accounts, logins, machines and pairing codes. Tokens are stored as their
//! sha256, so the database alone can't sign anyone in.

use rand_core::{OsRng, RngCore};
use base64::Engine;
use pinned_core::wire::{AccountView, MachineView};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio_postgres::{Client, NoTls};
use uuid::Uuid;

const SCHEMA: &str = "
create table if not exists accounts (id uuid primary key, email text unique not null, password_hash text not null,
  created_at timestamptz not null default now(), store jsonb not null default '{}', secrets jsonb not null default '{}');
create table if not exists logins (token_hash bytea primary key, account uuid not null references accounts on delete cascade,
  created_at timestamptz not null default now(), last_used timestamptz);
create table if not exists machines (id uuid primary key, account uuid not null references accounts on delete cascade,
  name text not null, os text not null, token_hash bytea unique not null,
  created_at timestamptz not null default now(), last_seen timestamptz);
create table if not exists pair_codes (code text primary key, account uuid not null references accounts on delete cascade,
  expires_at timestamptz not null);
";

const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const CODE_LEN: usize = 8;
/// A login unused this long stops working; every use pushes it back.
const LOGIN_IDLE_DAYS: i32 = 90;

#[derive(Clone)]
pub struct Db {
    url: Arc<str>,
    /// Replaced when Postgres drops the connection, as on its restart, so the hub doesn't stay down.
    client: Arc<tokio::sync::Mutex<Arc<Client>>>,
}

pub struct Token {
    pub text: String,
    pub hash: Vec<u8>,
}

impl Token {
    pub fn new() -> Token {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let text = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        Token { hash: hash(&text), text }
    }
}

pub fn hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub fn pair_code() -> String {
    (0..CODE_LEN).map(|_| CODE_ALPHABET[OsRng.next_u32() as usize % CODE_ALPHABET.len()] as char).collect()
}

pub struct MachineRow {
    pub id: Uuid,
    pub account: Uuid,
    pub name: String,
}

pub struct Connectors {
    pub store: Value,
    pub secrets: BTreeMap<String, String>,
}

async fn open(url: &str) -> Result<Client, String> {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await.map_err(|e| format!("could not connect to Postgres: {e}"))?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("pinned-server: Postgres connection ended: {e}");
        }
    });
    Ok(client)
}

fn err(e: tokio_postgres::Error) -> String {
    e.to_string()
}

impl Db {
    pub async fn connect(url: &str) -> Result<Db, String> {
        let client = open(url).await?;
        client.batch_execute(SCHEMA).await.map_err(|e| format!("could not create the tables: {e}"))?;
        Ok(Db { url: url.into(), client: Arc::new(tokio::sync::Mutex::new(Arc::new(client))) })
    }

    async fn client(&self) -> Result<Arc<Client>, String> {
        let mut client = self.client.lock().await;
        if client.is_closed() {
            *client = Arc::new(open(&self.url).await?);
            eprintln!("pinned-server: reconnected to Postgres");
        }
        Ok(client.clone())
    }

    /// The new account, or None when the email is taken.
    pub async fn create_account(&self, email: &str, password_hash: &str) -> Result<Option<AccountView>, String> {
        let id = Uuid::new_v4();
        let inserted = self
            .client().await?
            .execute("insert into accounts (id, email, password_hash) values ($1, $2, $3) on conflict (email) do nothing", &[&id, &email, &password_hash])
            .await
            .map_err(err)?;
        Ok((inserted == 1).then(|| AccountView { id: id.to_string(), email: email.to_owned() }))
    }

    pub async fn account_by_email(&self, email: &str) -> Result<Option<(AccountView, String)>, String> {
        let row = self.client().await?.query_opt("select id, password_hash from accounts where email = $1", &[&email]).await.map_err(err)?;
        Ok(row.map(|r| (AccountView { id: r.get::<_, Uuid>(0).to_string(), email: email.to_owned() }, r.get(1))))
    }

    pub async fn email(&self, account: Uuid) -> Result<Option<String>, String> {
        let row = self.client().await?.query_opt("select email from accounts where id = $1", &[&account]).await.map_err(err)?;
        Ok(row.map(|r| r.get(0)))
    }

    pub async fn create_login(&self, account: Uuid) -> Result<String, String> {
        let token = Token::new();
        self.client().await?.execute("insert into logins (token_hash, account) values ($1, $2)", &[&token.hash, &account]).await.map_err(err)?;
        Ok(token.text)
    }

    /// The account a login token belongs to, marking the login used.
    pub async fn login(&self, token: &str) -> Result<Option<Uuid>, String> {
        let row = self
            .client().await?
            .query_opt(
                "update logins set last_used = now() where token_hash = $1 \
                 and coalesce(last_used, created_at) > now() - make_interval(days => $2) returning account",
                &[&hash(token), &LOGIN_IDLE_DAYS],
            )
            .await
            .map_err(err)?;
        Ok(row.map(|r| r.get(0)))
    }

    pub async fn logout(&self, token: &str) -> Result<(), String> {
        self.client().await?.execute("delete from logins where token_hash = $1", &[&hash(token)]).await.map_err(err).map(drop)
    }

    pub async fn create_machine(&self, account: Uuid, name: &str, os: &str) -> Result<(MachineView, String), String> {
        let (id, token) = (Uuid::new_v4(), Token::new());
        let row = self
            .client().await?
            .query_one(
                "insert into machines (id, account, name, os, token_hash) values ($1, $2, $3, $4, $5)
                 returning extract(epoch from created_at)::bigint",
                &[&id, &account, &name, &os, &token.hash],
            )
            .await
            .map_err(err)?;
        let view = MachineView { id: id.to_string(), name: name.to_owned(), os: os.to_owned(), online: false, last_seen: row.get(0) };
        Ok((view, token.text))
    }

    /// The machine a token belongs to, recording that it just connected from `os`.
    pub async fn machine_connected(&self, token: &str, os: &str) -> Result<Option<MachineRow>, String> {
        let row = self
            .client().await?
            .query_opt("update machines set os = $2, last_seen = now() where token_hash = $1 returning id, account, name", &[&hash(token), &os])
            .await
            .map_err(err)?;
        Ok(row.map(|r| MachineRow { id: r.get(0), account: r.get(1), name: r.get(2) }))
    }

    pub async fn machine_seen(&self, machine: Uuid) -> Result<(), String> {
        self.client().await?.execute("update machines set last_seen = now() where id = $1", &[&machine]).await.map_err(err).map(drop)
    }

    /// The account's machines in the order they joined, all offline: the hub knows which are online.
    pub async fn machines(&self, account: Uuid) -> Result<Vec<MachineView>, String> {
        let rows = self
            .client().await?
            .query(
                "select id, name, os, extract(epoch from coalesce(last_seen, created_at))::bigint from machines
                 where account = $1 order by created_at, id",
                &[&account],
            )
            .await
            .map_err(err)?;
        Ok(rows
            .iter()
            .map(|r| MachineView { id: r.get::<_, Uuid>(0).to_string(), name: r.get(1), os: r.get(2), online: false, last_seen: r.get(3) })
            .collect())
    }

    pub async fn machine_name(&self, account: Uuid, machine: Uuid) -> Result<Option<String>, String> {
        let row = self.client().await?.query_opt("select name from machines where id = $1 and account = $2", &[&machine, &account]).await.map_err(err)?;
        Ok(row.map(|r| r.get(0)))
    }

    pub async fn rename_machine(&self, account: Uuid, machine: Uuid, name: &str) -> Result<bool, String> {
        let n = self.client().await?.execute("update machines set name = $3 where id = $1 and account = $2", &[&machine, &account, &name]).await.map_err(err)?;
        Ok(n == 1)
    }

    pub async fn remove_machine(&self, account: Uuid, machine: Uuid) -> Result<bool, String> {
        let n = self.client().await?.execute("delete from machines where id = $1 and account = $2", &[&machine, &account]).await.map_err(err)?;
        Ok(n == 1)
    }

    /// A fresh pairing code for the account and when it expires, in Unix seconds.
    pub async fn create_pair_code(&self, account: Uuid) -> Result<(String, i64), String> {
        self.client().await?.execute("delete from pair_codes where expires_at < now()", &[]).await.map_err(err)?;
        loop {
            let code = pair_code();
            let row = self
                .client().await?
                .query_opt(
                    "insert into pair_codes (code, account, expires_at) values ($1, $2, now() + interval '15 minutes')
                     on conflict do nothing returning extract(epoch from expires_at)::bigint",
                    &[&code, &account],
                )
                .await
                .map_err(err)?;
            if let Some(row) = row {
                return Ok((code, row.get(0)));
            }
        }
    }

    /// Spends a pairing code: the account it was made for, or None when it is unknown, used or expired.
    pub async fn take_pair_code(&self, code: &str) -> Result<Option<(Uuid, String)>, String> {
        let row = self
            .client().await?
            .query_opt(
                "with taken as (delete from pair_codes where code = $1 returning account, expires_at)
                 select a.id, a.email from taken join accounts a on a.id = taken.account where taken.expires_at > now()",
                &[&code],
            )
            .await
            .map_err(err)?;
        Ok(row.map(|r| (r.get(0), r.get(1))))
    }

    pub async fn connectors(&self, account: Uuid) -> Result<Connectors, String> {
        let row = self.client().await?.query_one("select store, secrets from accounts where id = $1", &[&account]).await.map_err(err)?;
        let secrets: Value = row.get(1);
        let secrets = serde_json::from_value(secrets).unwrap_or_default();
        Ok(Connectors { store: row.get(0), secrets })
    }

    pub async fn accounts_with_connectors(&self) -> Result<Vec<Uuid>, String> {
        let rows = self.client().await?.query("select id from accounts where store <> '{}' or secrets <> '{}'", &[]).await.map_err(err)?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    pub async fn save_store(&self, account: Uuid, store: &Value) -> Result<(), String> {
        self.client().await?.execute("update accounts set store = $2 where id = $1", &[&account, store]).await.map_err(err).map(drop)
    }

    pub async fn save_secrets(&self, account: Uuid, secrets: &Value) -> Result<(), String> {
        self.client().await?.execute("update accounts set secrets = $2 where id = $1", &[&account, secrets]).await.map_err(err).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_codes_use_only_unambiguous_characters() {
        for _ in 0..200 {
            let code = pair_code();
            assert_eq!(code.len(), 8);
            assert!(code.bytes().all(|c| CODE_ALPHABET.contains(&c)), "{code}");
            assert!(!code.contains(['0', 'O', '1', 'I', 'L']), "{code}");
        }
    }

    #[test]
    fn a_token_is_32_random_bytes_kept_as_their_sha256() {
        let (a, b) = (Token::new(), Token::new());
        assert_eq!(a.text.len(), 43);
        assert_ne!(a.text, b.text);
        assert_eq!(a.hash, hash(&a.text));
        assert_eq!(a.hash.len(), 32);
    }
}
