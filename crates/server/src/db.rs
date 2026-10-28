//! The hub's Postgres: accounts, logins, machines, pairing codes and the boxes runs live in. Tokens
//! are stored as their sha256, so the database alone can't sign anyone in.

use crate::boxes::{BoxRow, Phase};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand_core::{OsRng, RngCore};
use pinned_core::sync;
use pinned_core::upstash::BoxId;
use pinned_core::wire::{AccountView, MachineView};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
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
create table if not exists project_files (account uuid not null references accounts on delete cascade, project text not null,
  path text not null, sha text not null, modified bigint not null, data bytea not null, primary key (account, project, path));
create table if not exists shared_projects (account uuid not null references accounts on delete cascade, key text not null,
  name text not null, repo text not null, updated timestamptz not null default now(), primary key (account, key));
create table if not exists boxes (id text primary key, account uuid not null references accounts on delete cascade,
  machine uuid not null, upstash text, connector text not null, subject jsonb not null, spec jsonb not null,
  size text not null, phase text not null, reason text, created_at timestamptz not null default now(),
  unique (account, connector, subject));
";
const BOX_COLUMNS: &str = "id, account, machine, upstash, connector, subject, spec, size, phase, reason, extract(epoch from now() - created_at)::float8";
/// Compressed bytes of synced project files one account may keep on the hub.
const SYNC_QUOTA: i64 = 1 << 30;

const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const CODE_LEN: usize = 8;
/// A login unused this long stops working; every use pushes it back.
const LOGIN_IDLE_DAYS: i32 = 90;

#[derive(Clone)]
pub struct Db {
    sealer: Sealer,
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
    /// The machine runs in one of the account's boxes, so it takes only that box's run.
    pub dedicated: bool,
}

pub struct Connectors {
    pub store: Value,
    pub secrets: BTreeMap<String, String>,
}

/// Seals the integrations' keys before they reach Postgres, so a dump or a backup of the
/// database alone doesn't give them away.
#[derive(Clone)]
pub struct Sealer(Arc<ChaCha20Poly1305>);

const SEALED: &str = "sealed:";

impl Sealer {
    /// The key in `file`, made on first use, readable only by the hub's user.
    pub fn load(file: &Path) -> Result<Sealer, String> {
        let key = match std::fs::read(file) {
            Ok(key) => key,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let key = ChaCha20Poly1305::generate_key(&mut OsRng).to_vec();
                if let Some(dir) = file.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
                }
                let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(file).map_err(|e| format!("could not create {}: {e}", file.display()))?;
                f.write_all(&key).map_err(|e| format!("could not write {}: {e}", file.display()))?;
                key
            }
            Err(e) => return Err(format!("could not read {}: {e}", file.display())),
        };
        let key = Key::from_exact_iter(key).ok_or_else(|| format!("{} does not hold a 32-byte key", file.display()))?;
        Ok(Sealer(Arc::new(ChaCha20Poly1305::new(&key))))
    }

    fn seal(&self, secret: &str) -> String {
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let mut sealed = nonce.to_vec();
        sealed.extend(self.0.encrypt(&nonce, secret.as_bytes()).expect("sealing a short secret cannot fail"));
        format!("{SEALED}{}", BASE64.encode(sealed))
    }

    /// A value written before sealing came in reads as it is, and is sealed when it is next saved.
    fn open(&self, value: &str) -> Result<String, String> {
        let Some(sealed) = value.strip_prefix(SEALED) else { return Ok(value.to_owned()) };
        let unreadable = || "a secret can't be opened with this hub's key file".to_owned();
        let sealed = BASE64.decode(sealed).map_err(|_| unreadable())?;
        if sealed.len() < 12 {
            return Err(unreadable());
        }
        let (nonce, ciphertext) = sealed.split_at(12);
        let secret = self.0.decrypt(Nonce::from_slice(nonce), ciphertext).map_err(|_| unreadable())?;
        String::from_utf8(secret).map_err(|_| unreadable())
    }
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

fn box_row(r: &tokio_postgres::Row) -> Result<BoxRow, String> {
    let id: String = r.get(0);
    let unreadable = |e: serde_json::Error| format!("box {id} has an unreadable row: {e}");
    Ok(BoxRow {
        account: r.get(1),
        machine: r.get(2),
        upstash: r.get::<_, Option<String>>(3).map(BoxId),
        connector: r.get(4),
        subject: serde_json::from_value(r.get(5)).map_err(unreadable)?,
        spec: serde_json::from_value(r.get(6)).map_err(unreadable)?,
        size: serde_json::from_value(Value::String(r.get(7))).map_err(unreadable)?,
        phase: Phase::load(r.get(8), r.get(9)),
        age: std::time::Duration::from_secs_f64(r.get::<_, f64>(10).max(0.0)),
        id,
    })
}

impl Db {
    pub async fn connect(url: &str, sealer: Sealer) -> Result<Db, String> {
        let client = open(url).await?;
        client.batch_execute(SCHEMA).await.map_err(|e| format!("could not create the tables: {e}"))?;
        Ok(Db { sealer, url: url.into(), client: Arc::new(tokio::sync::Mutex::new(Arc::new(client))) })
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

    pub async fn has_accounts(&self) -> Result<bool, String> {
        let row = self.client().await?.query_one("select exists (select 1 from accounts)", &[]).await.map_err(err)?;
        Ok(row.get(0))
    }

    pub async fn account(&self, account: Uuid) -> Result<Option<(AccountView, String)>, String> {
        let row = self.client().await?.query_opt("select email, password_hash from accounts where id = $1", &[&account]).await.map_err(err)?;
        Ok(row.map(|r| (AccountView { id: account.to_string(), email: r.get(0) }, r.get(1))))
    }

    pub async fn set_password(&self, account: Uuid, password_hash: &str) -> Result<(), String> {
        self.client().await?.execute("update accounts set password_hash = $2 where id = $1", &[&account, &password_hash]).await.map_err(err).map(drop)
    }

    pub async fn sync_list(&self, account: Uuid, project: &str) -> Result<Vec<sync::Entry>, String> {
        let rows = self
            .client().await?
            .query("select path, sha, modified, octet_length(data) from project_files where account = $1 and project = $2", &[&account, &project])
            .await
            .map_err(err)?;
        Ok(rows.iter().map(|r| sync::Entry { path: r.get(0), sha: r.get(1), modified: r.get::<_, i64>(2) as u64, size: r.get::<_, i32>(3) as u64 }).collect())
    }

    /// Keeps the files, unless they would take the account past its quota. Two pushes at once may
    /// pass it by a batch, which is fine for a guard against filling the disk.
    pub async fn sync_push(&self, account: Uuid, project: &str, files: Vec<(sync::Entry, Vec<u8>)>) -> Result<(), String> {
        let client = self.client().await?;
        let paths: Vec<&str> = files.iter().map(|(e, _)| e.path.as_str()).collect();
        let row = client
            .query_one(
                "select coalesce(sum(octet_length(data)), 0)::bigint, \
                 coalesce(sum(octet_length(data)) filter (where project = $2 and path = any($3)), 0)::bigint \
                 from project_files where account = $1",
                &[&account, &project, &paths],
            )
            .await
            .map_err(err)?;
        let (used, replaced): (i64, i64) = (row.get(0), row.get(1));
        let incoming: i64 = files.iter().map(|(_, data)| data.len() as i64).sum();
        if used - replaced + incoming > SYNC_QUOTA {
            return Err(format!("the account's synced projects would pass {} MB; delete old sessions to sync more", SYNC_QUOTA >> 20));
        }
        for (entry, data) in &files {
            client
                .execute(
                    "insert into project_files (account, project, path, sha, modified, data) values ($1, $2, $3, $4, $5, $6) \
                     on conflict (account, project, path) do update set sha = excluded.sha, modified = excluded.modified, data = excluded.data",
                    &[&account, &project, &entry.path, &entry.sha, &(entry.modified as i64), data],
                )
                .await
                .map_err(err)?;
        }
        Ok(())
    }

    pub async fn sync_pull(&self, account: Uuid, project: &str, paths: &[String]) -> Result<Vec<(sync::Entry, Vec<u8>)>, String> {
        let rows = self
            .client().await?
            .query("select path, sha, modified, data from project_files where account = $1 and project = $2 and path = any($3)", &[&account, &project, &paths])
            .await
            .map_err(err)?;
        Ok(rows
            .iter()
            .map(|r| {
                let data: Vec<u8> = r.get(3);
                (sync::Entry { path: r.get(0), sha: r.get(1), modified: r.get::<_, i64>(2) as u64, size: data.len() as u64 }, data)
            })
            .collect())
    }

    pub async fn share_project(&self, account: Uuid, project: &sync::Project) -> Result<(), String> {
        self.client().await?
            .execute(
                "insert into shared_projects (account, key, name, repo) values ($1, $2, $3, $4) \
                 on conflict (account, key) do update set name = excluded.name, repo = excluded.repo, updated = now()",
                &[&account, &project.key, &project.name, &project.repo],
            )
            .await
            .map_err(err)
            .map(drop)
    }

    pub async fn shared_projects(&self, account: Uuid) -> Result<Vec<sync::Project>, String> {
        let rows = self.client().await?.query("select key, name, repo from shared_projects where account = $1 order by updated desc", &[&account]).await.map_err(err)?;
        Ok(rows.iter().map(|r| sync::Project { key: r.get(0), name: r.get(1), repo: r.get(2) }).collect())
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
        let view = MachineView { id: id.to_string(), name: name.to_owned(), os: os.to_owned(), online: false, last_seen: row.get(0), upstash_box: None };
        Ok((view, token.text))
    }

    /// The machine a token belongs to, recording that it just connected from `os`.
    pub async fn machine_connected(&self, token: &str, os: &str) -> Result<Option<MachineRow>, String> {
        let row = self
            .client().await?
            .query_opt(
                "update machines set os = $2, last_seen = now() where token_hash = $1
                 returning id, account, name, exists (select 1 from boxes where boxes.machine = machines.id)",
                &[&hash(token), &os],
            )
            .await
            .map_err(err)?;
        Ok(row.map(|r| MachineRow { id: r.get(0), account: r.get(1), name: r.get(2), dedicated: r.get(3) }))
    }

    pub async fn machine_seen(&self, machine: Uuid) -> Result<(), String> {
        self.client().await?.execute("update machines set last_seen = now() where id = $1", &[&machine]).await.map_err(err).map(drop)
    }

    /// The account's machines in the order they joined, all offline: the hub knows which are online.
    pub async fn machines(&self, account: Uuid) -> Result<Vec<MachineView>, String> {
        let rows = self
            .client().await?
            .query(
                "select m.id, m.name, m.os, extract(epoch from coalesce(m.last_seen, m.created_at))::bigint, b.upstash from machines m
                 left join boxes b on b.machine = m.id where m.account = $1 order by m.created_at, m.id",
                &[&account],
            )
            .await
            .map_err(err)?;
        Ok(rows
            .iter()
            .map(|r| MachineView { id: r.get::<_, Uuid>(0).to_string(), name: r.get(1), os: r.get(2), online: false, last_seen: r.get(3), upstash_box: r.get(4) })
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

    /// Gives the machine `id` of the account a fresh token, adding it when it is new. The hub adds a
    /// box's machine itself, under the id its box row holds, so no pairing code is involved.
    pub async fn put_machine(&self, id: Uuid, account: Uuid, name: &str, os: &str) -> Result<String, String> {
        let token = Token::new();
        let row = self
            .client().await?
            .query_opt(
                "insert into machines (id, account, name, os, token_hash) values ($1, $2, $3, $4, $5)
                 on conflict (id) do update set token_hash = excluded.token_hash, name = excluded.name
                 where machines.account = excluded.account returning id",
                &[&id, &account, &name, &os, &token.hash],
            )
            .await
            .map_err(err)?;
        row.map(|_| token.text).ok_or_else(|| format!("machine {id} belongs to another account"))
    }

    /// Writes `row` unless the account has a box for its workflow's subject already: the stored
    /// row, and whether it is the one just written.
    pub async fn reserve_box(&self, row: &BoxRow) -> Result<(BoxRow, bool), String> {
        let client = self.client().await?;
        let (subject, spec) = (serde_json::to_value(&row.subject).unwrap(), serde_json::to_value(&row.spec).unwrap());
        let (phase, reason) = row.phase.stored();
        let size = serde_json::to_value(row.size).unwrap().as_str().unwrap().to_owned();
        let inserted = client
            .execute(
                "insert into boxes (id, account, machine, connector, subject, spec, size, phase, reason) values ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 on conflict (account, connector, subject) do nothing",
                &[&row.id, &row.account, &row.machine, &row.connector, &subject, &spec, &size, &phase, &reason],
            )
            .await
            .map_err(err)?;
        let stored = client
            .query_one(&format!("select {BOX_COLUMNS} from boxes where account = $1 and connector = $2 and subject = $3"), &[&row.account, &row.connector, &subject])
            .await
            .map_err(err)?;
        Ok((box_row(&stored)?, inserted == 1))
    }

    pub async fn boxes(&self, account: Uuid) -> Result<Vec<BoxRow>, String> {
        let rows = self.client().await?.query(&format!("select {BOX_COLUMNS} from boxes where account = $1 order by created_at"), &[&account]).await.map_err(err)?;
        rows.iter().map(box_row).collect()
    }

    pub async fn box_of(&self, account: Uuid, machine: Uuid) -> Result<Option<BoxRow>, String> {
        let row = self.client().await?.query_opt(&format!("select {BOX_COLUMNS} from boxes where account = $1 and machine = $2"), &[&account, &machine]).await.map_err(err)?;
        row.as_ref().map(box_row).transpose()
    }

    /// Moves the row from `from` to `to`, and answers false when it was no longer at `from`:
    /// another hub, as during a deploy, moved it first.
    pub async fn set_box_phase(&self, id: &str, from: &Phase, to: &Phase) -> Result<bool, String> {
        let ((from, _), (to, reason)) = (from.stored(), to.stored());
        let n = self
            .client().await?
            .execute("update boxes set phase = $3, reason = $4 where id = $1 and phase = $2", &[&id, &from, &to, &reason])
            .await
            .map_err(err)?;
        Ok(n == 1)
    }

    /// Records the row's box and moves it on to booting, as `set_box_phase` does.
    pub async fn box_created(&self, id: &str, upstash: &BoxId) -> Result<bool, String> {
        let (from, _) = Phase::Creating.stored();
        let (to, _) = Phase::Booting.stored();
        let n = self
            .client().await?
            .execute("update boxes set upstash = $3, phase = $4 where id = $1 and phase = $2", &[&id, &from, &upstash.0, &to])
            .await
            .map_err(err)?;
        Ok(n == 1)
    }

    pub async fn box_exists(&self, id: &str) -> Result<bool, String> {
        let row = self.client().await?.query_one("select exists (select 1 from boxes where id = $1)", &[&id]).await.map_err(err)?;
        Ok(row.get(0))
    }

    pub async fn delete_box(&self, id: &str) -> Result<(), String> {
        self.client().await?.execute("delete from boxes where id = $1", &[&id]).await.map_err(err).map(drop)
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
        let sealed: BTreeMap<String, String> = serde_json::from_value(row.get(1)).map_err(|e| format!("account {account} has unreadable secrets: {e}"))?;
        let secrets = sealed.into_iter().map(|(name, value)| Ok((name, self.sealer.open(&value)?))).collect::<Result<_, String>>()?;
        Ok(Connectors { store: row.get(0), secrets })
    }

    pub async fn accounts_with_connectors(&self) -> Result<Vec<Uuid>, String> {
        let rows = self.client().await?.query("select id from accounts where store <> '{}' or secrets <> '{}'", &[]).await.map_err(err)?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    pub async fn save_store(&self, account: Uuid, store: &Value) -> Result<(), String> {
        self.client().await?.execute("update accounts set store = $2 where id = $1", &[&account, store]).await.map_err(err).map(drop)
    }

    pub async fn save_secrets(&self, account: Uuid, secrets: &BTreeMap<String, String>) -> Result<(), String> {
        let sealed: BTreeMap<&String, String> = secrets.iter().map(|(name, value)| (name, self.sealer.seal(value))).collect();
        let sealed = serde_json::to_value(sealed).map_err(|e| e.to_string())?;
        self.client().await?.execute("update accounts set secrets = $2 where id = $1", &[&account, &sealed]).await.map_err(err).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_sealed_with_the_key_file_made_on_first_start() {
        let dir = std::env::temp_dir().join(format!("pinned-sealer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sealer = Sealer::load(&dir.join("hub.key")).unwrap();
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(dir.join("hub.key")).unwrap().permissions()) & 0o777, 0o600);
        let sealed = sealer.seal("lin_api_secret");
        assert!(sealed.starts_with("sealed:") && !sealed.contains("lin_api_secret"));
        assert_ne!(sealer.seal("lin_api_secret"), sealed, "every seal takes a fresh nonce");
        assert_eq!(Sealer::load(&dir.join("hub.key")).unwrap().open(&sealed), Ok("lin_api_secret".into()), "the key file is reused");
        assert_eq!(sealer.open("xoxb-written-before-sealing"), Ok("xoxb-written-before-sealing".into()));
        let other = Sealer::load(&dir.join("other.key")).unwrap();
        assert_eq!(other.open(&sealed), Err("a secret can't be opened with this hub's key file".into()));
        let _ = std::fs::remove_dir_all(dir);
    }

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
