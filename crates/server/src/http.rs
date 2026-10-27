use crate::db;
use crate::hub::{Account, Hub, Outbox};
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use pinned_core::agent::Subject;
use pinned_core::account::{Failure, NewMachine, NewPassword, Paired, Pairing, Registered, Session, SignIn};
use pinned_core::link::UNAUTHORIZED;
use pinned_core::sync;
use pinned_core::wire::{AccountView, ClientFrame, HubCall, MachineFrame, MachineView, Reply, ToClient, ToMachine};
use serde_json::Value;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeader;
use uuid::Uuid;

const MIN_PASSWORD: usize = 8;
/// Sign-in attempts allowed within `ATTEMPT_WINDOW` from one address, the only limit a stranger can't
/// spend on someone else's behalf.
const ADDRESS_ATTEMPTS: usize = 10;
/// A higher limit per email slows guessing spread over many addresses without letting one
/// address lock the owner out.
const EMAIL_ATTEMPTS: usize = 100;
const ADDRESS_SIGNUPS: usize = 5;
const ATTEMPT_WINDOW: Duration = Duration::from_secs(15 * 60);
/// Each argon2 hash holds about 19 MiB, so only this many run at once whatever arrives.
const CONCURRENT_HASHES: usize = 4;
const AUTH_WAIT: Duration = Duration::from_secs(15);
/// Keeps sockets alive through proxies that close idle ones, such as nginx after 60s.
const PING: Duration = Duration::from_secs(25);
/// A machine pings every 20s; one silent this long is gone.
const MACHINE_SILENT: Duration = Duration::from_secs(60);
const INSTALL: &str = include_str!("install.sh");

pub struct Server {
    hub: Arc<Hub>,
    downloads: PathBuf,
    attempts: Mutex<HashMap<String, Vec<Instant>>>,
    hashing: tokio::sync::Semaphore,
    open_signup: bool,
}

pub fn router(hub: Arc<Hub>, ui: PathBuf, downloads: PathBuf, open_signup: bool) -> Router {
    let server = Arc::new(Server { hub, downloads, attempts: Mutex::default(), hashing: tokio::sync::Semaphore::new(CONCURRENT_HASHES), open_signup });
    Router::new()
        .route("/api/signup", post(signup))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/password", post(change_password))
        .route("/api/machines", post(register))
        .route("/api/pair", post(pair))
        .route("/api/version", get(|| async { env!("CARGO_PKG_VERSION") }))
        .route("/ws/client", get(client_socket))
        .route("/ws/machine", get(machine_socket))
        .route("/install.sh", get(install))
        .route("/download/{file}", get(download))
        // A prompt link, /new?prompt=…, is the app itself, which reads the link from its address.
        .route_service("/new", SetResponseHeader::overriding(ServeFile::new(ui.join("index.html")), header::CACHE_CONTROL, HeaderValue::from_static("no-cache")))
        // Without it a browser guesses the files stay fresh for hours, and runs an old UI after a deploy.
        .fallback_service(SetResponseHeader::overriding(ServeDir::new(ui), header::CACHE_CONTROL, HeaderValue::from_static("no-cache")))
        .with_state(server)
}

struct Error(StatusCode, String);

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (self.0, Json(Failure { error: self.1 })).into_response()
    }
}

fn internal(e: String) -> Error {
    eprintln!("pinned-server: {e}");
    Error(StatusCode::INTERNAL_SERVER_ERROR, "the hub failed; try again".into())
}

type Answer<T> = Result<Json<T>, Error>;

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ").map(str::trim)
}

async fn signed_in(server: &Server, headers: &HeaderMap) -> Result<Uuid, Error> {
    let unauthorized = || Error(StatusCode::UNAUTHORIZED, "sign in first".into());
    let token = bearer(headers).ok_or_else(unauthorized)?;
    server.hub.db.login(token).await.map_err(internal)?.ok_or_else(unauthorized)
}

/// The address a request came from: nginx's last `X-Forwarded-For` hop when it came through a
/// proxy on this host, else the peer itself.
fn client_address(peer: SocketAddr, headers: &HeaderMap) -> String {
    let forwarded = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.rsplit(',').next()).map(str::trim);
    match forwarded {
        Some(hop) if peer.ip().is_loopback() && !hop.is_empty() => hop.to_owned(),
        _ => peer.ip().to_string(),
    }
}

fn new_password(password: &str) -> Result<(), Error> {
    if password.chars().count() < MIN_PASSWORD {
        return Err(Error(StatusCode::BAD_REQUEST, format!("the password needs at least {MIN_PASSWORD} characters")));
    }
    Ok(())
}

fn email(raw: &str) -> Result<String, Error> {
    let email = raw.trim().to_lowercase();
    if !email.contains('@') || email.len() > 254 {
        return Err(Error(StatusCode::BAD_REQUEST, "that is not an email address".into()));
    }
    Ok(email)
}

async fn hash_password(server: &Server, password: String) -> Result<String, Error> {
    let _permit = server.hashing.acquire().await.map_err(|e| internal(e.to_string()))?;
    tokio::task::spawn_blocking(move || {
        use argon2::password_hash::{PasswordHasher, SaltString};
        use rand_core::OsRng;
        argon2::Argon2::default().hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng)).map(|h| h.to_string())
    })
    .await
    .map_err(|e| internal(e.to_string()))?
    .map_err(|e| internal(e.to_string()))
}

async fn password_matches(server: &Server, password: String, hash: String) -> bool {
    let Ok(_permit) = server.hashing.acquire().await else { return false };
    tokio::task::spawn_blocking(move || {
        use argon2::password_hash::{PasswordHash, PasswordVerifier};
        PasswordHash::new(&hash).is_ok_and(|h| argon2::Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
    })
    .await
    .unwrap_or(false)
}

async fn signup(State(server): State<Arc<Server>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(body): Json<SignIn>) -> Answer<Session> {
    let email = email(&body.email)?;
    new_password(&body.password)?;
    // Refused before the email is looked at, so a closed hub tells nobody which accounts it has.
    if !server.open_signup && server.hub.db.has_accounts().await.map_err(internal)? {
        return Err(Error(StatusCode::FORBIDDEN, "this hub already has its account, so sign in with it instead".into()));
    }
    if !server.attempt(&[(format!("signup:{}", client_address(peer, &headers)), ADDRESS_SIGNUPS)]) {
        return Err(Error(StatusCode::TOO_MANY_REQUESTS, "too many sign ups; try again in 15 minutes".into()));
    }
    let hash = hash_password(&server, body.password).await?;
    let account = server.hub.db.create_account(&email, &hash).await.map_err(internal)?;
    let account = account.ok_or_else(|| Error(StatusCode::CONFLICT, "an account with that email already exists".into()))?;
    let token = server.hub.db.create_login(Uuid::parse_str(&account.id).unwrap()).await.map_err(internal)?;
    eprintln!("pinned-server: account {} signed up", account.id);
    Ok(Json(Session { token, account }))
}

impl Server {
    /// Counts an attempt against each key and its limit, unless one is spent. The attempt counts
    /// before the password is checked, so a burst of parallel requests can't all slip under the limit.
    fn attempt(&self, keys: &[(String, usize)]) -> bool {
        let mut attempts = self.attempts.lock().unwrap();
        attempts.retain(|_, at| {
            at.retain(|at| at.elapsed() < ATTEMPT_WINDOW);
            !at.is_empty()
        });
        if keys.iter().any(|(key, limit)| attempts.get(key).is_some_and(|at| at.len() >= *limit)) {
            return false;
        }
        for (key, _) in keys {
            attempts.entry(key.clone()).or_default().push(Instant::now());
        }
        true
    }

    /// Takes back the attempts of a sign in that succeeded.
    fn succeeded(&self, keys: &[(String, usize)]) {
        let mut attempts = self.attempts.lock().unwrap();
        for (key, _) in keys {
            if let Some(at) = attempts.get_mut(key) {
                at.pop();
            }
        }
    }
}

async fn login(State(server): State<Arc<Server>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(body): Json<SignIn>) -> Answer<Session> {
    let email = email(&body.email)?;
    let keys = [(format!("email:{email}"), EMAIL_ATTEMPTS), (format!("address:{}", client_address(peer, &headers)), ADDRESS_ATTEMPTS)];
    if !server.attempt(&keys) {
        return Err(Error(StatusCode::TOO_MANY_REQUESTS, "too many failed sign ins; try again in 15 minutes".into()));
    }
    let found = server.hub.db.account_by_email(&email).await.map_err(internal)?;
    let account = match found {
        Some((account, hash)) => password_matches(&server, body.password, hash).await.then_some(account),
        None => None,
    };
    let Some(account) = account else { return Err(Error(StatusCode::UNAUTHORIZED, "wrong email or password".into())) };
    server.succeeded(&keys);
    let token = server.hub.db.create_login(Uuid::parse_str(&account.id).unwrap()).await.map_err(internal)?;
    Ok(Json(Session { token, account }))
}

async fn logout(State(server): State<Arc<Server>>, headers: HeaderMap) -> Result<StatusCode, Error> {
    let token = bearer(&headers).ok_or_else(|| Error(StatusCode::UNAUTHORIZED, "sign in first".into()))?;
    server.hub.db.logout(token).await.map_err(internal)?;
    server.hub.logged_out(&db::hash(token));
    Ok(StatusCode::NO_CONTENT)
}

async fn change_password(State(server): State<Arc<Server>>, headers: HeaderMap, Json(body): Json<NewPassword>) -> Answer<AccountView> {
    let account = signed_in(&server, &headers).await?;
    new_password(&body.password)?;
    let keys = [(format!("password:{account}"), ADDRESS_ATTEMPTS)];
    if !server.attempt(&keys) {
        return Err(Error(StatusCode::TOO_MANY_REQUESTS, "too many wrong passwords; try again in 15 minutes".into()));
    }
    let (view, hash) = server.hub.db.account(account).await.map_err(internal)?.ok_or_else(|| Error(StatusCode::UNAUTHORIZED, "sign in first".into()))?;
    if !password_matches(&server, body.current, hash).await {
        return Err(Error(StatusCode::FORBIDDEN, "the current password is wrong".into()));
    }
    server.succeeded(&keys);
    let hash = hash_password(&server, body.password).await?;
    server.hub.db.set_password(account, &hash).await.map_err(internal)?;
    eprintln!("pinned-server: account {account} changed its password");
    Ok(Json(view))
}

fn machine_name(name: &str) -> Result<String, Error> {
    let name = name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(Error(StatusCode::BAD_REQUEST, "a machine needs a name of up to 100 characters".into()));
    }
    Ok(name.to_owned())
}

async fn register(State(server): State<Arc<Server>>, headers: HeaderMap, Json(body): Json<NewMachine>) -> Answer<Registered> {
    let account = signed_in(&server, &headers).await?;
    let (machine, token) = server.hub.db.create_machine(account, &machine_name(&body.name)?, body.os.trim()).await.map_err(internal)?;
    eprintln!("pinned-server: account {account} added machine {}", machine.id);
    server.hub.push_machines(&*server.hub.account(account).await.map_err(internal)?).await;
    Ok(Json(Registered { machine, token }))
}

async fn pair(State(server): State<Arc<Server>>, Json(body): Json<Pairing>) -> Answer<Paired> {
    let name = machine_name(&body.name)?;
    let taken = server.hub.db.take_pair_code(body.code.trim()).await.map_err(internal)?;
    let (account, email) = taken.ok_or_else(|| Error(StatusCode::NOT_FOUND, "that pairing code is unknown, used or expired".into()))?;
    let (machine, token) = server.hub.db.create_machine(account, &name, body.os.trim()).await.map_err(internal)?;
    eprintln!("pinned-server: account {account} paired machine {}", machine.id);
    server.hub.push_machines(&*server.hub.account(account).await.map_err(internal)?).await;
    Ok(Json(Paired { machine, token, hub: server.hub.public_url.clone(), email }))
}

async fn install(State(server): State<Arc<Server>>) -> Response {
    let script = INSTALL.replace("{{hub}}", &server.hub.public_url);
    ([(header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8")], script).into_response()
}

async fn download(State(server): State<Arc<Server>>, Path(file): Path<String>) -> Response {
    let named = file.strip_prefix("pinned-server-").is_some_and(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'));
    if !named {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(server.downloads.join(&file)).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn close_unauthorized() -> Message {
    Message::Close(Some(CloseFrame { code: UNAUTHORIZED, reason: "unauthorized".into() }))
}

/// 1013, try again later: the peer reconnects, which a refusal would stop.
fn close_behind() -> Message {
    Message::Close(Some(CloseFrame { code: 1013, reason: "fell behind".into() }))
}

async fn first_text(socket: &mut WebSocket) -> Option<String> {
    match tokio::time::timeout(AUTH_WAIT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => Some(text.to_string()),
        _ => None,
    }
}

fn unreadable_call(text: &str, calls: &[&str]) -> Option<u64> {
    let value: Value = serde_json::from_str(text).ok()?;
    calls.contains(&value["type"].as_str()?).then(|| value["id"].as_u64()).flatten()
}

fn unknown_call(id: u64, e: serde_json::Error) -> Reply {
    Reply::new(id, Err(format!("this hub runs pinned {}, which can't answer that: {e}", env!("CARGO_PKG_VERSION"))))
}

async fn client_socket(State(server): State<Arc<Server>>, upgrade: WebSocketUpgrade) -> Response {
    // A browser sends each message as one frame, and an upload is one message.
    upgrade
        .max_message_size(sync::MESSAGE_LIMIT)
        .max_frame_size(sync::MESSAGE_LIMIT)
        .on_upgrade(move |socket| serve_client(server.hub.clone(), socket))
}

async fn serve_client(hub: Arc<Hub>, mut socket: WebSocket) {
    let token = match first_text(&mut socket).await.map(|t| serde_json::from_str::<ClientFrame>(&t)) {
        Some(Ok(ClientFrame::Auth { token })) => token,
        _ => return drop(socket.send(close_unauthorized()).await),
    };
    let account = match hub.db.login(&token).await {
        Ok(Some(id)) => hub.account(id).await,
        Ok(None) => return drop(socket.send(close_unauthorized()).await),
        Err(e) => Err(e),
    };
    let account = match account {
        Ok(account) => account,
        Err(e) => return eprintln!("pinned-server: client sign in failed: {e}"),
    };
    let machines = match hub.machines(&account).await {
        Ok(machines) => machines,
        Err(e) => return eprintln!("pinned-server: client sign in failed: {e}"),
    };
    let id = hub.next_socket();
    let (out, mut outbox, overflowed) = Outbox::new();
    account.client_connected(id, db::hash(&token), out, machines);
    eprintln!("pinned-server: client {id} of account {} connected", account.id);

    let (mut sink, mut stream) = socket.split();
    let mut ping = tokio::time::interval(PING);
    loop {
        tokio::select! {
            text = outbox.recv() => match text {
                Some(text) => if sink.send(Message::Text(text.into())).await.is_err() { break },
                None => {
                    let _ = sink.send(close_unauthorized()).await;
                    break;
                }
            },
            frame = stream.next() => match frame {
                Some(Ok(Message::Text(text))) => client_frame(&hub, &account, id, &text),
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            _ = ping.tick() => if sink.send(Message::Ping(Default::default())).await.is_err() { break },
            _ = overflowed.notified() => {
                let _ = sink.send(close_behind()).await;
                break;
            }
        }
    }
    // Answers a client's close frame, so the client sees a clean close instead of 1006.
    let _ = sink.close().await;
    account.client_gone(id);
    eprintln!("pinned-server: client {id} of account {} disconnected", account.id);
}

fn client_frame(hub: &Arc<Hub>, account: &Arc<Account>, client: u64, text: &str) {
    match serde_json::from_str::<ClientFrame>(text) {
        Ok(ClientFrame::Call { id, machine, call }) => {
            // Sent before anything is spawned, so fast typing reaches the machine in order.
            let sent = account.send_call(&machine, call);
            let (hub, account) = (hub.clone(), account.clone());
            tokio::spawn(async move {
                let result = hub.client_call(&account, &machine, sent).await;
                if let Err(e) = &result {
                    eprintln!("pinned-server: client {client}'s call to {machine} failed: {e}");
                }
                account.to_client(client, &ToClient::Reply(Reply::new(id, result)));
            });
        }
        Ok(ClientFrame::Hub { id, call }) => {
            let (hub, account) = (hub.clone(), account.clone());
            tokio::spawn(async move {
                let result = hub.hub_call(&account, call).await;
                if let Err(e) = &result {
                    eprintln!("pinned-server: client {client}'s hub call failed: {e}");
                }
                account.to_client(client, &ToClient::Reply(Reply::new(id, result)));
            });
        }
        Ok(ClientFrame::Attach(target)) => account.attach(client, target),
        Ok(ClientFrame::Detach(target)) => account.detach(client, target),
        Ok(ClientFrame::Auth { .. }) => {}
        Err(e) => {
            if let Some(id) = unreadable_call(text, &["call", "hub"]) {
                account.to_client(client, &ToClient::Reply(unknown_call(id, e)));
            }
        }
    }
}

async fn machine_socket(State(server): State<Arc<Server>>, upgrade: WebSocketUpgrade) -> Response {
    upgrade
        .max_message_size(sync::MESSAGE_LIMIT)
        .max_frame_size(sync::MESSAGE_LIMIT)
        .on_upgrade(move |socket| serve_machine(server.hub.clone(), socket))
}

async fn serve_machine(hub: Arc<Hub>, mut socket: WebSocket) {
    let (token, os, version) = match first_text(&mut socket).await.map(|t| serde_json::from_str::<MachineFrame>(&t)) {
        Some(Ok(MachineFrame::Auth { token, os, version, .. })) => (token, os, version),
        _ => return drop(socket.send(close_unauthorized()).await),
    };
    let row = match hub.db.machine_connected(&token, &os).await {
        Ok(Some(row)) => row,
        Ok(None) => return drop(socket.send(close_unauthorized()).await),
        Err(e) => return eprintln!("pinned-server: machine sign in failed: {e}"),
    };
    let account = match hub.account(row.account).await {
        Ok(account) => account,
        Err(e) => return eprintln!("pinned-server: machine sign in failed: {e}"),
    };
    let (machine, socket_id) = (row.id.to_string(), hub.next_socket());
    let (out, mut outbox, overflowed) = Outbox::new();
    let view = MachineView { id: machine.clone(), name: row.name.clone(), os, online: true, last_seen: 0 };
    let _ = out.send(serde_json::to_string(&ToMachine::Hello { machine: view }).unwrap());
    // A machine that reconnects before its old socket timed out goes offline and back, so clients attach again.
    if account.went_offline(&machine, None) {
        hub.push_machines(&account).await;
    }
    account.came_online(&machine, socket_id, row.name, out);
    hub.push_machines(&account).await;
    eprintln!("pinned-server: machine {machine} of account {} connected (pinned {version})", account.id);

    let (mut sink, mut stream) = socket.split();
    let mut ping = tokio::time::interval(PING);
    loop {
        tokio::select! {
            text = outbox.recv() => match text {
                Some(text) => if sink.send(Message::Text(text.into())).await.is_err() { break },
                None => break,
            },
            frame = tokio::time::timeout(MACHINE_SILENT, stream.next()) => match frame {
                Ok(Some(Ok(Message::Text(text)))) => machine_frame(&hub, &account, &machine, socket_id, &text),
                Ok(Some(Ok(Message::Close(_)) | Err(_)) | None) | Err(_) => break,
                Ok(Some(Ok(_))) => {}
            },
            _ = ping.tick() => if sink.send(Message::Ping(Default::default())).await.is_err() { break },
            _ = overflowed.notified() => {
                let _ = sink.send(close_behind()).await;
                break;
            }
        }
    }
    let _ = sink.close().await;
    if account.went_offline(&machine, Some(socket_id)) {
        if let Err(e) = hub.db.machine_seen(row.id).await {
            eprintln!("pinned-server: could not record when machine {machine} was last seen: {e}");
        }
        hub.push_machines(&account).await;
    }
    eprintln!("pinned-server: machine {machine} of account {} disconnected", account.id);
}

fn machine_frame(hub: &Arc<Hub>, account: &Arc<Account>, machine: &str, socket: u64, text: &str) {
    match serde_json::from_str::<MachineFrame>(text) {
        Ok(MachineFrame::Reply(reply)) => account.answered(machine, socket, reply),
        Ok(MachineFrame::Event { event }) => account.machine_event(machine, socket, event),
        Ok(MachineFrame::Output { viewer, tab, data }) => account.output(machine, &viewer, tab, data),
        Ok(MachineFrame::Call { id, call }) => {
            let (hub, account, machine) = (hub.clone(), account.clone(), machine.to_owned());
            tokio::spawn(async move {
                // A machine holds a lesser credential than a login: it may only pass on the callbacks
                // of sessions it runs, so a prompt-injected session can't act for another connector or thread,
                // import into an account with no connectors yet, and sync its projects.
                let work = match &call {
                    HubCall::Stage { connector, subject, .. } | HubCall::Checkpoint { connector, subject, .. } => {
                        Some((connector.clone(), subject.clone()))
                    }
                    HubCall::SlackReply { connector, channel, thread, .. }
                    | HubCall::LinearCreate { connector, channel, thread, .. }
                    | HubCall::LinearTake { connector, channel, thread, .. } => {
                        Some((connector.clone(), Subject::SlackThread { channel: channel.clone(), thread: thread.clone() }))
                    }
                    _ => None,
                };
                let result = match work {
                    Some((connector, subject)) if account.working_on(&connector, &subject, |id| *id == machine).is_some() => {
                        match hub.held_by(&account, &connector, &subject, &machine).await {
                            Ok(()) => hub.hub_call(&account, call).await,
                            Err(e) => Err(e),
                        }
                    }
                    Some(_) => Err("this machine runs no session of that workflow on that subject".into()),
                    // An app's first sign-in moves its own connectors in, which only an empty account takes,
                    // and every machine keeps its projects' files in step with the account's.
                    None if matches!(
                        call,
                        HubCall::Import { .. } | HubCall::ShareProject { .. } | HubCall::SyncList { .. } | HubCall::SyncPush { .. } | HubCall::SyncPull { .. }
                    ) =>
                    {
                        hub.hub_call(&account, call).await
                    }
                    None => Err("a machine may only forward a workflow session's callbacks".into()),
                };
                if let Err(e) = &result {
                    eprintln!("pinned-server: machine {machine}'s hub call failed: {e}");
                }
                account.to_machine(&machine, socket, &ToMachine::Reply(Reply::new(id, result)));
            });
        }
        Ok(MachineFrame::Auth { .. }) => {}
        Err(e) => {
            if let Some(id) = unreadable_call(text, &["call"]) {
                account.to_machine(machine, socket, &ToMachine::Reply(unknown_call(id, e)));
            }
        }
    }
}
