//! The hub's Postgres: accounts, logins, machines, pairing codes and the boxes runs live in. Tokens
//! are stored as their sha256, so the database alone can't sign anyone in. A second, optional
//! database keeps what users chose to share to improve the app, such as rated dictations.

use crate::boxes::{BoxRow, BoxWork, Opener, Phase, Remains};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand_core::{OsRng, RngCore};
use pinned_core::account::{DictationEngine, DictationFeedback, DictationRating};
use pinned_core::agent::Subject;
use pinned_core::delegate::TaskRecord;
use pinned_core::marks::{self, MarkChange, SessionMarks};
use pinned_core::schedule::{Actor, Task, TaskRun};
use pinned_core::sync;
use pinned_core::taste::Tastes;
use pinned_core::upstash::BoxId;
use pinned_core::surface::{ActorRole, AuditEntry, AuditOutcome, Confirmation, Decision};
use pinned_core::wire::{AccountView, AgentProject, AgentView, InviteView, MachineView, NewInvite, NEVER_EXPIRES};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio_postgres::{Client, NoTls};
use uuid::Uuid;

const SCHEMA: &str = "
create table if not exists accounts (id uuid primary key, email text unique not null, password_hash text not null,
  created_at timestamptz not null default now(), store jsonb not null default '{}', secrets jsonb not null default '{}');
create table if not exists logins (token_hash bytea primary key, account uuid not null references accounts on delete cascade,
  created_at timestamptz not null default now(), last_used timestamptz);
create table if not exists machines (id uuid primary key, account uuid not null references accounts on delete cascade,
  name text not null, os text not null, token_hash bytea unique not null,
  created_at timestamptz not null default now(), last_seen timestamptz, version text not null default '');
alter table machines add column if not exists version text not null default '';
-- Set once an app that asks for the password signs in with the login: the login alone then runs
-- no call only the account's owner makes (`surface::owner_only`).
alter table logins add column if not exists locked boolean not null default false;
-- Null until the account first sets or imports its tastes.
alter table accounts add column if not exists tastes jsonb;
create table if not exists pair_codes (code text primary key, account uuid not null references accounts on delete cascade,
  expires_at timestamptz not null);
-- A code one account made that lets one person create their own account, on a hub in invite mode.
-- It lives until it is used or revoked. Rows made before 0.17.22 carried a 7-day date, which
-- nothing reads anymore and this clears.
create table if not exists invites (id uuid primary key, code_hash bytea unique not null,
  account uuid not null references accounts on delete cascade, hint text not null,
  created_at timestamptz not null default now(), expires_at timestamptz,
  used_by uuid references accounts on delete set null, used_at timestamptz);
alter table invites alter column expires_at drop not null;
update invites set expires_at = null where used_at is null and expires_at is not null;
create table if not exists project_files (account uuid not null references accounts on delete cascade, project text not null,
  path text not null, sha text not null, modified bigint not null, data bytea not null, executable boolean not null default false,
  primary key (account, project, path));
alter table project_files add column if not exists executable boolean not null default false;
create table if not exists shared_projects (account uuid not null references accounts on delete cascade, key text not null,
  name text not null, repo text not null, updated timestamptz not null default now(), primary key (account, key));
-- Without a cascade: an account whose boxes still run can't be deleted, which would orphan them.
create table if not exists boxes (id text primary key, account uuid not null references accounts,
  machine uuid not null, upstash text, connector text not null, subject jsonb not null, spec jsonb not null,
  size text not null, phase text not null, reason text, created_at timestamptz not null default now(),
  unique (account, connector, subject));
-- External MCP agents. A client registers itself; the user lets it in through a request, which
-- makes a grant; the grant's codes and tokens are kept as their sha256, as logins are.
create table if not exists agent_clients (id text primary key, name text not null, redirect_uris jsonb not null,
  created_at timestamptz not null default now());
create table if not exists agent_requests (id_hash bytea primary key, client text not null references agent_clients on delete cascade,
  redirect_uri text not null, state text, code_challenge text not null, scopes jsonb not null, resource text not null,
  expires_at timestamptz not null);
create table if not exists agent_grants (id uuid primary key, account uuid not null references accounts on delete cascade,
  client text not null references agent_clients on delete cascade, scopes jsonb not null, projects jsonb not null,
  created_at timestamptz not null default now(), last_used timestamptz, revoked boolean not null default false);
create table if not exists agent_codes (code_hash bytea primary key, grant_id uuid not null references agent_grants on delete cascade,
  redirect_uri text not null, code_challenge text not null, resource text not null, expires_at timestamptz not null,
  used boolean not null default false);
create table if not exists agent_tokens (token_hash bytea primary key, grant_id uuid not null references agent_grants on delete cascade,
  kind text not null, expires_at timestamptz not null, rotated boolean not null default false);
create table if not exists agent_sessions (id uuid primary key, grant_id uuid not null references agent_grants on delete cascade,
  machine uuid not null, project text not null, title text not null, created_at timestamptz not null default now());
create table if not exists agent_calls (grant_id uuid not null references agent_grants on delete cascade, key text not null,
  tool text not null, args_hash bytea not null, state text not null, session uuid, result jsonb,
  created_at timestamptz not null default now(), primary key (grant_id, key));
-- One finished delegated task per row, as the machine that ran it last sent it. `ended` is unix
-- milliseconds. A removed machine's rows stay: they are the account's history.
create table if not exists delegation_tasks (account uuid not null references accounts on delete cascade,
  machine uuid not null, id text not null, ended bigint not null, record jsonb not null,
  primary key (account, machine, id));
create index if not exists delegation_tasks_ended on delegation_tasks (account, ended);
-- One conversation's marks per row, as the `marks::SessionMarks` it holds. A session with no mark has no row.
create table if not exists session_marks (account uuid not null references accounts on delete cascade,
  session text not null, marks jsonb not null, updated timestamptz not null default now(),
  primary key (account, session));
-- What agents did as the user: one row per act, refusal, or decision on a confirmation.
create table if not exists agent_actions (id uuid primary key, account uuid not null references accounts on delete cascade,
  machine uuid not null, tab text not null, role text not null, action text not null, args text not null,
  outcome text not null, detail text not null default '', request_id text, at timestamptz not null default now());
alter table agent_actions add column if not exists task text;
create index if not exists agent_actions_at on agent_actions (account, at desc);
create index if not exists agent_actions_request on agent_actions (account, machine, tab, action, request_id) where request_id is not null;
-- A confirm act that waits on the user's approval in the app, then what they decided and it came to.
-- `notified` turns true once the session that asked has the notice.
create table if not exists agent_confirmations (id uuid primary key, account uuid not null references accounts on delete cascade,
  machine uuid not null, tab text not null, session text, role text not null, title text not null default '',
  action text not null, args jsonb not null, request_id text, created_at timestamptz not null default now(),
  decided_at timestamptz, approved boolean, outcome text, notified boolean not null default false,
  target text, result jsonb, unique (account, machine, tab, action, request_id));
-- A scheduled task per row, as the `schedule::Task` it is (without its hub-computed last run), and
-- the columns the scheduler reads. `next_at` is unix seconds, computed on every save.
create table if not exists tasks (id uuid primary key, account uuid not null references accounts on delete cascade,
  task jsonb not null, enabled boolean not null, next_at bigint, by_agent boolean not null default false,
  created_at timestamptz not null default now());
create index if not exists tasks_account on tasks (account);
create index if not exists tasks_due on tasks (next_at) where enabled and next_at is not null;
-- One occurrence of a task per row, claimed once: `unique (task, due_at)`. `run` is the `schedule::TaskRun`.
create table if not exists task_runs (task uuid not null references tasks on delete cascade,
  account uuid not null references accounts on delete cascade, due_at bigint not null, state text not null,
  run jsonb not null, unique (task, due_at));
create index if not exists task_runs_account on task_runs (account, task, due_at desc);
-- Whether tag and main protection exist for the account's repos: set only by the user's app, off until then.
alter table accounts add column if not exists release_protection boolean not null default false;
-- A box an agent opened through act, until the box ends and is recorded as that session's task.
-- `started` is unix milliseconds.
create table if not exists agent_boxes (box text primary key, account uuid not null references accounts on delete cascade,
  machine uuid not null, tab text not null, role text not null, started bigint not null);
";
const LEARNING_SCHEMA: &str = "
create table if not exists dictation_feedback (id uuid primary key, account uuid not null,
  created_at timestamptz not null default now(), rating text not null, raw text not null, cleaned text not null,
  final_text text not null, locale text not null, engine text not null, app_version text not null,
  platform text not null, duration_ms bigint not null);
";
/// A box row of the account, as the Usage page joins it to what Upstash lists.
pub struct OpenedBox {
    pub upstash: String,
    pub folder: Option<String>,
    pub repo: Option<String>,
    pub role: Option<String>,
    pub machine: Option<String>,
}
const BOX_COLUMNS: &str = "id, account, machine, upstash, connector, subject, spec, size, phase, reason, extract(epoch from now() - created_at)::float8";
/// Compressed bytes of synced project files one account may keep on the hub.
const SYNC_QUOTA: i64 = 1 << 30;

const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const CODE_LEN: usize = 8;
const INVITE_GROUPS: usize = 4;
/// A login unused this long stops working; every use pushes it back.
const LOGIN_IDLE_DAYS: i32 = 90;

#[derive(Clone)]
pub struct Db {
    sealer: Sealer,
    connection: Connection,
    /// Opened on first use. The shared connection pipelines every caller's statements, so a
    /// transaction there would take in other callers' statements too.
    transactions: Arc<tokio::sync::Mutex<Option<Client>>>,
}

/// The learning database. It holds no accounts, so its rows name the account only by id.
#[derive(Clone)]
pub struct Learning(Connection);

#[derive(Clone)]
struct Connection {
    url: Arc<str>,
    /// Replaced when Postgres drops the connection, as on its restart, so the hub doesn't stay down.
    client: Arc<tokio::sync::Mutex<Arc<Client>>>,
}

impl Connection {
    async fn open(url: &str, schema: &str) -> Result<Connection, String> {
        let client = open(url).await?;
        client.batch_execute(schema).await.map_err(|e| format!("could not create the tables: {e}"))?;
        Ok(Connection { url: url.into(), client: Arc::new(tokio::sync::Mutex::new(Arc::new(client))) })
    }

    async fn client(&self) -> Result<Arc<Client>, String> {
        let mut client = self.client.lock().await;
        if client.is_closed() {
            *client = Arc::new(open(&self.url).await?);
            eprintln!("pinned-server: reconnected to Postgres");
        }
        Ok(client.clone())
    }
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

/// A fresh invite code, such as `K7QM-2XRP-9HTB-WNCE`.
pub fn invite_code() -> String {
    (0..INVITE_GROUPS).map(|_| pair_code()[..4].to_owned()).collect::<Vec<_>>().join("-")
}

/// The hash an invite code is kept as. Case, spaces and dashes don't matter.
pub fn invite_hash(code: &str) -> Vec<u8> {
    hash(&code.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).collect::<String>())
}

/// What spending an invite code on a new account came to.
pub enum Invited {
    Created(AccountView),
    /// The code is unknown, used, revoked or expired. The account was not made.
    NoInvite,
    /// The code is good but the email is taken. The code stays unused.
    EmailTaken,
}

/// An audit row as an act writes it.
pub struct Audit<'a> {
    pub account: Uuid,
    pub machine: Uuid,
    pub tab: &'a str,
    pub role: ActorRole,
    pub action: &'a str,
    pub args: &'a str,
    pub outcome: AuditOutcome,
    pub detail: &'a str,
    pub request_id: Option<&'a str>,
    /// The scheduled task the act is about.
    pub task: Option<&'a str>,
}

/// A login token's account, and whether an app locked it (`Db::login`).
pub struct Login {
    pub account: Uuid,
    pub locked: bool,
}

/// A confirmation as the hub keeps it: what the user sees, and the Claude session that asked,
/// which finds its tab after a restart.
pub struct Held {
    pub confirmation: Confirmation,
    pub session: Option<String>,
    /// What running it answered, as the agent may see it.
    pub result: Value,
}

const CONFIRMATION_COLUMNS: &str = "c.id, extract(epoch from c.created_at)::bigint, c.machine, coalesce(m.name, ''), c.tab, c.title, c.role, c.action, c.args,
  c.request_id, c.approved, extract(epoch from c.decided_at)::bigint, c.outcome, c.session, c.target, c.result";

fn held(r: &tokio_postgres::Row) -> Held {
    let role: String = r.get(6);
    let action: String = r.get(7);
    let decided_at: Option<i64> = r.get(11);
    let confirmation = Confirmation {
        id: r.get::<_, Uuid>(0).to_string(),
        at: r.get(1),
        machine: r.get::<_, Uuid>(2).to_string(),
        machine_name: r.get(3),
        tab: r.get(4),
        title: r.get(5),
        role: serde_json::from_value(Value::String(role)).unwrap_or(ActorRole::Session),
        about: pinned_core::surface::find(&action).map_or_else(String::new, |a| a.about.to_owned()),
        action,
        args: r.get(8),
        target: r.get(14),
        request_id: r.get(9),
        decision: decided_at.and(r.get::<_, Option<bool>>(10)).map(|approved| if approved { Decision::Approved } else { Decision::Denied }),
        decided_at,
        outcome: r.get(12),
    };
    Held { confirmation, session: r.get(13), result: r.get::<_, Option<Value>>(15).unwrap_or(Value::Null) }
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

/// The row, or what deleting its box takes when its run is in a shape this hub can't read.
fn box_row(r: &tokio_postgres::Row) -> Result<BoxRow, Remains> {
    let (id, machine, upstash): (String, Uuid, _) = (r.get(0), r.get(2), r.get::<_, Option<String>>(3).map(BoxId));
    let unreadable = |e: serde_json::Error| Remains { id: id.clone(), machine, upstash: upstash.clone(), why: format!("its row is unreadable: {e}") };
    Ok(BoxRow {
        account: r.get(1),
        machine,
        connector: r.get(4),
        subject: serde_json::from_value(r.get(5)).map_err(unreadable)?,
        spec: serde_json::from_value(r.get(6)).map_err(unreadable)?,
        work: match r.get::<_, Value>(6).get("work") {
            Some(work) => serde_json::from_value(work.clone()).map_err(unreadable)?,
            None => BoxWork::Run,
        },
        size: serde_json::from_value(Value::String(r.get(7))).map_err(unreadable)?,
        phase: Phase::load(r.get(8), r.get(9)),
        age: std::time::Duration::from_secs_f64(r.get::<_, f64>(10).max(0.0)),
        upstash,
        id,
    })
}

fn readable(row: Result<BoxRow, Remains>) -> Result<BoxRow, String> {
    row.map_err(|r| format!("box {} {}; the next check deletes it", r.id, r.why))
}

fn stored_task(account: Uuid, value: Value) -> Result<Task, String> {
    serde_json::from_value(value).map_err(|e| format!("account {account} has an unreadable task: {e}"))
}

fn stored_marks(account: Uuid, value: Value) -> Result<SessionMarks, String> {
    serde_json::from_value(value).map_err(|e| format!("account {account} has unreadable session marks: {e}"))
}

impl Learning {
    pub async fn connect(url: &str) -> Result<Learning, String> {
        Connection::open(url, LEARNING_SCHEMA).await.map(Learning).map_err(|e| format!("learning database: {e}"))
    }

    pub async fn save_dictation(&self, account: Uuid, feedback: &DictationFeedback) -> Result<(), String> {
        let rating = match feedback.rating {
            DictationRating::Good => "good",
            DictationRating::Bad => "bad",
        };
        let engine = match feedback.engine {
            DictationEngine::Native => "native",
            DictationEngine::Web => "web",
        };
        let duration = i64::try_from(feedback.duration_ms).map_err(|e| e.to_string())?;
        self.0
            .client()
            .await?
            .execute(
                "insert into dictation_feedback (id, account, rating, raw, cleaned, final_text, locale, engine, app_version, platform, duration_ms)
                 values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                &[
                    &Uuid::new_v4(),
                    &account,
                    &rating,
                    &feedback.raw,
                    &feedback.cleaned,
                    &feedback.final_text,
                    &feedback.locale,
                    &engine,
                    &feedback.app_version,
                    &feedback.platform,
                    &duration,
                ],
            )
            .await
            .map_err(err)
            .map(drop)
    }
}

impl Db {
    pub async fn connect(url: &str, sealer: Sealer) -> Result<Db, String> {
        Ok(Db { sealer, connection: Connection::open(url, SCHEMA).await?, transactions: Arc::default() })
    }

    async fn client(&self) -> Result<Arc<Client>, String> {
        self.connection.client().await
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
            .query("select path, sha, modified, octet_length(data), executable from project_files where account = $1 and project = $2", &[&account, &project])
            .await
            .map_err(err)?;
        Ok(rows
            .iter()
            .map(|r| sync::Entry { path: r.get(0), sha: r.get(1), modified: r.get::<_, i64>(2) as u64, size: r.get::<_, i32>(3) as u64, executable: Some(r.get(4)) })
            .collect())
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
            // `known` is false when the sender omitted the mode, so a stored true survives. A null column would be omitted on the way out, and a unix machine would never send the bit.
            let known = entry.executable.is_some();
            let bit = entry.executable.unwrap_or(false);
            client
                .execute(
                    "insert into project_files (account, project, path, sha, modified, data, executable) values ($1, $2, $3, $4, $5, $6, $7) \
                     on conflict (account, project, path) do update set sha = excluded.sha, modified = excluded.modified, data = excluded.data, \
                     executable = case when $8 then excluded.executable else project_files.executable end",
                    &[&account, &project, &entry.path, &entry.sha, &(entry.modified as i64), data, &bit, &known],
                )
                .await
                .map_err(err)?;
        }
        Ok(())
    }

    /// Sets the executable bit only while the row still has `sha`. Leaves the bytes and modified time alone.
    pub async fn sync_mark_executable(&self, account: Uuid, project: &str, repairs: &[sync::ModeRepair]) -> Result<(), String> {
        let client = self.client().await?;
        for repair in repairs {
            // A content upsert would store the caller's bytes and mtime, and erase an edit that landed after list.
            client
                .execute(
                    "update project_files set executable = true where account = $1 and project = $2 and path = $3 and sha = $4",
                    &[&account, &project, &repair.path, &repair.sha],
                )
                .await
                .map_err(err)?;
        }
        Ok(())
    }

    pub async fn sync_pull(&self, account: Uuid, project: &str, paths: &[String]) -> Result<Vec<(sync::Entry, Vec<u8>)>, String> {
        let rows = self
            .client().await?
            .query("select path, sha, modified, data, executable from project_files where account = $1 and project = $2 and path = any($3)", &[&account, &project, &paths])
            .await
            .map_err(err)?;
        Ok(rows
            .iter()
            .map(|r| {
                let data: Vec<u8> = r.get(3);
                (sync::Entry { path: r.get(0), sha: r.get(1), modified: r.get::<_, i64>(2) as u64, size: data.len() as u64, executable: Some(r.get(4)) }, data)
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

    /// The login `token` opens, marking it used, and locked from now on when `lock`.
    pub async fn login(&self, token: &str, lock: bool) -> Result<Option<Login>, String> {
        let row = self
            .client().await?
            .query_opt(
                "update logins set last_used = now(), locked = locked or $3 where token_hash = $1 \
                 and coalesce(last_used, created_at) > now() - make_interval(days => $2) returning account, locked",
                &[&hash(token), &LOGIN_IDLE_DAYS, &lock],
            )
            .await
            .map_err(err)?;
        Ok(row.map(|r| Login { account: r.get(0), locked: r.get(1) }))
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
        let view = MachineView { id: id.to_string(), name: name.to_owned(), os: os.to_owned(), online: false, last_seen: row.get(0), upstash_box: None, version: String::new(), update: None, state: None, background: None };
        Ok((view, token.text))
    }

    pub async fn machine_connected(&self, token: &str, os: &str, version: &str) -> Result<Option<MachineRow>, String> {
        let row = self
            .client().await?
            .query_opt(
                "update machines set os = $2, last_seen = now(), version = $3 where token_hash = $1
                 returning id, account, name, exists (select 1 from boxes where boxes.machine = machines.id)",
                &[&hash(token), &os, &version],
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
                "select m.id, m.name, m.os, extract(epoch from coalesce(m.last_seen, m.created_at))::bigint, b.upstash, m.version from machines m
                 left join boxes b on b.machine = m.id where m.account = $1 order by m.created_at, m.id",
                &[&account],
            )
            .await
            .map_err(err)?;
        Ok(rows
            .iter()
            .map(|r| MachineView { id: r.get::<_, Uuid>(0).to_string(), name: r.get(1), os: r.get(2), online: false, last_seen: r.get(3), upstash_box: r.get(4), version: r.get(5), update: None, state: None, background: None })
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
        let (subject, mut spec) = (serde_json::to_value(&row.subject).unwrap(), serde_json::to_value(&row.spec).unwrap());
        if row.work != BoxWork::Run {
            spec["work"] = serde_json::to_value(&row.work).unwrap();
        }
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
        Ok((readable(box_row(&stored))?, inserted == 1))
    }

    pub async fn boxes(&self, account: Uuid) -> Result<Vec<Result<BoxRow, Remains>>, String> {
        let rows = self.client().await?.query(&format!("select {BOX_COLUMNS} from boxes where account = $1 order by created_at"), &[&account]).await.map_err(err)?;
        Ok(rows.iter().map(box_row).collect())
    }

    pub async fn box_of(&self, account: Uuid, machine: Uuid) -> Result<Option<BoxRow>, String> {
        let row = self.client().await?.query_opt(&format!("select {BOX_COLUMNS} from boxes where account = $1 and machine = $2"), &[&account, &machine]).await.map_err(err)?;
        row.as_ref().map(|r| readable(box_row(r))).transpose()
    }

    /// Adds to the row's budget, kept in its spec beside the run.
    pub async fn set_box_budget(&self, id: &str, budget: f64) -> Result<(), String> {
        self.client().await?.execute("update boxes set spec = spec || jsonb_build_object('budget_usd', $2::float8) where id = $1", &[&id, &budget]).await.map_err(err)?;
        Ok(())
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

    /// The account's row, or what is left of one this hub cannot read. `None` when the account has
    /// no row with this id. The id comes from the account's settings, which its client writes.
    pub async fn box_by_id(&self, account: Uuid, id: &str) -> Result<Option<Result<BoxRow, Remains>>, String> {
        let row = self.client().await?.query_opt(&format!("select {BOX_COLUMNS} from boxes where id = $1 and account = $2"), &[&id, &account]).await.map_err(err)?;
        Ok(row.as_ref().map(box_row))
    }

    /// The id of the account's row for the workflow's subject.
    pub async fn box_for(&self, account: Uuid, connector: &str, subject: &Subject) -> Result<Option<String>, String> {
        let subject = serde_json::to_value(subject).unwrap();
        let row = self
            .client().await?
            .query_opt("select id from boxes where account = $1 and connector = $2 and subject = $3", &[&account, &connector, &subject])
            .await
            .map_err(err)?;
        Ok(row.map(|r| r.get(0)))
    }

    /// The row `subject` has for `connector`, whatever phase it is in.
    pub async fn box_row_for(&self, account: Uuid, connector: &str, subject: &Subject) -> Result<Option<BoxRow>, String> {
        let subject = serde_json::to_value(subject).unwrap();
        let row = self
            .client().await?
            .query_opt(&format!("select {BOX_COLUMNS} from boxes where account = $1 and connector = $2 and subject = $3"), &[&account, &connector, &subject])
            .await
            .map_err(err)?;
        row.as_ref().map(|r| readable(box_row(r))).transpose()
    }

    pub async fn box_exists(&self, id: &str) -> Result<bool, String> {
        let row = self.client().await?.query_one("select exists (select 1 from boxes where id = $1)", &[&id]).await.map_err(err)?;
        Ok(row.get(0))
    }

    /// Keeps that the agent session `opener` opened box row `id` at unix ms `started`.
    pub async fn hold_agent_box(&self, id: &str, account: Uuid, opener: &Opener, started: u64) -> Result<(), String> {
        let started = i64::try_from(started).map_err(|_| format!("box {id} started at an impossible time"))?;
        self.client()
            .await?
            .execute(
                "insert into agent_boxes (box, account, machine, tab, role, started) values ($1, $2, $3, $4, $5, $6) on conflict (box) do nothing",
                &[&id, &account, &opener.machine, &opener.tab, &opener.role.name(), &started],
            )
            .await
            .map_err(err)
            .map(drop)
    }

    /// The agent session that opened the account's box row `id`, and when, while it is not recorded yet.
    pub async fn agent_box(&self, account: Uuid, id: &str) -> Result<Option<(Opener, u64)>, String> {
        let row = self.client().await?.query_opt("select machine, tab, role, started from agent_boxes where box = $1 and account = $2", &[&id, &account]).await.map_err(err)?;
        Ok(row.map(|r| {
            let role = serde_json::from_value(Value::String(r.get(2))).unwrap_or(ActorRole::Session);
            (Opener { machine: r.get(0), tab: r.get(1), role }, u64::try_from(r.get::<_, i64>(3)).unwrap_or(0))
        }))
    }

    pub async fn forget_agent_box(&self, account: Uuid, id: &str) -> Result<(), String> {
        self.client().await?.execute("delete from agent_boxes where box = $1 and account = $2", &[&id, &account]).await.map_err(err).map(drop)
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

    pub async fn create_invite(&self, account: Uuid) -> Result<NewInvite, String> {
        let (id, code) = (Uuid::new_v4(), invite_code());
        self.client()
            .await?
            .execute(
                "insert into invites (id, code_hash, account, hint) values ($1, $2, $3, $4)",
                &[&id, &invite_hash(&code), &account, &code[..4].to_owned()],
            )
            .await
            .map_err(err)?;
        Ok(NewInvite { id: id.to_string(), code, expires_at: NEVER_EXPIRES })
    }

    /// Writes one row of the account's audit trail.
    pub async fn audit(&self, row: &Audit<'_>) -> Result<(), String> {
        self.client()
            .await?
            .execute(
                "insert into agent_actions (id, account, machine, tab, role, action, args, outcome, detail, request_id, task) values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                &[&Uuid::new_v4(), &row.account, &row.machine, &row.tab, &row.role.name(), &row.action, &row.args, &row.outcome.name(), &row.detail, &row.request_id, &row.task],
            )
            .await
            .map_err(err)
            .map(drop)
    }

    /// The account's audit rows, newest first: `limit` of them from unix second `before` and earlier.
    /// A page repeats the rows of its boundary second, which the client drops by id.
    pub async fn audit_rows(&self, account: Uuid, before: Option<i64>, limit: i64) -> Result<Vec<AuditEntry>, String> {
        let rows = self
            .client()
            .await?
            .query(
                "select a.id, extract(epoch from a.at)::bigint, a.machine, coalesce(m.name, ''), a.tab, a.role, a.action, a.args, a.outcome, a.detail, a.request_id, a.task
                 from agent_actions a left join machines m on m.id = a.machine and m.account = a.account
                 where a.account = $1 and ($2::bigint is null or a.at < to_timestamp($2::bigint + 1)) order by a.at desc limit $3",
                &[&account, &before, &limit],
            )
            .await
            .map_err(err)?;
        Ok(rows
            .iter()
            .map(|r| AuditEntry {
                id: r.get::<_, Uuid>(0).to_string(),
                at: r.get(1),
                machine: r.get::<_, Uuid>(2).to_string(),
                machine_name: r.get(3),
                tab: r.get(4),
                role: serde_json::from_value(Value::String(r.get(5))).unwrap_or(ActorRole::Session),
                action: r.get(6),
                args: r.get(7),
                outcome: AuditOutcome::parse(r.get(8)).unwrap_or(AuditOutcome::Failed),
                detail: r.get(9),
                request_id: r.get(10),
                task: r.get(11),
            })
            .collect())
    }

    // ---- Scheduled tasks ----

    /// The account's tasks, soonest first (paused and finished ones last), each with its last run.
    pub async fn tasks(&self, account: Uuid) -> Result<Vec<Task>, String> {
        let client = self.client().await?;
        let rows = client.query("select task, next_at from tasks where account = $1 order by next_at nulls last, created_at", &[&account]).await.map_err(err)?;
        let runs = client
            .query("select distinct on (task) run from task_runs where account = $1 order by task, due_at desc", &[&account])
            .await
            .map_err(err)?;
        let mut last: BTreeMap<String, TaskRun> = BTreeMap::new();
        for row in runs {
            let run: TaskRun = serde_json::from_value(row.get(0)).map_err(|e| format!("account {account} has an unreadable task run: {e}"))?;
            last.insert(run.task.clone(), run);
        }
        rows.iter()
            .map(|r| {
                let mut task = stored_task(account, r.get(0))?;
                task.next_at = r.get(1);
                task.last_run = last.remove(&task.id);
                Ok(task)
            })
            .collect()
    }

    pub async fn task(&self, account: Uuid, id: Uuid) -> Result<Option<Task>, String> {
        let row = self.client().await?.query_opt("select task, next_at from tasks where account = $1 and id = $2", &[&account, &id]).await.map_err(err)?;
        row.map(|r| stored_task(account, r.get(0)).map(|task| Task { next_at: r.get(1), ..task })).transpose()
    }

    /// Stores `task` (id and `next_at` already set) as the account's. False when the id is another account's.
    pub async fn save_task(&self, account: Uuid, task: &Task) -> Result<bool, String> {
        let id = Uuid::parse_str(&task.id).map_err(|_| format!("{} is not a task id", task.id))?;
        let by_agent = matches!(task.created_by, Actor::Agent { .. });
        let stored = Task { next_at: None, last_run: None, ..task.clone() };
        let value = serde_json::to_value(&stored).map_err(|e| e.to_string())?;
        let changed = self
            .client()
            .await?
            .execute(
                "insert into tasks (id, account, task, enabled, next_at, by_agent) values ($1, $2, $3, $4, $5, $6)
                 on conflict (id) do update set task = excluded.task, enabled = excluded.enabled, next_at = excluded.next_at where tasks.account = excluded.account",
                &[&id, &account, &value, &task.enabled, &task.next_at, &by_agent],
            )
            .await
            .map_err(err)?;
        Ok(changed > 0)
    }

    pub async fn delete_task(&self, account: Uuid, id: Uuid) -> Result<bool, String> {
        self.client().await?.execute("delete from tasks where account = $1 and id = $2", &[&account, &id]).await.map(|n| n > 0).map_err(err)
    }

    /// How many tasks agents created that the account still holds.
    pub async fn agent_task_count(&self, account: Uuid) -> Result<usize, String> {
        let row = self.client().await?.query_one("select count(*) from tasks where account = $1 and by_agent", &[&account]).await.map_err(err)?;
        Ok(row.get::<_, i64>(0) as usize)
    }

    /// The task's runs, newest first.
    pub async fn task_runs(&self, account: Uuid, id: Uuid, limit: i64) -> Result<Vec<TaskRun>, String> {
        let rows = self.client().await?.query("select run from task_runs where account = $1 and task = $2 order by due_at desc limit $3", &[&account, &id, &limit]).await.map_err(err)?;
        rows.iter().map(|r| serde_json::from_value(r.get(0)).map_err(|e| format!("account {account} has an unreadable task run: {e}"))).collect()
    }

    pub async fn release_protection(&self, account: Uuid) -> Result<bool, String> {
        let row = self.client().await?.query_opt("select release_protection from accounts where id = $1", &[&account]).await.map_err(err)?;
        Ok(row.is_some_and(|r| r.get(0)))
    }

    pub async fn set_release_protection(&self, account: Uuid, on: bool) -> Result<(), String> {
        self.client().await?.execute("update accounts set release_protection = $2 where id = $1", &[&account, &on]).await.map(drop).map_err(err)
    }

    /// What the first `action` with `request_id` from the session in `tab` on `machine` came to, and when.
    pub async fn audited(&self, account: Uuid, machine: Uuid, tab: &str, action: &str, request_id: &str) -> Result<Option<(AuditOutcome, i64)>, String> {
        let row = self
            .client()
            .await?
            .query_opt(
                "select outcome, extract(epoch from at)::bigint from agent_actions where account = $1 and machine = $2 and tab = $3 and action = $4 and request_id = $5 order by at limit 1",
                &[&account, &machine, &tab, &action, &request_id],
            )
            .await
            .map_err(err)?;
        Ok(row.map(|r| (AuditOutcome::parse(r.get(0)).unwrap_or(AuditOutcome::Failed), r.get(1))))
    }

    /// Holds a confirm act for the user and answers its id, or, with false, the id of the one the same
    /// session already asked for with the same action and `request_id`.
    pub async fn hold_confirmation(&self, account: Uuid, machine: Uuid, session: Option<&str>, asked: &Confirmation) -> Result<(Uuid, bool), String> {
        let client = self.client().await?;
        let id = Uuid::new_v4();
        let request_id = asked.request_id.as_deref();
        let inserted = client
            .query_opt(
                "insert into agent_confirmations (id, account, machine, tab, session, role, title, action, args, request_id, target) values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                 on conflict (account, machine, tab, action, request_id) do nothing returning id",
                &[&id, &account, &machine, &asked.tab, &session, &asked.role.name(), &asked.title, &asked.action, &asked.args, &request_id, &asked.target],
            )
            .await
            .map_err(err)?;
        if inserted.is_some() {
            return Ok((id, true));
        }
        let row = client
            .query_one(
                "select id from agent_confirmations where account = $1 and machine = $2 and tab = $3 and action = $4 and request_id = $5",
                &[&account, &machine, &asked.tab, &asked.action, &request_id],
            )
            .await
            .map_err(err)?;
        Ok((row.get(0), false))
    }

    /// The account's confirmation `id`.
    pub async fn confirmation(&self, account: Uuid, id: Uuid) -> Result<Option<Held>, String> {
        let query = format!("select {CONFIRMATION_COLUMNS} from agent_confirmations c left join machines m on m.id = c.machine and m.account = c.account where c.account = $1 and c.id = $2");
        Ok(self.client().await?.query_opt(&query, &[&account, &id]).await.map_err(err)?.as_ref().map(held))
    }

    /// The account's confirmations that wait, oldest first, then its last 20 decided, newest first.
    pub async fn confirmations(&self, account: Uuid) -> Result<Vec<Confirmation>, String> {
        let query = format!(
            "(select {CONFIRMATION_COLUMNS} from agent_confirmations c left join machines m on m.id = c.machine and m.account = c.account
               where c.account = $1 and c.decided_at is null order by c.created_at)
             union all
             (select {CONFIRMATION_COLUMNS} from agent_confirmations c left join machines m on m.id = c.machine and m.account = c.account
               where c.account = $1 and c.decided_at is not null order by c.decided_at desc limit 20)"
        );
        Ok(self.client().await?.query(&query, &[&account]).await.map_err(err)?.iter().map(|r| held(r).confirmation).collect())
    }

    /// Decides the account's waiting confirmation `id`, once: None when it is unknown, another
    /// account's, or decided already.
    pub async fn decide_confirmation(&self, account: Uuid, id: Uuid, approved: bool) -> Result<Option<Held>, String> {
        let client = self.client().await?;
        let taken = client
            .execute("update agent_confirmations set decided_at = now(), approved = $3 where account = $1 and id = $2 and decided_at is null", &[&account, &id, &approved])
            .await
            .map_err(err)?;
        if taken == 0 {
            return Ok(None);
        }
        self.confirmation(account, id).await
    }

    /// Gives every confirmation decided more than `running` ago and still without an outcome one: a
    /// hub that stopped between the decision and its outcome left it. A denial ran nothing; an
    /// approval may have run, so it reads as failed with `approved_outcome`. Answers each with its account.
    pub async fn finish_interrupted(&self, approved_outcome: &str, running: Duration) -> Result<Vec<(Uuid, Held)>, String> {
        let rows = self
            .client().await?
            .query(
                "update agent_confirmations set outcome = case when approved then $1 else 'denied' end, result = 'null'::jsonb \
                 where decided_at < now() - make_interval(secs => $2) and outcome is null returning account, id",
                &[&approved_outcome, &running.as_secs_f64()],
            )
            .await
            .map_err(err)?;
        let mut finished = vec![];
        for row in &rows {
            let account: Uuid = row.get(0);
            finished.extend(self.confirmation(account, row.get(1)).await?.map(|held| (account, held)));
        }
        Ok(finished)
    }

    /// Puts a decided confirmation back to waiting, as a credential action whose run failed, so the
    /// user can type another key.
    pub async fn reopen_confirmation(&self, account: Uuid, id: Uuid) -> Result<(), String> {
        self.client()
            .await?
            .execute("update agent_confirmations set decided_at = null, approved = null, outcome = null, result = null where id = $1 and account = $2", &[&id, &account])
            .await
            .map_err(err)
            .map(drop)
    }

    pub async fn finish_confirmation(&self, id: Uuid, outcome: &str, result: &Value) -> Result<(), String> {
        self.client().await?.execute("update agent_confirmations set outcome = $2, result = $3 where id = $1", &[&id, &outcome, result]).await.map_err(err).map(drop)
    }

    /// The account's confirmations decided in the last week whose session on `machine` has no notice yet.
    pub async fn unnotified(&self, account: Uuid, machine: Uuid) -> Result<Vec<Held>, String> {
        let query = format!(
            "select {CONFIRMATION_COLUMNS} from agent_confirmations c left join machines m on m.id = c.machine and m.account = c.account
             where c.account = $1 and c.machine = $2 and c.decided_at > now() - interval '7 days' and c.outcome is not null and not c.notified order by c.decided_at"
        );
        Ok(self.client().await?.query(&query, &[&account, &machine]).await.map_err(err)?.iter().map(held).collect())
    }

    pub async fn notified(&self, id: Uuid) -> Result<(), String> {
        self.client().await?.execute("update agent_confirmations set notified = true where id = $1", &[&id]).await.map_err(err).map(drop)
    }

    /// The account's codes nobody has used yet, newest first.
    pub async fn invites(&self, account: Uuid) -> Result<Vec<InviteView>, String> {
        let rows = self
            .client().await?
            .query(
                "select id, hint, extract(epoch from created_at)::bigint from invites
                 where account = $1 and used_at is null order by created_at desc",
                &[&account],
            )
            .await
            .map_err(err)?;
        Ok(rows.iter().map(|r| InviteView { id: r.get::<_, Uuid>(0).to_string(), hint: r.get(1), created_at: r.get(2), expires_at: NEVER_EXPIRES }).collect())
    }

    /// False when the account has no unused code `id`.
    pub async fn revoke_invite(&self, account: Uuid, id: Uuid) -> Result<bool, String> {
        let deleted = self.client().await?.execute("delete from invites where id = $1 and account = $2 and used_at is null", &[&id, &account]).await.map_err(err)?;
        Ok(deleted == 1)
    }

    /// Makes the account and spends the code in one statement, so two sign ups can't share a code
    /// and a taken email leaves the code unused.
    pub async fn create_invited_account(&self, code_hash: &[u8], email: &str, password_hash: &str) -> Result<Invited, String> {
        let id = Uuid::new_v4();
        let row = self
            .client().await?
            .query_one(
                "with invite as (select id from invites where code_hash = $1 and used_at is null for update),
                 made as (insert into accounts (id, email, password_hash) select $2, $3, $4 from invite on conflict (email) do nothing returning id),
                 spent as (update invites set used_by = made.id, used_at = now() from made, invite where invites.id = invite.id returning invites.id)
                 select exists (select 1 from invite), exists (select 1 from made), exists (select 1 from spent)",
                &[&code_hash, &id, &email, &password_hash],
            )
            .await
            .map_err(err)?;
        Ok(match (row.get(0), row.get(1)) {
            (false, _) => Invited::NoInvite,
            (true, false) => Invited::EmailTaken,
            (true, true) => Invited::Created(AccountView { id: id.to_string(), email: email.to_owned() }),
        })
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

    /// Stores `record` as the machine's latest word on that task.
    pub async fn save_task_record(&self, account: Uuid, machine: Uuid, record: &TaskRecord) -> Result<(), String> {
        let ended = i64::try_from(record.ended).map_err(|_| format!("task {} ended at an impossible time", record.id))?;
        let value = serde_json::to_value(record).map_err(|e| e.to_string())?;
        self.client()
            .await?
            .execute(
                "insert into delegation_tasks (account, machine, id, ended, record) values ($1, $2, $3, $4, $5)
                 on conflict (account, machine, id) do update set ended = excluded.ended, record = excluded.record",
                &[&account, &machine, &record.id, &ended, &value],
            )
            .await
            .map_err(err)
            .map(drop)
    }

    pub async fn task_record(&self, account: Uuid, machine: Uuid, id: &str) -> Result<Option<TaskRecord>, String> {
        let row = self.client().await?.query_opt("select record from delegation_tasks where account = $1 and machine = $2 and id = $3", &[&account, &machine, &id]).await.map_err(err)?;
        row.map(|r| serde_json::from_value(r.get(0)).map_err(|e| format!("task {id} has an unreadable record: {e}"))).transpose()
    }

    /// The account's task records that ended at `since` (unix ms) or later, and how many stored
    /// rows did not read as one. Those are left out of the records and logged once.
    pub async fn task_records(&self, account: Uuid, since: u64) -> Result<(Vec<TaskRecord>, u32), String> {
        let since = i64::try_from(since).unwrap_or(i64::MAX);
        let rows = self.client().await?.query("select id, record from delegation_tasks where account = $1 and ended >= $2", &[&account, &since]).await.map_err(err)?;
        let (mut records, mut unreadable) = (Vec::with_capacity(rows.len()), vec![]);
        for row in &rows {
            match serde_json::from_value::<TaskRecord>(row.get(1)) {
                Ok(record) => records.push(record),
                Err(e) => unreadable.push((row.get::<_, String>(0), e.to_string())),
            }
        }
        if let Some((id, why)) = unreadable.first() {
            eprintln!("pinned-server: {} of account {account}'s stored task records did not read, so delegation_stats leaves them out (first: {id}: {why})", unreadable.len());
        }
        Ok((records, u32::try_from(unreadable.len()).unwrap_or(u32::MAX)))
    }

    /// The box time the account's delegated tasks that ended at `since` (unix ms) or later were
    /// billed, in milliseconds, and how many of them had any.
    pub async fn box_ms_since(&self, account: Uuid, since: u64) -> Result<(u64, u32), String> {
        let since = i64::try_from(since).unwrap_or(i64::MAX);
        let row = self
            .client()
            .await?
            .query_one(
                "select coalesce(sum((record->'usage'->>'box_ms')::bigint), 0)::bigint, count(*) filter (where record->'usage'->>'box_ms' is not null)::bigint from delegation_tasks where account = $1 and ended >= $2",
                &[&account, &since],
            )
            .await
            .map_err(err)?;
        Ok((u64::try_from(row.get::<_, i64>(0)).unwrap_or(0), u32::try_from(row.get::<_, i64>(1)).unwrap_or(u32::MAX)))
    }

    /// What the account's box rows with an Upstash id say of who opened the box, by that id: the
    /// project's folder and repo, and the agent session that opened it, when one did. Never the
    /// run spec's token.
    pub async fn opened_boxes(&self, account: Uuid) -> Result<Vec<OpenedBox>, String> {
        let rows = self
            .client()
            .await?
            .query(
                "select b.upstash, b.spec->>'folder', b.spec->>'repo', a.role, coalesce(m.name, a.machine::text) \
                 from boxes b left join agent_boxes a on a.box = b.id and a.account = b.account \
                 left join machines m on m.id = a.machine and m.account = b.account \
                 where b.account = $1 and b.upstash is not null",
                &[&account],
            )
            .await
            .map_err(err)?;
        Ok(rows.iter().map(|r| OpenedBox { upstash: r.get(0), folder: r.get(1), repo: r.get(2), role: r.get(3), machine: r.get(4) }).collect())
    }

    /// The box time already billed to the other tasks of `machine`, in milliseconds.
    pub async fn billed_box_ms(&self, account: Uuid, machine: Uuid, except: &str) -> Result<u64, String> {
        let row = self
            .client()
            .await?
            .query_one(
                "select coalesce(sum((record->'usage'->>'box_ms')::bigint), 0)::bigint from delegation_tasks where account = $1 and machine = $2 and id <> $3",
                &[&account, &machine, &except],
            )
            .await
            .map_err(err)?;
        Ok(u64::try_from(row.get::<_, i64>(0)).unwrap_or(0))
    }

    /// When the box `machine` runs in was made, in unix milliseconds, while it is one of the account's boxes.
    pub async fn box_created_ms(&self, account: Uuid, machine: Uuid) -> Result<Option<u64>, String> {
        let row = self
            .client()
            .await?
            .query_opt("select (extract(epoch from created_at) * 1000)::bigint from boxes where account = $1 and machine = $2", &[&account, &machine])
            .await
            .map_err(err)?;
        Ok(row.and_then(|r| u64::try_from(r.get::<_, i64>(0)).ok()))
    }

    /// The account's tastes, read as what they were saved as on the current parallel defaults.
    pub async fn tastes(&self, account: Uuid) -> Result<Option<Tastes>, String> {
        let row = self.client().await?.query_one("select tastes from accounts where id = $1", &[&account]).await.map_err(err)?;
        let stored: Option<Value> = row.get(0);
        match stored {
            None => Ok(None),
            Some(tastes) => {
                let mut tastes: Tastes = serde_json::from_value(tastes).map_err(|e| format!("account {account} has unreadable tastes: {e}"))?;
                tastes.take_raised_defaults();
                Ok(Some(tastes))
            }
        }
    }

    pub async fn save_tastes(&self, account: Uuid, tastes: &Tastes) -> Result<(), String> {
        let tastes = serde_json::to_value(tastes).map_err(|e| e.to_string())?;
        self.client().await?.execute("update accounts set tastes = $2 where id = $1", &[&account, &tastes]).await.map_err(err).map(drop)
    }

    /// Stores `tastes` only when the account has none, and answers whether it did.
    pub async fn import_tastes(&self, account: Uuid, tastes: &Tastes) -> Result<bool, String> {
        let tastes = serde_json::to_value(tastes).map_err(|e| e.to_string())?;
        let updated = self.client().await?.execute("update accounts set tastes = $2 where id = $1 and tastes is null", &[&account, &tastes]).await.map_err(err)?;
        Ok(updated == 1)
    }

    pub async fn session_marks(&self, account: Uuid) -> Result<Vec<SessionMarks>, String> {
        let rows = self.client().await?.query("select marks from session_marks where account = $1 order by session", &[&account]).await.map_err(err)?;
        rows.iter().map(|r| stored_marks(account, r.get(0))).collect()
    }

    /// The session's marks, empty when it has none.
    pub async fn session_marks_of(&self, account: Uuid, session: &str) -> Result<SessionMarks, String> {
        let row = self.client().await?.query_opt("select marks from session_marks where account = $1 and session = $2", &[&account, &session]).await.map_err(err)?;
        row.map_or_else(|| Ok(SessionMarks::new(session)), |r| stored_marks(account, r.get(0)))
    }

    /// Lands `changes` in order and answers the sessions they named as they are now. A session left
    /// with no mark loses its row.
    pub async fn mark_sessions(&self, account: Uuid, changes: Vec<MarkChange>) -> Result<Vec<SessionMarks>, String> {
        let named: Vec<String> = changes.iter().map(|change| change.session.clone()).collect();
        let mut held = self.transactions.lock().await;
        if held.as_ref().map_or(true, Client::is_closed) {
            *held = Some(open(&self.connection.url).await?);
        }
        let tx = held.as_mut().expect("opened above").transaction().await.map_err(err)?;
        let rows = tx
            .query("select session, marks from session_marks where account = $1 and session = any($2) for update", &[&account, &named])
            .await
            .map_err(err)?;
        let mut current: BTreeMap<String, SessionMarks> = BTreeMap::new();
        for row in rows {
            current.insert(row.get(0), stored_marks(account, row.get(1))?);
        }
        let touched = marks::apply(&mut current, changes);
        let (gone, kept): (Vec<&SessionMarks>, Vec<&SessionMarks>) = touched.iter().partition(|row| row.is_empty());
        let gone: Vec<&str> = gone.iter().map(|row| row.session.as_str()).collect();
        tx.execute("delete from session_marks where account = $1 and session = any($2)", &[&account, &gone]).await.map_err(err)?;
        let kept = serde_json::to_value(kept).map_err(|e| e.to_string())?;
        tx.execute(
            "insert into session_marks (account, session, marks) select $1::uuid, given->>'session', given from jsonb_array_elements($2::jsonb) given \
             on conflict (account, session) do update set marks = excluded.marks, updated = now()",
            &[&account, &kept],
        )
        .await
        .map_err(err)?;
        tx.commit().await.map_err(err)?;
        Ok(touched)
    }

    pub async fn save_secrets(&self, account: Uuid, secrets: &BTreeMap<String, String>) -> Result<(), String> {
        let sealed: BTreeMap<&String, String> = secrets.iter().map(|(name, value)| (name, self.sealer.seal(value))).collect();
        let sealed = serde_json::to_value(sealed).map_err(|e| e.to_string())?;
        self.client().await?.execute("update accounts set secrets = $2 where id = $1", &[&account, &sealed]).await.map_err(err).map(drop)
    }
}

/// How long a pending authorization waits for the user, a code for its exchange, and each token.
const AGENT_REQUEST_MINUTES: i32 = 10;
const AGENT_CODE_MINUTES: i32 = 5;
pub const AGENT_ACCESS_SECS: i64 = 3600;
const AGENT_REFRESH_DAYS: i32 = 30;
/// A call still pending this long was cut off, as by a hub restart, so its outcome is unknown.
const AGENT_CALL_STALE_SECS: i64 = 600;

/// An external agent's client registration.
pub struct AgentClient {
    pub id: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
}

pub struct AgentRequestRow {
    pub client: String,
    pub client_name: String,
    pub redirect_uri: String,
    pub state: Option<String>,
    pub scopes: Vec<String>,
    pub code_challenge: String,
    pub resource: String,
    pub expires_at: i64,
}

/// What an access token lets its agent do: the account it acts in, the scopes and the projects.
#[derive(Clone, Debug)]
pub struct AgentGrant {
    pub id: Uuid,
    pub account: Uuid,
    pub name: String,
    pub scopes: Vec<String>,
    pub projects: Vec<AgentProject>,
}

pub struct AgentCode {
    pub grant: Uuid,
    pub client: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub resource: String,
}

/// What spending a one-use secret found.
pub enum Spent<T> {
    Fresh(T),
    /// Spent before, or expired: the secret leaked or was replayed, so its grant should end.
    Replayed(Uuid),
    Unknown,
}

/// A side-effecting tool call, kept so a retry with the same key answers rather than acts twice.
pub enum Reserved {
    New,
    Existing { tool: String, args_hash: Vec<u8>, state: String, session: Option<Uuid>, result: Option<Value>, stale: bool },
}

pub struct AgentSession {
    pub id: Uuid,
    pub machine: Uuid,
    pub project: String,
    pub title: String,
    pub created_at: i64,
}

fn strings(value: Value) -> Vec<String> {
    serde_json::from_value(value).unwrap_or_default()
}

impl Db {
    pub async fn register_agent_client(&self, name: &str, redirect_uris: &[String]) -> Result<String, String> {
        let id = format!("pinned-agent-{}", Token::new().text);
        let uris = serde_json::to_value(redirect_uris).map_err(|e| e.to_string())?;
        self.client().await?.execute("insert into agent_clients (id, name, redirect_uris) values ($1, $2, $3)", &[&id, &name, &uris]).await.map_err(err)?;
        Ok(id)
    }

    pub async fn agent_client(&self, id: &str) -> Result<Option<AgentClient>, String> {
        let row = self.client().await?.query_opt("select name, redirect_uris from agent_clients where id = $1", &[&id]).await.map_err(err)?;
        Ok(row.map(|r| AgentClient { id: id.to_owned(), name: r.get(0), redirect_uris: strings(r.get(1)) }))
    }

    /// A pending authorization for the user to approve, by an id that only its URL carries.
    pub async fn create_agent_request(&self, client: &str, redirect_uri: &str, state: Option<&str>, code_challenge: &str, scopes: &[String], resource: &str) -> Result<String, String> {
        let client_db = self.client().await?;
        client_db.execute("delete from agent_requests where expires_at < now()", &[]).await.map_err(err)?;
        let id = Token::new();
        let scopes = serde_json::to_value(scopes).map_err(|e| e.to_string())?;
        client_db
            .execute(
                "insert into agent_requests (id_hash, client, redirect_uri, state, code_challenge, scopes, resource, expires_at) \
                 values ($1, $2, $3, $4, $5, $6, $7, now() + make_interval(mins => $8))",
                &[&id.hash, &client, &redirect_uri, &state, &code_challenge, &scopes, &resource, &AGENT_REQUEST_MINUTES],
            )
            .await
            .map_err(err)?;
        Ok(id.text)
    }

    fn agent_request_row(r: &tokio_postgres::Row) -> AgentRequestRow {
        AgentRequestRow {
            client: r.get(0),
            client_name: r.get(1),
            redirect_uri: r.get(2),
            state: r.get(3),
            code_challenge: r.get(4),
            scopes: strings(r.get(5)),
            resource: r.get(6),
            expires_at: r.get(7),
        }
    }

    pub async fn agent_request(&self, id: &str) -> Result<Option<AgentRequestRow>, String> {
        let row = self
            .client().await?
            .query_opt(
                "select r.client, c.name, r.redirect_uri, r.state, r.code_challenge, r.scopes, r.resource, extract(epoch from r.expires_at)::bigint \
                 from agent_requests r join agent_clients c on c.id = r.client where r.id_hash = $1 and r.expires_at > now()",
                &[&hash(id)],
            )
            .await
            .map_err(err)?;
        Ok(row.as_ref().map(Self::agent_request_row))
    }

    /// Spends the pending request, so it is approved or refused once.
    pub async fn take_agent_request(&self, id: &str) -> Result<Option<AgentRequestRow>, String> {
        let row = self
            .client().await?
            .query_opt(
                "with taken as (delete from agent_requests where id_hash = $1 returning *) \
                 select t.client, c.name, t.redirect_uri, t.state, t.code_challenge, t.scopes, t.resource, extract(epoch from t.expires_at)::bigint \
                 from taken t join agent_clients c on c.id = t.client where t.expires_at > now()",
                &[&hash(id)],
            )
            .await
            .map_err(err)?;
        Ok(row.as_ref().map(Self::agent_request_row))
    }

    /// Lets the client in to the account, and answers the one-use code its redirect carries.
    pub async fn create_agent_grant(&self, account: Uuid, request: &AgentRequestRow, scopes: &[String], projects: &[AgentProject]) -> Result<String, String> {
        let client = self.client().await?;
        let grant = Uuid::new_v4();
        let (scopes, projects) = (serde_json::to_value(scopes).map_err(|e| e.to_string())?, serde_json::to_value(projects).map_err(|e| e.to_string())?);
        client
            .execute("insert into agent_grants (id, account, client, scopes, projects) values ($1, $2, $3, $4, $5)", &[&grant, &account, &request.client, &scopes, &projects])
            .await
            .map_err(err)?;
        let code = Token::new();
        client
            .execute(
                "insert into agent_codes (code_hash, grant_id, redirect_uri, code_challenge, resource, expires_at) \
                 values ($1, $2, $3, $4, $5, now() + make_interval(mins => $6))",
                &[&code.hash, &grant, &request.redirect_uri, &request.code_challenge, &request.resource, &AGENT_CODE_MINUTES],
            )
            .await
            .map_err(err)?;
        Ok(code.text)
    }

    /// Spends an authorization code. A second use, or a late one, is `Replayed`.
    pub async fn take_agent_code(&self, code: &str) -> Result<Spent<AgentCode>, String> {
        let row = self
            .client().await?
            .query_opt(
                "with old as (select used from agent_codes where code_hash = $1 for update) \
                 update agent_codes c set used = true from old where c.code_hash = $1 \
                 returning c.grant_id, c.redirect_uri, c.code_challenge, c.resource, c.expires_at > now(), old.used, \
                 (select client from agent_grants g where g.id = c.grant_id)",
                &[&hash(code)],
            )
            .await
            .map_err(err)?;
        let Some(r) = row else { return Ok(Spent::Unknown) };
        let (grant, live, used): (Uuid, bool, bool) = (r.get(0), r.get(4), r.get(5));
        if used || !live {
            return Ok(Spent::Replayed(grant));
        }
        Ok(Spent::Fresh(AgentCode { grant, redirect_uri: r.get(1), code_challenge: r.get(2), resource: r.get(3), client: r.get(6) }))
    }

    /// A fresh access token and refresh token for the grant, unless it was revoked.
    pub async fn issue_agent_tokens(&self, grant: Uuid) -> Result<Option<(String, String)>, String> {
        let client = self.client().await?;
        let live = client.query_opt("select 1 from agent_grants where id = $1 and not revoked", &[&grant]).await.map_err(err)?;
        if live.is_none() {
            return Ok(None);
        }
        // Expired rows go; a rotated refresh token stays until it expires, so its replay is still caught.
        client.execute("delete from agent_tokens where grant_id = $1 and expires_at < now()", &[&grant]).await.map_err(err)?;
        client.execute("delete from agent_codes where grant_id = $1 and expires_at < now()", &[&grant]).await.map_err(err)?;
        let (access, refresh) = (Token::new(), Token::new());
        client
            .execute(
                "insert into agent_tokens (token_hash, grant_id, kind, expires_at) values \
                 ($1, $3, 'access', now() + make_interval(secs => $4)), ($2, $3, 'refresh', now() + make_interval(days => $5))",
                &[&access.hash, &refresh.hash, &grant, &(AGENT_ACCESS_SECS as f64), &AGENT_REFRESH_DAYS],
            )
            .await
            .map_err(err)?;
        Ok(Some((access.text, refresh.text)))
    }

    /// Spends a refresh token: its grant and client. One already rotated is `Replayed`.
    pub async fn take_agent_refresh(&self, token: &str) -> Result<Spent<(Uuid, String)>, String> {
        let row = self
            .client().await?
            .query_opt(
                "with old as (select rotated from agent_tokens where token_hash = $1 and kind = 'refresh' for update) \
                 update agent_tokens t set rotated = true from old where t.token_hash = $1 and t.kind = 'refresh' \
                 returning t.grant_id, t.expires_at > now(), old.rotated, (select client from agent_grants g where g.id = t.grant_id)",
                &[&hash(token)],
            )
            .await
            .map_err(err)?;
        let Some(r) = row else { return Ok(Spent::Unknown) };
        let (grant, live, rotated): (Uuid, bool, bool) = (r.get(0), r.get(1), r.get(2));
        if rotated {
            return Ok(Spent::Replayed(grant));
        }
        if !live {
            return Ok(Spent::Unknown);
        }
        Ok(Spent::Fresh((grant, r.get(3))))
    }

    /// Ends the grant: its tokens and codes go. With `account`, only that account's grant.
    pub async fn revoke_agent_grant(&self, grant: Uuid, account: Option<Uuid>) -> Result<bool, String> {
        let client = self.client().await?;
        let n = client
            .execute("update agent_grants set revoked = true where id = $1 and ($2::uuid is null or account = $2)", &[&grant, &account])
            .await
            .map_err(err)?;
        if n == 1 {
            client.execute("delete from agent_tokens where grant_id = $1", &[&grant]).await.map_err(err)?;
            client.execute("delete from agent_codes where grant_id = $1", &[&grant]).await.map_err(err)?;
        }
        Ok(n == 1)
    }

    /// The scopes of a grant that still stands.
    pub async fn agent_grant_scopes(&self, grant: Uuid) -> Result<Option<Vec<String>>, String> {
        let row = self.client().await?.query_opt("select scopes from agent_grants where id = $1 and not revoked", &[&grant]).await.map_err(err)?;
        Ok(row.map(|r| strings(r.get(0))))
    }

    /// RFC 7009: forgets one token, of whichever kind. Unknown tokens are no error.
    pub async fn revoke_agent_token(&self, token: &str) -> Result<(), String> {
        self.client().await?.execute("delete from agent_tokens where token_hash = $1", &[&hash(token)]).await.map_err(err).map(drop)
    }

    /// The grant a live access token belongs to, marking it used.
    pub async fn agent_by_token(&self, token: &str) -> Result<Option<AgentGrant>, String> {
        let row = self
            .client().await?
            .query_opt(
                "update agent_grants g set last_used = now() from agent_tokens t, agent_clients c \
                 where t.token_hash = $1 and t.kind = 'access' and t.expires_at > now() and t.grant_id = g.id and c.id = g.client and not g.revoked \
                 returning g.id, g.account, c.name, g.scopes, g.projects",
                &[&hash(token)],
            )
            .await
            .map_err(err)?;
        Ok(row.map(|r| AgentGrant { id: r.get(0), account: r.get(1), name: r.get(2), scopes: strings(r.get(3)), projects: serde_json::from_value(r.get(4)).unwrap_or_default() }))
    }

    pub async fn agent_grants(&self, account: Uuid) -> Result<Vec<AgentView>, String> {
        let rows = self
            .client().await?
            .query(
                "select g.id, c.name, c.redirect_uris, g.scopes, g.projects, extract(epoch from g.created_at)::bigint, extract(epoch from g.last_used)::bigint \
                 from agent_grants g join agent_clients c on c.id = g.client where g.account = $1 and not g.revoked order by g.created_at",
                &[&account],
            )
            .await
            .map_err(err)?;
        Ok(rows
            .iter()
            .map(|r| AgentView {
                id: r.get::<_, Uuid>(0).to_string(),
                name: r.get(1),
                redirect: strings(r.get(2)).into_iter().next().unwrap_or_default(),
                scopes: strings(r.get(3)),
                projects: serde_json::from_value(r.get(4)).unwrap_or_default(),
                created_at: r.get(5),
                last_used: r.get(6),
            })
            .collect())
    }

    /// Records the call under `key`, or answers what an earlier call with that key did.
    pub async fn reserve_agent_call(&self, grant: Uuid, key: &str, tool: &str, args_hash: &[u8], session: Option<Uuid>) -> Result<Reserved, String> {
        let client = self.client().await?;
        let inserted = client
            .execute(
                "insert into agent_calls (grant_id, key, tool, args_hash, state, session) values ($1, $2, $3, $4, 'pending', $5) on conflict do nothing",
                &[&grant, &key, &tool, &args_hash, &session],
            )
            .await
            .map_err(err)?;
        if inserted == 1 {
            return Ok(Reserved::New);
        }
        let r = client
            .query_one(
                "select tool, args_hash, state, session, result, extract(epoch from now() - created_at)::bigint from agent_calls where grant_id = $1 and key = $2",
                &[&grant, &key],
            )
            .await
            .map_err(err)?;
        Ok(Reserved::Existing { tool: r.get(0), args_hash: r.get(1), state: r.get(2), session: r.get(3), result: r.get(4), stale: r.get::<_, i64>(5) > AGENT_CALL_STALE_SECS })
    }

    pub async fn finish_agent_call(&self, grant: Uuid, key: &str, state: &str, result: &Value) -> Result<(), String> {
        self.client().await?
            .execute("update agent_calls set state = $3, result = $4 where grant_id = $1 and key = $2", &[&grant, &key, &state, result])
            .await
            .map_err(err)
            .map(drop)
    }

    /// Forgets a call that did nothing, so the same key may try again.
    pub async fn drop_agent_call(&self, grant: Uuid, key: &str) -> Result<(), String> {
        self.client().await?.execute("delete from agent_calls where grant_id = $1 and key = $2", &[&grant, &key]).await.map_err(err).map(drop)
    }

    pub async fn add_agent_session(&self, grant: Uuid, id: Uuid, machine: Uuid, project: &str, title: &str) -> Result<(), String> {
        self.client().await?
            .execute("insert into agent_sessions (id, grant_id, machine, project, title) values ($1, $2, $3, $4, $5)", &[&id, &grant, &machine, &project, &title])
            .await
            .map_err(err)
            .map(drop)
    }

    pub async fn drop_agent_session(&self, id: Uuid) -> Result<(), String> {
        self.client().await?.execute("delete from agent_sessions where id = $1", &[&id]).await.map_err(err).map(drop)
    }

    /// The grant's sessions, newest first, or only `id` when given. Another grant's never.
    pub async fn agent_sessions(&self, grant: Uuid, id: Option<Uuid>) -> Result<Vec<AgentSession>, String> {
        let rows = self
            .client().await?
            .query(
                "select id, machine, project, title, extract(epoch from created_at)::bigint from agent_sessions \
                 where grant_id = $1 and ($2::uuid is null or id = $2) order by created_at desc limit 50",
                &[&grant, &id],
            )
            .await
            .map_err(err)?;
        Ok(rows.iter().map(|r| AgentSession { id: r.get(0), machine: r.get(1), project: r.get(2), title: r.get(3), created_at: r.get(4) }).collect())
    }

    /// How many sessions the grant started in the last hour.
    pub async fn agent_recent_starts(&self, grant: Uuid) -> Result<i64, String> {
        let row = self
            .client().await?
            .query_one("select count(*) from agent_sessions where grant_id = $1 and created_at > now() - interval '1 hour'", &[&grant])
            .await
            .map_err(err)?;
        Ok(row.get(0))
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
