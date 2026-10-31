pub mod boxes;
pub mod db;
pub mod http;
pub mod hub;
pub mod self_update;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

pub struct Config {
    pub db: String,
    pub listen: String,
    pub ui: PathBuf,
    /// Where `pinned-server-<target>` binaries for the install script live.
    pub downloads: PathBuf,
    /// The URL machines and browsers reach the hub at; `http://<listen address>` without one.
    pub public_url: Option<String>,
    /// The key the integrations' secrets are sealed with in Postgres, created on first start.
    pub key_file: PathBuf,
    /// Lets anyone who reaches the hub create an account; otherwise only the first one can.
    pub open_signup: bool,
    /// Upstash's Box API; tests pass a fake.
    pub box_api: Arc<dyn boxes::BoxApi>,
    pub box_policy: boxes::Policy,
}

pub struct Running {
    pub hub: Arc<hub::Hub>,
    pub addr: SocketAddr,
    pub server: tokio::task::JoinHandle<()>,
}

/// Connects to Postgres, starts every account's connectors and serves until the task ends.
pub async fn start(config: Config) -> Result<Running, String> {
    let db = db::Db::connect(&config.db, db::Sealer::load(&config.key_file)?).await?;
    let listener = tokio::net::TcpListener::bind(&config.listen).await.map_err(|e| format!("could not listen on {}: {e}", config.listen))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let public_url = config.public_url.unwrap_or_else(|| format!("http://{addr}"));
    let hub = hub::Hub::new(db, public_url, config.box_api, config.box_policy);
    hub.watch_all().await?;
    let app = http::router(hub.clone(), config.ui, config.downloads, config.open_signup).into_make_service_with_connect_info::<SocketAddr>();
    let server = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("pinned-server: the hub stopped: {e}");
        }
    });
    eprintln!("pinned-server: hub listening on {addr} as {}", hub.public_url);
    Ok(Running { hub, addr, server })
}

/// Serves `hub` on `listen` again. The account state already on `hub` stays.
pub async fn serve(hub: Arc<hub::Hub>, listen: SocketAddr, ui: PathBuf, downloads: PathBuf, open_signup: bool) -> Result<tokio::task::JoinHandle<()>, String> {
    let listener = tokio::net::TcpListener::bind(listen).await.map_err(|e| format!("could not listen on {listen}: {e}"))?;
    let app = http::router(hub, ui, downloads, open_signup).into_make_service_with_connect_info::<SocketAddr>();
    Ok(tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("pinned-server: the hub stopped: {e}");
        }
    }))
}
