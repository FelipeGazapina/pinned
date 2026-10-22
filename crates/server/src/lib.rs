pub mod db;
pub mod http;
pub mod hub;

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
}

pub struct Running {
    pub hub: Arc<hub::Hub>,
    pub addr: SocketAddr,
    pub server: tokio::task::JoinHandle<()>,
}

/// Connects to Postgres, starts every account's connectors and serves until the task ends.
pub async fn start(config: Config) -> Result<Running, String> {
    let db = db::Db::connect(&config.db).await?;
    let listener = tokio::net::TcpListener::bind(&config.listen).await.map_err(|e| format!("could not listen on {}: {e}", config.listen))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let public_url = config.public_url.unwrap_or_else(|| format!("http://{addr}"));
    let hub = hub::Hub::new(db, public_url);
    hub.watch_all().await?;
    let app = http::router(hub.clone(), config.ui, config.downloads).into_make_service_with_connect_info::<SocketAddr>();
    let server = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("pinned-server: the hub stopped: {e}");
        }
    });
    eprintln!("pinned-server: hub listening on {addr} as {}", hub.public_url);
    Ok(Running { hub, addr, server })
}
