pub mod box_exit;
pub mod boxes;
pub mod db;
pub mod http;
pub mod hub;
pub mod mcp;
pub mod oauth;
pub mod relay;
pub mod remote;
pub mod self_update;
pub mod surface;

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
    /// Who may create an account.
    pub signup: pinned_core::account::Signup,
    /// Upstash's Box API; tests pass a fake.
    pub box_api: Arc<dyn boxes::BoxApi>,
    pub box_policy: boxes::Policy,
    /// Convex, which the hub reads for sessions with the keys it keeps; tests replay fixtures.
    pub convex_http: Arc<dyn pinned_core::convex::client::Http>,
    /// A second Postgres for what users share to improve the app; without it the hub keeps none.
    pub learning_db: Option<String>,
    /// Which proxy's client address the hub believes, and how many accounts it takes.
    pub access: http::Access,
}

/// How long the hub gives sockets to close once it is told to stop, before it drops them.
pub const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

pub struct Running {
    pub hub: Arc<hub::Hub>,
    pub addr: SocketAddr,
    pub server: tokio::task::JoinHandle<()>,
}

/// Connects to Postgres, starts every account's connectors and serves until the task ends.
pub async fn start(config: Config) -> Result<Running, String> {
    let db = db::Db::connect(&config.db, db::Sealer::load(&config.key_file)?).await?;
    let learning = match &config.learning_db {
        Some(url) => Some(db::Learning::connect(url).await?),
        None => None,
    };
    let listener = tokio::net::TcpListener::bind(&config.listen).await.map_err(|e| format!("could not listen on {}: {e}", config.listen))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let public_url = config.public_url.unwrap_or_else(|| format!("http://{addr}"));
    let hub = hub::Hub::new(db, public_url, config.signup, config.box_api, config.box_policy, config.convex_http);
    surface::finish_interrupted(&hub).await?;
    // Again once a decision the hub before this one ran can no longer be running.
    let later = hub.clone();
    tokio::spawn(async move {
        tokio::time::sleep(surface::DECIDING_AT_MOST).await;
        if let Err(e) = surface::finish_interrupted(&later).await {
            eprintln!("pinned-server: could not finish the confirmations a stopped hub left: {e}");
        }
    });
    hub.watch_all().await?;
    let app = http::router(hub.clone(), config.ui, config.downloads, learning, config.access).into_make_service_with_connect_info::<SocketAddr>();
    // `going_away` closes every socket with a close frame, which lets the graceful stop finish. A
    // socket that doesn't answer is dropped after the grace period.
    let (mut stopping, mut forced) = (hub.watch_going_away(), hub.watch_going_away());
    let serving = axum::serve(listener, app).with_graceful_shutdown(async move {
        let _ = stopping.wait_for(|away| *away).await;
    });
    let server = tokio::spawn(async move {
        tokio::select! {
            served = serving => if let Err(e) = served {
                eprintln!("pinned-server: the hub stopped: {e}");
            },
            _ = async {
                let _ = forced.wait_for(|away| *away).await;
                tokio::time::sleep(SHUTDOWN_GRACE).await;
            } => eprintln!("pinned-server: dropped the connections that did not close within {}s", SHUTDOWN_GRACE.as_secs()),
        }
    });
    eprintln!("pinned-server: hub listening on {addr} as {}", hub.public_url);
    Ok(Running { hub, addr, server })
}
