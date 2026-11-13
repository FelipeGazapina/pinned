use crate::db;
use crate::{mcp, oauth};
use crate::hub::{Account, Hub, Outbox};
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use pinned_core::agent::Subject;
use pinned_core::account::{DictationFeedback, Failure, NewMachine, NewPassword, Paired, Pairing, Registered, Session, SignIn, Signup, SignupInfo};
use pinned_core::link::UNAUTHORIZED;
use pinned_core::surface::{self, Side};
use pinned_core::sync;
use pinned_core::wire::{AccountView, Call, ClientFrame, HubCall, MachineEvent, MachineFrame, MachineView, Reply, ToClient, ToMachine, UpdateStatus};
use serde_json::Value;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeader;
use uuid::Uuid;

/// The WebSocket close code for an endpoint that is going down.
pub const GOING_AWAY: u16 = 1001;
const MIN_PASSWORD: usize = 8;
/// Sign-in attempts allowed within `ATTEMPT_WINDOW` from one address, the only limit a stranger can't
/// spend on someone else's behalf.
const ADDRESS_ATTEMPTS: usize = 10;
/// Failed sign ins allowed within `ATTEMPT_WINDOW` against one email from addresses that never signed in
/// to it, however many addresses they come from. Above `ADDRESS_ATTEMPTS`, so one address can't lock the
/// owner out; an address the owner signed in from is not held by it.
const EMAIL_ATTEMPTS: usize = 30;
/// Wrong passwords allowed within `ATTEMPT_WINDOW` to unlock one account's apps, over all its logins.
const ACCOUNT_UNLOCKS: usize = 100;
const ADDRESS_SIGNUPS: usize = 5;
/// Sign ups the whole hub takes within `ATTEMPT_WINDOW`, whichever addresses they come from: a burst of
/// invited testers fits, a client that keeps changing its address does not get past it.
const HUB_SIGNUPS: usize = 50;
/// Keys the limiter holds at most, under 20 MB when filled by one-off keys of the longest email. Rotating addresses or emails can't grow
/// it past this; when it is full a request that needs a new key is refused, and no live key is dropped.
const ATTEMPT_KEYS: usize = 50_000;
/// Addresses an account signed in from that the limiter remembers at most, and for how long.
const KNOWN_ADDRESSES: usize = 10_000;
const KNOWN_FOR: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// The one answer to a sign up without a good code, so it tells nobody which codes or accounts exist.
/// The one answer to a sign up at a hub that holds as many accounts as it was told to.
const NOT_TAKING: &str = "This hub is not taking new accounts.";
const INVITE_NEEDED: &str = "this hub needs a valid invite code to create an account";
const ATTEMPT_WINDOW: Duration = Duration::from_secs(15 * 60);
/// Each argon2 hash holds about 19 MiB, so only this many run at once whatever arrives.
const CONCURRENT_HASHES: usize = 4;
const AUTH_WAIT: Duration = Duration::from_secs(15);
/// Keeps sockets alive through proxies that close idle ones, such as nginx after 60s.
const PING: Duration = Duration::from_secs(25);
/// A machine pings every 20s; one silent this long is gone.
const MACHINE_SILENT: Duration = Duration::from_secs(60);
const INSTALL: &str = include_str!("install.sh");
/// Bounds on one shared dictation, checked before it reaches the learning database.
const DICTATION_TEXT_CHARS: usize = 20_000;
const DICTATION_LABEL_CHARS: usize = 40;
const DICTATION_LONGEST_MS: u64 = 24 * 60 * 60 * 1000;

/// Which proxy in front of the hub the hub believes about a client's address. Off unless the hub is told.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrustedProxy {
    /// No proxy is believed, except one on this host: nginx on loopback, whose last `X-Forwarded-For` hop counts.
    #[default]
    Off,
    /// Railway's edge, which sets `X-Real-IP` to the address it accepted the connection from.
    Railway,
}

impl std::str::FromStr for TrustedProxy {
    type Err = String;

    fn from_str(name: &str) -> Result<TrustedProxy, String> {
        match name {
            "off" => Ok(TrustedProxy::Off),
            "railway" => Ok(TrustedProxy::Railway),
            other => Err(format!("--trusted-proxy takes off or railway, not {other}")),
        }
    }
}

/// Who and how many the hub lets in, beyond `--signup`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Access {
    pub trusted_proxy: TrustedProxy,
    /// Accounts the hub holds at most; none means no limit.
    pub max_accounts: Option<u32>,
}

pub struct Server {
    pub(crate) hub: Arc<Hub>,
    access: Access,
    /// Held from counting accounts to making one, so a hub with a cap can't pass it.
    signups: tokio::sync::Mutex<()>,
    downloads: PathBuf,
    attempts: Mutex<Limiter>,
    hashing: tokio::sync::Semaphore,
    learning: Option<db::Learning>,
}

/// Routes a released hub keeps serving. Dropping one breaks apps that still call it.
pub const ROUTES: &[&str] = &[
    "POST /api/signup",
    "GET /api/signup",
    "POST /api/login",
    "POST /api/logout",
    "POST /api/password",
    "POST /api/machines",
    "POST /api/pair",
    "POST /api/dictation/feedback",
    "GET /api/version",
    "GET /ws/client",
    "GET /ws/machine",
    "GET /install.sh",
    "GET /download/{file}",
    "GET /new",
    "GET /.well-known/oauth-protected-resource",
    "GET /.well-known/oauth-protected-resource/mcp",
    "GET /.well-known/oauth-authorization-server",
    "POST /oauth/register",
    "GET /oauth/authorize",
    "POST /oauth/token",
    "POST /oauth/revoke",
    "GET /connect-agent",
    "POST /mcp",
    "GET /mcp",
    "DELETE /mcp",
];

fn route(spec: &str) -> &str {
    assert!(ROUTES.contains(&spec), "{spec} is missing from ROUTES");
    spec.split_once(' ').expect("METHOD /path").1
}

pub fn router(hub: Arc<Hub>, ui: PathBuf, downloads: PathBuf, learning: Option<db::Learning>, access: Access) -> Router {
    let server = Arc::new(Server { hub, access, signups: tokio::sync::Mutex::new(()), downloads, attempts: Mutex::new(Limiter::new(ATTEMPT_KEYS)), hashing: tokio::sync::Semaphore::new(CONCURRENT_HASHES), learning });
    Router::new()
        .route(route("POST /api/signup"), post(signup))
        // The app's sign-in form reads it from whichever hub the user types, so any origin may.
        .route(
            route("GET /api/signup"),
            get(|State(server): State<Arc<Server>>| async move { ([(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")], Json(SignupInfo { signup: server.hub.signup })) }),
        )
        .route(route("POST /api/login"), post(login))
        .route(route("POST /api/logout"), post(logout))
        .route(route("POST /api/password"), post(change_password))
        .route(route("POST /api/machines"), post(register))
        .route(route("POST /api/pair"), post(pair))
        .route(route("POST /api/dictation/feedback"), post(dictation_feedback))
        .route(route("GET /api/version"), get(|| async { env!("CARGO_PKG_VERSION") }))
        .route(route("GET /ws/client"), get(client_socket))
        .route(route("GET /ws/machine"), get(machine_socket))
        .route(route("GET /install.sh"), get(install))
        .route(route("GET /download/{file}"), get(download))
        // A prompt link, /new?prompt=…, is the app itself, which reads the link from its address.
        .route_service(route("GET /new"), SetResponseHeader::overriding(ServeFile::new(ui.join("index.html")), header::CACHE_CONTROL, HeaderValue::from_static("no-cache")))
        .route(route("GET /.well-known/oauth-protected-resource"), get(oauth::resource_metadata))
        .route(route("GET /.well-known/oauth-protected-resource/mcp"), get(oauth::resource_metadata))
        .route(route("GET /.well-known/oauth-authorization-server"), get(oauth::server_metadata))
        .route(route("POST /oauth/register"), post(oauth::register))
        .route(route("GET /oauth/authorize"), get(oauth::authorize))
        .route(route("POST /oauth/token"), post(oauth::token))
        .route(route("POST /oauth/revoke"), post(oauth::revoke))
        // The web app shows the pending authorization in its address to the signed-in account.
        .route_service(
            route("GET /connect-agent"),
            SetResponseHeader::overriding(
                SetResponseHeader::overriding(ServeFile::new(ui.join("index.html")), header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            ),
        )
        .route(route("POST /mcp"), post(mcp::post))
        .route(route("GET /mcp"), get(mcp::not_offered))
        .route(route("DELETE /mcp"), axum::routing::delete(mcp::not_offered))
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

pub(crate) fn internal_response(e: String) -> Response {
    internal(e).into_response()
}

fn internal(e: String) -> Error {
    eprintln!("pinned-server: {e}");
    Error(StatusCode::INTERNAL_SERVER_ERROR, "the hub failed; try again".into())
}

type Answer<T> = Result<Json<T>, Error>;

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ").map(str::trim)
}

async fn signed_in(server: &Server, headers: &HeaderMap) -> Result<db::Login, Error> {
    let unauthorized = || Error(StatusCode::UNAUTHORIZED, "sign in first".into());
    let token = bearer(headers).ok_or_else(unauthorized)?;
    server.hub.db.login(token, false).await.map_err(internal)?.ok_or_else(unauthorized)
}

/// The address a request came from. Behind a proxy the hub is told to trust, the address that proxy
/// puts in its own header; else nginx's last `X-Forwarded-For` hop when the request came through a
/// proxy on this host; else the peer itself.
///
/// Railway's edge overwrites `X-Real-IP` with the address it accepted the connection from, on every
/// request, so the whole header is the client and there is no hop to pick. `X-Forwarded-For` is not
/// read for it: its hop count is not fixed and its left part may be the client's own. A header that is
/// missing or is not one address (a list, text) leaves the peer, the proxy, as the key.
pub(crate) fn client_address(trusted: TrustedProxy, peer: SocketAddr, headers: &HeaderMap) -> String {
    if trusted == TrustedProxy::Railway {
        let real = headers.get("x-real-ip").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<std::net::IpAddr>().ok());
        return real.map_or_else(|| peer.ip().to_string(), |ip| ip.to_canonical().to_string());
    }
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

/// Whether `password` is the one `hash` holds. With no hash, as for an email no account has, it checks
/// against a stand-in that never matches at the same cost, so the time taken doesn't tell whether the
/// account exists.
async fn password_matches(server: &Server, password: String, hash: Option<String>) -> bool {
    let Ok(_permit) = server.hashing.acquire().await else { return false };
    tokio::task::spawn_blocking(move || {
        use argon2::password_hash::{PasswordHash, PasswordVerifier};
        let known = hash.is_some();
        let hash = hash.unwrap_or_else(|| no_account_hash().to_owned());
        let matches = PasswordHash::new(&hash).is_ok_and(|h| argon2::Argon2::default().verify_password(password.as_bytes(), &h).is_ok());
        known && matches
    })
    .await
    .unwrap_or(false)
}

/// An argon2 hash made as an account's is, of a random password nobody knows.
fn no_account_hash() -> &'static str {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| {
        use argon2::password_hash::{PasswordHasher, SaltString};
        use rand_core::{OsRng, RngCore};
        let unknown = OsRng.next_u64().to_string();
        argon2::Argon2::default().hash_password(unknown.as_bytes(), &SaltString::generate(&mut OsRng)).map(|h| h.to_string()).unwrap_or_default()
    })
}

/// Whether the hub holds as many accounts as `--max-accounts` lets it. The first account counts like any other.
async fn at_cap(server: &Server) -> Result<bool, Error> {
    let Some(cap) = server.access.max_accounts else { return Ok(false) };
    Ok(server.hub.db.account_count().await.map_err(internal)? >= i64::from(cap))
}

async fn signup(State(server): State<Arc<Server>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(body): Json<SignIn>) -> Answer<Session> {
    let email = email(&body.email)?;
    new_password(&body.password)?;
    // Refused before the email or the code is looked at, so a closed hub tells nobody which accounts it has.
    let first = !server.hub.db.has_accounts().await.map_err(internal)?;
    if server.hub.signup == Signup::First && !first {
        return Err(Error(StatusCode::FORBIDDEN, "this hub already has its account, so sign in with it instead".into()));
    }
    let keys = [(format!("signup:{}", server.client_address(peer, &headers)), ADDRESS_SIGNUPS), ("hub:signup".to_owned(), HUB_SIGNUPS)];
    if !server.attempt(&keys) {
        return Err(Error(StatusCode::TOO_MANY_REQUESTS, "too many sign ups; try again in 15 minutes".into()));
    }
    // After the limiter, so a full hub's answer costs a counted attempt before it costs a query.
    if at_cap(&server).await? {
        return Err(Error(StatusCode::FORBIDDEN, NOT_TAKING.into()));
    }
    let invite_needed = || Error(StatusCode::FORBIDDEN, INVITE_NEEDED.into());
    // The first account of an invite hub needs no code, so a fresh hub can start.
    let invite = match server.hub.signup {
        Signup::Invite if !first => {
            let code = body.invite.as_deref().map(str::trim).filter(|code| !code.is_empty()).ok_or_else(invite_needed)?;
            Some(db::invite_hash(code))
        }
        _ => None,
    };
    let hash = hash_password(&server, body.password).await?;
    // Counted again with the account made under one lock, so signups racing at the cap can't pass it.
    let _slot = match server.access.max_accounts {
        Some(_) => Some(server.signups.lock().await),
        None => None,
    };
    if at_cap(&server).await? {
        return Err(Error(StatusCode::FORBIDDEN, NOT_TAKING.into()));
    }
    let account = match invite {
        Some(code) => match server.hub.db.create_invited_account(&code, &email, &hash).await.map_err(internal)? {
            db::Invited::Created(account) => Some(account),
            db::Invited::NoInvite => return Err(invite_needed()),
            db::Invited::EmailTaken => None,
        },
        None => server.hub.db.create_account(&email, &hash).await.map_err(internal)?,
    };
    let account = account.ok_or_else(|| Error(StatusCode::CONFLICT, "an account with that email already exists".into()))?;
    let token = server.hub.db.create_login(Uuid::parse_str(&account.id).unwrap()).await.map_err(internal)?;
    eprintln!("pinned-server: account {} signed up", account.id);
    Ok(Json(Session { token, account }))
}

impl Server {
    pub(crate) fn client_address(&self, peer: SocketAddr, headers: &HeaderMap) -> String {
        client_address(self.access.trusted_proxy, peer, headers)
    }

    /// Counts an attempt against each key and its limit, unless one is spent. The attempt counts
    /// before the password is checked, so a burst of parallel requests can't all slip under the limit.
    pub(crate) fn attempt(&self, keys: &[(String, usize)]) -> bool {
        self.attempts.lock().unwrap().attempt(keys, Instant::now())
    }

    /// Takes back the attempts of a sign in that succeeded.
    pub(crate) fn succeeded(&self, keys: &[(String, usize)]) {
        self.attempts.lock().unwrap().succeeded(keys);
    }
}

/// Attempts within `ATTEMPT_WINDOW`, per key, and the addresses each email signed in from.
struct Limiter {
    keys: HashMap<String, Vec<Instant>>,
    cap: usize,
    /// When the expired keys were last dropped, so a full limiter is not swept on every request.
    swept: Option<Instant>,
    known: HashMap<String, Instant>,
    /// When a full limiter last said so.
    full_logged: Option<Instant>,
}

impl Limiter {
    fn new(cap: usize) -> Limiter {
        Limiter { keys: HashMap::new(), cap, swept: None, known: HashMap::new(), full_logged: None }
    }

    /// A key holds at most its limit of stamps: one at its limit takes no more.
    fn attempt(&mut self, keys: &[(String, usize)], now: Instant) -> bool {
        let live = |at: &Instant| now.saturating_duration_since(*at) < ATTEMPT_WINDOW;
        for (key, _) in keys {
            if let Some(at) = self.keys.get_mut(key) {
                at.retain(live);
            }
        }
        if keys.iter().any(|(key, limit)| self.keys.get(key).is_some_and(|at| at.len() >= *limit)) {
            return false;
        }
        let new = |limiter: &Limiter| keys.iter().filter(|(key, _)| !limiter.keys.contains_key(key)).count();
        let full = |limiter: &Limiter| limiter.keys.len() + new(limiter) > limiter.cap;
        let since = |at: Option<Instant>| at.map_or(Duration::MAX, |at| now.saturating_duration_since(at));
        // A whole sweep at most once a second when full, else once a minute.
        if since(self.swept) >= Duration::from_secs(60) || (full(self) && since(self.swept) >= Duration::from_secs(1)) {
            self.keys.retain(|_, at| {
                at.retain(live);
                !at.is_empty()
            });
            self.swept = Some(now);
        }
        if full(self) {
            if since(self.full_logged) >= Duration::from_secs(60 * 60) {
                eprintln!("pinned-server: the attempt limiter holds {} keys, its most; requests that need a new key are refused until old ones expire", self.cap);
                self.full_logged = Some(now);
            }
            return false;
        }
        for (key, _) in keys {
            self.keys.entry(key.clone()).or_default().push(now);
        }
        true
    }

    /// Whether `key`, an email and an address, signed in within `KNOWN_FOR`.
    fn known(&self, key: &str, now: Instant) -> bool {
        self.known.get(key).is_some_and(|at| now.saturating_duration_since(*at) < KNOWN_FOR)
    }

    /// Notes a sign in, unless the limiter already holds as many as it keeps: then an address is not
    /// remembered, and only loses the way past the email's bound.
    fn remember(&mut self, key: String, now: Instant) {
        if self.known.len() >= KNOWN_ADDRESSES && !self.known.contains_key(&key) {
            self.known.retain(|_, at| now.saturating_duration_since(*at) < KNOWN_FOR);
        }
        if self.known.len() < KNOWN_ADDRESSES || self.known.contains_key(&key) {
            self.known.insert(key, now);
        }
    }

    fn succeeded(&mut self, keys: &[(String, usize)]) {
        for (key, _) in keys {
            if let Some(at) = self.keys.get_mut(key) {
                at.pop();
            }
        }
    }
}

async fn login(State(server): State<Arc<Server>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(body): Json<SignIn>) -> Answer<Session> {
    let email = email(&body.email)?;
    let address = server.client_address(peer, &headers);
    // Guesses from addresses that never signed in to this email share one bound, so changing address
    // doesn't buy more of them; the owner's own address is held only by its own limit.
    let known = format!("{email}\n{address}");
    let mut keys = vec![(format!("address:{address}"), ADDRESS_ATTEMPTS)];
    if !server.attempts.lock().unwrap().known(&known, Instant::now()) {
        keys.push((format!("email:{email}"), EMAIL_ATTEMPTS));
    }
    if !server.attempt(&keys) {
        return Err(Error(StatusCode::TOO_MANY_REQUESTS, "too many failed sign ins; try again in 15 minutes".into()));
    }
    // An unknown email takes the same path, hash included, to the same answer as a wrong password.
    let (account, hash) = server.hub.db.account_by_email(&email).await.map_err(internal)?.unzip();
    let matches = password_matches(&server, body.password, hash).await;
    let Some(account) = account.filter(|_| matches) else { return Err(Error(StatusCode::UNAUTHORIZED, "wrong email or password".into())) };
    server.succeeded(&keys);
    server.attempts.lock().unwrap().remember(known, Instant::now());
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
    let account = signed_in(&server, &headers).await?.account;
    new_password(&body.password)?;
    let keys = [(format!("password:{account}"), ADDRESS_ATTEMPTS)];
    if !server.attempt(&keys) {
        return Err(Error(StatusCode::TOO_MANY_REQUESTS, "too many wrong passwords; try again in 15 minutes".into()));
    }
    let (view, hash) = server.hub.db.account(account).await.map_err(internal)?.ok_or_else(|| Error(StatusCode::UNAUTHORIZED, "sign in first".into()))?;
    if !password_matches(&server, body.current, Some(hash)).await {
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
    let login = signed_in(&server, &headers).await?;
    // An app signs in, adds its machine, then locks the login. A locked one adding a machine is
    // whatever read it off that machine's disk.
    if login.locked {
        return Err(Error(StatusCode::FORBIDDEN, "this login adds no machine: pair it with a code from the app".into()));
    }
    let account = login.account;
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

/// Keeps one dictation the user rated and agreed to share, in the learning database.
async fn dictation_feedback(State(server): State<Arc<Server>>, headers: HeaderMap, Json(body): Json<DictationFeedback>) -> Result<StatusCode, Error> {
    let account = signed_in(&server, &headers).await?.account;
    let learning = server.learning.as_ref().ok_or_else(|| Error(StatusCode::SERVICE_UNAVAILABLE, "this hub keeps no dictation feedback: it runs without a learning database".into()))?;
    dictation_fields(&body)?;
    learning.save_dictation(account, &body).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

fn dictation_fields(feedback: &DictationFeedback) -> Result<(), Error> {
    let bad = |why: String| Err(Error(StatusCode::BAD_REQUEST, why));
    // Postgres text can't hold a NUL.
    let texts = [("raw", &feedback.raw), ("cleaned", &feedback.cleaned), ("final_text", &feedback.final_text)];
    for (name, text) in texts {
        if text.chars().count() > DICTATION_TEXT_CHARS || text.contains('\0') {
            return bad(format!("{name} must be text of up to {DICTATION_TEXT_CHARS} characters"));
        }
    }
    if feedback.raw.trim().is_empty() {
        return bad("raw is empty: there is no dictation to keep".into());
    }
    let labels = [("locale", &feedback.locale), ("app_version", &feedback.app_version), ("platform", &feedback.platform)];
    for (name, label) in labels {
        if label.is_empty() || label.chars().count() > DICTATION_LABEL_CHARS || label.chars().any(|c| c.is_control()) {
            return bad(format!("{name} must be one line of 1 to {DICTATION_LABEL_CHARS} characters"));
        }
    }
    if feedback.duration_ms > DICTATION_LONGEST_MS {
        return bad("duration_ms is longer than a day".into());
    }
    Ok(())
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

/// 1013 to a machine whose sign in never arrived or could not be read. A slow or overloaded hub must
/// not read as a refusal: a box's node leaves after 15 minutes of 4001, so 4001 means only "no machine
/// has this token".
fn close_no_sign_in() -> Message {
    Message::Close(Some(CloseFrame { code: 1013, reason: "no sign in in time".into() }))
}

/// Resolves once the hub is told to stop. It doesn't hand out the watch's guard, which can't be held across an await.
async fn hub_going_away(away: &mut tokio::sync::watch::Receiver<bool>) {
    let _ = away.wait_for(|away| *away).await;
}

/// 1001, going away: the hub is stopping, and the peer reconnects at once to whichever hub follows.
fn close_going_away() -> Message {
    Message::Close(Some(CloseFrame { code: GOING_AWAY, reason: "the hub is restarting".into() }))
}

async fn first_text(socket: &mut WebSocket) -> Option<String> {
    match tokio::time::timeout(AUTH_WAIT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => Some(text.to_string()),
        _ => None,
    }
}

/// Unlocks client `client` once `password` is its account's: the owner is there, not something that
/// read the app's login off a disk.
async fn unlock(server: &Server, account: &Account, client: u64, password: String) -> Result<Value, String> {
    let login: String = account.login_of(client).ok_or("this connection closed")?.iter().map(|b| format!("{b:02x}")).collect();
    // Per login, so whatever guesses with a login read off a disk locks out only that login; the
    // owner's other devices still unlock.
    let keys = [(format!("unlock:{login}"), ADDRESS_ATTEMPTS), (format!("unlock:{}", account.id), ACCOUNT_UNLOCKS)];
    if !server.attempt(&keys) {
        return Err("too many wrong passwords; try again in 15 minutes".into());
    }
    let (_, hash) = server.hub.db.account(account.id).await?.ok_or("this account no longer exists")?;
    if !password_matches(server, password, Some(hash)).await {
        return Err("that is not this account's password".into());
    }
    server.succeeded(&keys);
    let unlocked = server.hub.unlock(account, client).ok_or("this connection closed")?;
    serde_json::to_value(unlocked).map_err(|e| e.to_string())
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
        .on_upgrade(move |socket| serve_client(server, socket))
}

async fn serve_client(server: Arc<Server>, mut socket: WebSocket) {
    let hub = server.hub.clone();
    let (token, lock, unlock) = match first_text(&mut socket).await.map(|t| serde_json::from_str::<ClientFrame>(&t)) {
        Some(Ok(ClientFrame::Auth { token, lock, unlock })) => (token, lock, unlock),
        _ => return drop(socket.send(close_unauthorized()).await),
    };
    let (account, locked) = match hub.db.login(&token, lock).await {
        Ok(Some(login)) => (hub.account(login.account).await, login.locked),
        Ok(None) => return drop(socket.send(close_unauthorized()).await),
        Err(e) => (Err(e), false),
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
    let login = db::hash(&token);
    let unlocked = unlock.and_then(|unlock| hub.unlocked_until(&unlock, &login));
    account.client_connected(id, login, locked, unlocked, out, machines);
    eprintln!("pinned-server: client {id} of account {} connected", account.id);
    let (pushing, to) = (hub.clone(), account.clone());
    tokio::spawn(async move { crate::surface::send_confirmations(&pushing, &to, id).await });

    let (mut sink, mut stream) = socket.split();
    let mut ping = tokio::time::interval(PING);
    let mut going_away = hub.watch_going_away();
    loop {
        tokio::select! {
            text = outbox.recv() => match text {
                Some(text) => if sink.send(Message::Text(text.into())).await.is_err() { break },
                None => {
                    let _ = sink.send(close_unauthorized()).await;
                    break;
                }
            },
            _ = hub_going_away(&mut going_away) => {
                let _ = sink.send(close_going_away()).await;
                break;
            }
            frame = stream.next() => match frame {
                Some(Ok(Message::Text(text))) => client_frame(&server, &account, id, &text),
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

fn client_frame(server: &Arc<Server>, account: &Arc<Account>, client: u64, text: &str) {
    let hub = &server.hub;
    let locked = |id| account.to_client(client, &ToClient::Reply(Reply::new(id, Err(surface::LOCKED.to_owned()))));
    match serde_json::from_str::<ClientFrame>(text) {
        Ok(ClientFrame::Call { id, call, .. }) if account.needs_unlock(client, Side::Node, &call) => locked(id),
        Ok(ClientFrame::Hub { id, call }) if account.needs_unlock(client, Side::Hub, &call) => locked(id),
        Ok(ClientFrame::Hub { id, call: HubCall::Unlock { password } }) => {
            let (server, account) = (server.clone(), account.clone());
            tokio::spawn(async move {
                let result = unlock(&server, &account, client, password).await;
                account.to_client(client, &ToClient::Reply(Reply::new(id, result)));
            });
        }
        Ok(ClientFrame::Call { id, machine, call }) => match call {
            // A machine answers it with the token itself, which only the hub may keep.
            Call::SetupToken { .. } => {
                let refused = "only the hub asks a machine for a Claude token: use Use for boxes on the Upstash Box card".to_owned();
                account.to_client(client, &ToClient::Reply(Reply::new(id, Err(refused))));
            }
            Call::WatchThreads { tabs } => {
                account.watch_threads(client, &machine, tabs);
                account.to_client(client, &ToClient::Reply(Reply::new(id, Ok(Value::Null))));
            }
            Call::UpdateSelf if account.dedicated(&machine) => {
                let status = UpdateStatus::Cannot { reason: "an Upstash box is replaced by the hub".into() };
                account.note_update(&machine, Some(status.clone()));
                account.to_client(client, &ToClient::Reply(Reply::new(id, Ok(serde_json::to_value(&status).unwrap()))));
                let (hub, account) = (hub.clone(), account.clone());
                tokio::spawn(async move { hub.push_machines(&account).await });
            }
            call => {
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
        },
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
        _ => return drop(socket.send(close_no_sign_in()).await),
    };
    let row = match hub.db.machine_connected(&token, &os, &version).await {
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
    let view = MachineView { id: machine.clone(), name: row.name.clone(), os, online: true, last_seen: 0, upstash_box: None, version: version.clone(), update: None, state: None, background: None };
    let _ = out.send(serde_json::to_string(&ToMachine::Hello { machine: view }).unwrap());
    // A machine that reconnects before its old socket timed out goes offline and back, so clients attach again.
    if account.went_offline(&machine, None) {
        hub.push_machines(&account).await;
    }
    account.came_online(&machine, socket_id, row.name, row.dedicated, version.clone(), out);
    hub.push_machines(&account).await;
    tokio::spawn(crate::surface::deliver_notices(hub.clone(), account.clone(), machine.clone()));
    eprintln!("pinned-server: machine {machine} of account {} connected (pinned {version})", account.id);

    let (mut sink, mut stream) = socket.split();
    let mut ping = tokio::time::interval(PING);
    let mut going_away = hub.watch_going_away();
    loop {
        tokio::select! {
            text = outbox.recv() => match text {
                Some(text) => if sink.send(Message::Text(text.into())).await.is_err() { break },
                None => break,
            },
            _ = hub_going_away(&mut going_away) => {
                let _ = sink.send(close_going_away()).await;
                break;
            }
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
        Ok(MachineFrame::Reply(reply)) => {
            if account.answered(machine, socket, reply) {
                let (hub, account) = (hub.clone(), account.clone());
                tokio::spawn(async move { hub.push_machines(&account).await });
            }
        }
        Ok(MachineFrame::Event { event }) => {
            let push = match &event {
                MachineEvent::Update { status } => {
                    account.note_update(machine, status.clone());
                    true
                }
                MachineEvent::Background { .. } => true,
                _ => false,
            };
            let after = match &event {
                MachineEvent::Tabs { tabs } => Some(tabs.clone()),
                _ => None,
            };
            account.machine_event(machine, socket, event);
            if push {
                let (hub, account) = (hub.clone(), account.clone());
                tokio::spawn(async move { hub.push_machines(&account).await });
            }
            if let Some(after) = after {
                let observed = pinned_core::connectors::unix_millis();
                let (hub, account, machine) = (hub.clone(), account.clone(), machine.to_owned());
                tokio::spawn(async move {
                    let Ok(connectors) = hub.connectors(&account).await else { return };
                    let id = account.id;
                    let open = move |work| {
                        let hub = hub.clone();
                        async move { hub.open_placed(id, work).await }
                    };
                    if let Err(e) = connectors.on_tabs(&machine, observed, &after, open).await {
                        eprintln!("pinned-server: could not wake a workflow run: {e}");
                    }
                });
            }
        }
        Ok(MachineFrame::Output { viewer, tab, data }) => account.output(machine, &viewer, tab, data),
        Ok(MachineFrame::Call { id, call }) => {
            let (hub, account, machine) = (hub.clone(), account.clone(), machine.to_owned());
            tokio::spawn(async move {
                // A machine holds a lesser credential than a login: it may only pass on the callbacks
                // of sessions it runs, so a prompt-injected session can't act for another connector or thread,
                // import into an account with no connectors or tastes yet, read the tastes, sync its projects,
                // and serve the global orchestrator and the Capitães, which `relay` checks against this socket,
                // and a thread's workers on other machines, which `remote` stamps with this socket.
                // Its sessions act as the user only through `surface`, which checks, audits and holds
                // each act, and never decide a confirmation.
                let surfaced = match call {
                    HubCall::Act { act } => crate::surface::act(&hub, &account, &machine, act).await,
                    HubCall::Confirmations => hub.hub_call(&account, HubCall::Confirmations).await,
                    HubCall::DecideConfirmation { .. } => Err("only the account's owner decides, in the app".to_owned()),
                    call => return forward(hub, account, machine, socket, id, call).await,
                };
                if let Err(e) = &surfaced {
                    eprintln!("pinned-server: machine {machine}'s agent act failed: {e}");
                }
                account.to_machine(&machine, socket, &ToMachine::Reply(Reply::new(id, surfaced)));
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

/// A machine's hub call other than an agent's act: only the callbacks its sessions may pass on.
async fn forward(hub: Arc<Hub>, account: Arc<Account>, machine: String, socket: u64, id: u64, call: HubCall) {
    if matches!(call, HubCall::Delegate { .. } | HubCall::RemoteTask { .. } | HubCall::RemoteEnded { .. } | HubCall::RemoteAsked { .. } | HubCall::Place { .. }) {
        let result = crate::remote::serve(&hub, &account, &machine, call).await;
        if let Err(e) = &result {
            eprintln!("pinned-server: machine {machine}'s delegate call failed: {e}");
        }
        return account.to_machine(&machine, socket, &ToMachine::Reply(Reply::new(id, result)));
    }
    if matches!(call, HubCall::Relay { .. } | HubCall::Overview | HubCall::ReadCaptain { .. } | HubCall::ReadSession { .. } | HubCall::RepoLook { .. }) {
        let result = crate::relay::serve(&hub, &account, crate::relay::Source::Machine(&machine), call).await;
        if let Err(e) = &result {
            eprintln!("pinned-server: machine {machine}'s global orchestrator call failed: {e}");
        }
        return account.to_machine(&machine, socket, &ToMachine::Reply(Reply::new(id, result)));
    }
    // The account's MCP servers with their secrets, which only its machines get, to load into
    // the sessions they start.
    if matches!(call, HubCall::LoadMcpServers) {
        let result = hub.connectors(&account).await.and_then(|c| serde_json::to_value(c.load_mcp_servers()).map_err(|e| e.to_string()));
        if let Err(e) = &result {
            eprintln!("pinned-server: machine {machine} could not load the account's MCP servers: {e}");
        }
        return account.to_machine(&machine, socket, &ToMachine::Reply(Reply::new(id, result)));
    }
    // A session's Convex tool. Nothing of it is logged: an answer or a refusal may quote the
    // account's data.
    if let HubCall::Convex { tab, repo, ask } = call {
        let result = hub.convex(&account, &machine, &tab, repo.as_deref(), ask).await;
        return account.to_machine(&machine, socket, &ToMachine::Reply(Reply::new(id, result)));
    }
    // A machine archives only its own tabs, found finished by its own check.
    if let HubCall::AutoArchive { tab } = call {
        let result = hub.auto_archive(&account, &machine, &tab).await.and_then(|marks| serde_json::to_value(marks).map_err(|e| e.to_string()));
        return account.to_machine(&machine, socket, &ToMachine::Reply(Reply::new(id, result)));
    }
    if let HubCall::RecordTasks { records } = call {
        let result = hub.record_tasks(&account, &machine, records).await.map(|()| Value::Null);
        if let Err(e) = &result {
            eprintln!("pinned-server: machine {machine}'s task records were not kept: {e}");
        }
        return account.to_machine(&machine, socket, &ToMachine::Reply(Reply::new(id, result)));
    }
    let work = match &call {
        HubCall::Stage { connector, subject, .. } | HubCall::RunSet { connector, subject, .. } => Some((connector.clone(), subject.clone())),
        HubCall::SlackReply { connector, channel, thread, .. }
        | HubCall::LinearCreate { connector, channel, thread, .. }
        | HubCall::LinearTake { connector, channel, thread, .. } => {
            Some((connector.clone(), Subject::SlackThread { channel: channel.clone(), thread: thread.clone() }))
        }
        _ => None,
    };
    let result = match work {
        Some((connector, subject)) if account.working_on(&connector, &subject, |id| *id == machine) => hub.hub_call(&account, call).await,
        Some(_) => Err("this machine runs no session of that workflow on that subject".into()),
        // An app's first sign-in moves its own connectors and tastes in, which only an empty account
        // takes, every machine keeps its projects' files in step with the account's, and a Capitão
        // or the global orchestrator starts with the account's tastes, and a delegator reads how
        // the account's delegated tasks went.
        None if matches!(
            call,
            HubCall::Import { .. }
                | HubCall::ImportTastes { .. }
                | HubCall::GetTastes
                | HubCall::DelegationStats { .. }
                | HubCall::MachineStates
                | HubCall::ShareProject { .. }
                | HubCall::SyncList { .. }
                | HubCall::SyncPush { .. }
                | HubCall::SyncPull { .. }
                | HubCall::SyncMarkExecutable { .. }
        ) =>
        {
            hub.hub_call(&account, call).await
        }
        // A thread opened with Chat to adjust changes the account's tastes, and the hub
        // takes that only while this machine reports such a thread open.
        None if matches!(call, HubCall::SetTaste { .. } | HubCall::ResetTasteField { .. }) => {
            if account.tabs_of(&machine).is_some_and(|tabs| tabs.iter().any(|tab| tab.taste_chat)) {
                hub.hub_call(&account, call).await
            } else {
                Err("only a machine with a Chat to adjust thread open may change the way of working".into())
            }
        }
        None => Err("a machine may only forward a workflow session's callbacks".into()),
    };
    if let Err(e) = &result {
        eprintln!("pinned-server: machine {machine}'s hub call failed: {e}");
    }
    account.to_machine(&machine, socket, &ToMachine::Reply(Reply::new(id, result)));
}

#[cfg(test)]
mod tests {
    use super::{client_address, Limiter, TrustedProxy, ROUTES};
    use axum::http::HeaderMap;
    use std::net::SocketAddr;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        pairs.iter().map(|(name, value)| (axum::http::HeaderName::from_static(name), value.parse().unwrap())).collect()
    }

    #[test]
    fn behind_railway_the_client_is_the_edges_x_real_ip_alone() {
        let peer: SocketAddr = "100.64.0.7:41000".parse().unwrap();
        let railway = |pairs: &[(&'static str, &str)]| client_address(TrustedProxy::Railway, peer, &headers(pairs));
        assert_eq!(railway(&[("x-real-ip", "203.0.113.5")]), "203.0.113.5");
        assert_eq!(railway(&[("x-real-ip", " 203.0.113.5 "), ("x-forwarded-for", "6.6.6.6, 203.0.113.9")]), "203.0.113.5", "X-Forwarded-For is not read");
        assert_eq!(railway(&[("x-real-ip", "2001:db8::1")]), "2001:db8::1");
        assert_eq!(railway(&[("x-real-ip", "::ffff:203.0.113.5")]), "203.0.113.5", "a mapped v4 address is the v4 address");
        for unusable in ["", "unknown", "203.0.113.5, 6.6.6.6", "203.0.113.5:80", "<script>"] {
            assert_eq!(railway(&[("x-real-ip", unusable), ("x-forwarded-for", "6.6.6.6")]), "100.64.0.7", "{unusable:?} leaves the proxy's own address");
        }
        assert_eq!(railway(&[]), "100.64.0.7");
    }

    #[test]
    fn with_the_proxy_off_only_a_loopback_peers_last_forwarded_hop_counts() {
        let from = |peer: &str, pairs: &[(&'static str, &str)]| client_address(TrustedProxy::Off, peer.parse().unwrap(), &headers(pairs));
        let forwarded = [("x-forwarded-for", "6.6.6.6, 203.0.113.9"), ("x-real-ip", "198.51.100.1")];
        assert_eq!(from("127.0.0.1:5000", &forwarded), "203.0.113.9");
        assert_eq!(from("100.64.0.7:5000", &forwarded), "100.64.0.7", "a peer off this host is the address, whatever it forwards");
        assert_eq!(from("10.1.2.3:5000", &forwarded), "10.1.2.3");
        assert_eq!(from("127.0.0.1:5000", &[]), "127.0.0.1");
    }

    #[test]
    fn rotating_keys_do_not_grow_the_limiter_past_its_cap_nor_evict_a_live_one() {
        let mut limiter = Limiter::new(4);
        let now = std::time::Instant::now();
        let guess = |address: u32| [("email:owner@example.invalid".to_owned(), 3), (format!("address:203.0.113.{address}"), 10)];
        assert!(limiter.attempt(&guess(1), now));
        assert!(limiter.attempt(&guess(2), now));
        for address in 3..50 {
            limiter.attempt(&[(format!("address:198.51.100.{address}"), 10)], now);
        }
        assert!(limiter.keys.len() <= 4, "{} keys", limiter.keys.len());
        assert_eq!(limiter.keys["email:owner@example.invalid"].len(), 2, "a full limiter keeps the account's count");
        assert!(!limiter.attempt(&[("address:192.0.2.1".to_owned(), 10)], now), "a full limiter refuses a new key");
        assert!(limiter.attempt(&guess(1), now), "and still takes a request whose keys it holds");
        assert!(!limiter.attempt(&guess(2), now), "whose limit still holds");
        let later = now + super::ATTEMPT_WINDOW;
        assert!(limiter.attempt(&[("address:192.0.2.1".to_owned(), 10)], later), "a window later the old keys are gone");
        assert!(limiter.keys.len() <= 4);
    }

    #[test]
    fn the_router_serves_each_published_route() {
        let src = include_str!("http.rs");
        let mut used = vec![];
        let mut rest = src;
        while let Some(at) = rest.find("route(\"") {
            rest = &rest[at + "route(\"".len()..];
            let end = rest.find('"').expect("route spec");
            used.push(&rest[..end]);
            rest = &rest[end..];
        }
        assert_eq!(used.as_slice(), ROUTES);
    }
}
