use pinned_core::account::{self, Credentials, Signup};
use pinned_core::link;
use pinned_core::machine::{Launch, Machine};
use pinned_core::reminders;
use pinned_server::http::TrustedProxy;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Counts live heap bytes for the hub's census (`pinned_server::heap`).
#[global_allocator]
static HEAP: pinned_server::heap::Counting = pinned_server::heap::Counting;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const USAGE: &str = "usage:
  pinned-server hub --db <postgres url> --listen <addr> --ui <dir> --downloads <dir> [--public-url <url>] [--key-file <file>] [--signup first|open|invite] [--learning-db <postgres url>] [--trusted-proxy off|railway] [--max-accounts <n>]
  pinned-server pair <hub url> <code> [--name <name>] [--config-dir <dir>]
  pinned-server login <hub url> <email> [--name <name>] [--config-dir <dir>]   (password on stdin or in PINNED_PASSWORD)
  pinned-server node [--config-dir <dir>]";
const UNWATCHED: (u16, u16) = (120, 32);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // reqwest, the hub link and the statusline's OpenCode Go usage share rustls, built here with
    // no default crypto provider. The statusline runs as a subcommand below.
    let _ = rustls::crypto::ring::default_provider().install_default();
    // Claude Code calls back the binary that started the session, which on a node is this one.
    if let Some(code) = pinned_core::cli::dispatch(&args) {
        std::process::exit(code);
    }
    let result = match args.split_first() {
        Some((command, rest)) => match command.as_str() {
            "hub" => hub(rest),
            "pair" => pair(rest),
            "login" => login(rest),
            "node" => node(rest),
            "--version" | "version" => Ok(println!("pinned-server {VERSION}")),
            _ => Err(USAGE.into()),
        },
        None => Err(USAGE.into()),
    };
    if let Err(e) = result {
        eprintln!("pinned-server: {e}");
        std::process::exit(1);
    }
}

struct Args {
    positional: Vec<String>,
    flags: HashMap<String, String>,
}

impl Args {
    fn parse(args: &[String], known: &[&str], positional: usize) -> Result<Args, String> {
        let (mut parsed, mut rest) = (Args { positional: vec![], flags: HashMap::new() }, args.iter());
        while let Some(arg) = rest.next() {
            match arg.strip_prefix("--") {
                Some(flag) if known.contains(&flag) => {
                    let value = rest.next().ok_or_else(|| format!("--{flag} needs a value\n{USAGE}"))?;
                    parsed.flags.insert(flag.to_owned(), value.clone());
                }
                Some(_) => return Err(format!("unknown option {arg}\n{USAGE}")),
                None => parsed.positional.push(arg.clone()),
            }
        }
        if parsed.positional.len() != positional {
            return Err(USAGE.into());
        }
        Ok(parsed)
    }

    fn required(&self, flag: &str) -> Result<String, String> {
        self.flags.get(flag).cloned().ok_or_else(|| format!("--{flag} is required\n{USAGE}"))
    }

    fn config_dir(&self) -> PathBuf {
        if let Some(dir) = self.flags.get("config-dir") {
            return dir.into();
        }
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"))
            .join("pinned")
    }

    fn name(&self) -> String {
        self.flags.get("name").cloned().unwrap_or_else(account::hostname)
    }
}

fn hub(args: &[String]) -> Result<(), String> {
    let args = Args::parse(args, &["db", "listen", "ui", "downloads", "public-url", "key-file", "signup", "learning-db", "trusted-proxy", "max-accounts"], 0)?;
    let (signup, unknown) = signup_mode(args.flags.get("signup").map(String::as_str));
    if let Some(warning) = unknown {
        eprintln!("pinned-server: {warning}");
    }
    let (trusted_proxy, unknown) = trusted_proxy_flag(args.flags.get("trusted-proxy").map(String::as_str));
    if let Some(warning) = unknown {
        eprintln!("pinned-server: {warning}");
    }
    let max_accounts = args
        .flags
        .get("max-accounts")
        .map(|n| n.parse::<u32>().map_err(|_| format!("--max-accounts takes a number of accounts, not {n}\n{USAGE}")))
        .transpose()?;
    let config = pinned_server::Config {
        db: args.required("db")?,
        listen: args.required("listen")?,
        ui: args.required("ui")?.into(),
        downloads: args.required("downloads")?.into(),
        public_url: args.flags.get("public-url").cloned(),
        key_file: args.flags.get("key-file").map_or_else(|| args.config_dir().join("hub.key"), PathBuf::from),
        signup,
        box_api: std::sync::Arc::new(pinned_server::boxes::Upstash),
        box_policy: pinned_server::boxes::Policy::default(),
        convex_http: std::sync::Arc::new(pinned_core::convex::client::Reqwest),
        learning_db: args.flags.get("learning-db").cloned(),
        access: pinned_server::http::Access { trusted_proxy, max_accounts, hashing: None },
    };
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let pinned_server::Running { hub, server, .. } = pinned_server::start(config).await?;
        let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(|e| e.to_string())?;
        let mut server = server;
        let ended = tokio::select! {
            ended = &mut server => Some(ended),
            _ = terminate.recv() => { eprintln!("pinned-server: SIGTERM, closing every socket before stopping"); None }
            _ = tokio::signal::ctrl_c() => { eprintln!("pinned-server: interrupted, closing every socket before stopping"); None }
        };
        let ended = match ended {
            Some(ended) => ended,
            None => {
                hub.going_away();
                server.await
            }
        };
        // The sockets are closed, but a connectors task may still run and save. Write what has not
        // reached Postgres, and again what was saved meanwhile, until a pass finds nothing waiting
        // or FLUSH_WITHIN passed. A save after that last pass is not written.
        if hub.flush_writes(pinned_server::FLUSH_WITHIN).await > 0 {
            eprintln!("pinned-server: stopped with saves not written after {}s", pinned_server::FLUSH_WITHIN.as_secs());
        }
        ended.map_err(|e| e.to_string())
    })
}

/// The hub's sign up mode from `--signup`. Unset is first. A value that is none of the three is invite,
/// the closed mode that still lets invited people in, with a line saying so: a mistyped Railway variable
/// leaves the hub up and closed.
fn signup_mode(flag: Option<&str>) -> (Signup, Option<String>) {
    match flag {
        None | Some("first") => (Signup::First, None),
        Some("open") => (Signup::Open, None),
        Some("invite") => (Signup::Invite, None),
        Some(other) => (Signup::Invite, Some(format!("--signup takes first, open or invite, not {other:?}; using invite"))),
    }
}

/// The proxy the hub trusts from `--trusted-proxy`. Unset is off. A value that is neither off nor railway
/// is off, with a line saying so: a mistyped Railway variable leaves the hub up and trusting no proxy,
/// where it would otherwise not start.
fn trusted_proxy_flag(flag: Option<&str>) -> (TrustedProxy, Option<String>) {
    match flag.map(str::parse) {
        None => (TrustedProxy::Off, None),
        Some(Ok(proxy)) => (proxy, None),
        Some(Err(_)) => (TrustedProxy::Off, Some(format!("--trusted-proxy takes off or railway, not {:?}; using off", flag.unwrap_or_default()))),
    }
}

fn pair(args: &[String]) -> Result<(), String> {
    let args = Args::parse(args, &["name", "config-dir"], 2)?;
    let name = args.name();
    let creds = account::pair(&args.positional[0], &args.positional[1], &name, &account::os())?;
    creds.save(&args.config_dir())?;
    println!("{name}");
    Ok(())
}

fn login(args: &[String]) -> Result<(), String> {
    let args = Args::parse(args, &["name", "config-dir"], 2)?;
    let (hub, email) = (&args.positional[0], &args.positional[1]);
    let password = match std::env::var("PINNED_PASSWORD") {
        Ok(password) => password,
        Err(_) => {
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).map_err(|e| e.to_string())?;
            line.trim_end_matches(['\r', '\n']).to_owned()
        }
    };
    let (user_token, account) = account::login(hub, email, &password)?;
    let (machine, machine_token) = account::register_machine(hub, &user_token, &args.name(), &account::os())?;
    // The machine links with its own token; the login only added it.
    let creds = Credentials { hub: hub.trim_end_matches('/').to_owned(), email: account.email, user_token: String::new(), machine_id: machine.id, machine_token };
    creds.save(&args.config_dir())?;
    if let Err(e) = account::logout(hub, &user_token) {
        eprintln!("pinned-server: could not end the login that added this machine: {e}");
    }
    println!("{}", machine.name);
    Ok(())
}

fn running_version() -> String {
    match std::env::var(pinned_core::boot::VERSION_ENV) {
        Ok(version) if !version.is_empty() => version,
        _ => VERSION.to_owned(),
    }
}

fn node(args: &[String]) -> Result<(), String> {
    pinned_server::self_update::prove_or_restore();
    let args = Args::parse(args, &["config-dir"], 0)?;
    let dir = args.config_dir();
    let mut creds = Credentials::load(&dir).ok_or_else(|| format!("{} has no account.json: run `pinned-server pair <hub url> <code>` first", dir.display()))?;
    if let Some(login) = creds.forget_login(&dir)? {
        let hub = creds.hub.clone();
        std::thread::spawn(move || match account::logout(&hub, &login) {
            Ok(()) => eprintln!("pinned-server: ended the login an older pinned kept in account.json"),
            Err(e) => eprintln!("pinned-server: could not end the login an older pinned kept in account.json: {e}"),
        });
    }
    let exe = std::env::current_exe().map_err(|e| format!("could not find pinned-server's own binary: {e}"))?;
    let mut launch = Launch::prepare(&exe).map_err(|e| format!("could not prepare claude's settings: {e}"))?;
    // Reminders notify on a desktop, which a node doesn't have, so its sessions get delegation and the project tools.
    launch.reminders = false;
    if let Some(config) = &launch.mcp_config {
        reminders::write_mcp_config(config, &exe, false).map_err(|e| format!("could not write claude's MCP config: {e}"))?;
    }
    let machine = Machine::new(launch, Some(dir.clone()), None, |_| {});
    machine.set_installer(Arc::new(pinned_server::self_update::NodeUpdate::production()));
    let version = running_version();
    let machine_token = creds.machine_token.clone();
    let link = Arc::new(link::spawn(creds, machine.clone(), &version));
    let forward = link.clone();
    machine.listen(move |message| forward.forward(message)).map_err(|e| format!("could not listen for claude's hooks: {e}"))?;
    pinned_core::boot::ready();
    for tab in machine.restore_working() {
        if let Err(e) = machine.start_claude(&tab, UNWATCHED.0, UNWATCHED.1) {
            eprintln!("pinned-server: could not resume {tab}: {e}");
        }
    }

    let in_box = pinned_server::box_exit::in_box();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let refused = runtime.block_on(async {
        let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(|e| e.to_string())?;
        let refused = tokio::select! {
            _ = tokio::signal::ctrl_c() => false,
            _ = terminate.recv() => false,
            _ = pinned_server::self_update::roll_back_if_refused(&machine) => false,
            _ = pinned_server::box_exit::refused_in_box(in_box, link.refused_for()) => true,
        };
        Ok::<_, String>(refused)
    })?;
    if refused {
        // A box the hub ended but could not delete: no session runs on, and no start script relaunches the node.
        pinned_server::box_exit::leave(&dir, &machine_token, || machine.quit(), |line| eprintln!("{line}"));
    } else {
        // The sessions stay saved, so the next start resumes them.
        machine.quit();
    }
    let _ = std::fs::remove_file(&machine.launch().sock);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{signup_mode, trusted_proxy_flag};
    use pinned_core::account::Signup;
    use pinned_server::http::TrustedProxy;

    #[test]
    fn an_unknown_trusted_proxy_starts_the_hub_with_none_and_says_so() {
        assert_eq!(trusted_proxy_flag(None), (TrustedProxy::Off, None));
        assert_eq!(trusted_proxy_flag(Some("off")), (TrustedProxy::Off, None));
        assert_eq!(trusted_proxy_flag(Some("railway")), (TrustedProxy::Railway, None));
        assert_eq!(trusted_proxy_flag(Some("raliway")), (TrustedProxy::Off, Some(r#"--trusted-proxy takes off or railway, not "raliway"; using off"#.into())));
        assert_eq!(trusted_proxy_flag(Some("")), (TrustedProxy::Off, Some(r#"--trusted-proxy takes off or railway, not ""; using off"#.into())));
        assert_eq!(trusted_proxy_flag(Some("Railway")).0, TrustedProxy::Off, "the names are exact");
    }

    #[test]
    fn an_unknown_signup_mode_runs_as_invite_and_says_so() {
        assert_eq!(signup_mode(None), (Signup::First, None));
        assert_eq!(signup_mode(Some("first")), (Signup::First, None));
        assert_eq!(signup_mode(Some("open")), (Signup::Open, None));
        assert_eq!(signup_mode(Some("invite")), (Signup::Invite, None));
        assert_eq!(signup_mode(Some("bogus")), (Signup::Invite, Some(r#"--signup takes first, open or invite, not "bogus"; using invite"#.into())));
        assert_eq!(signup_mode(Some("")), (Signup::Invite, Some(r#"--signup takes first, open or invite, not ""; using invite"#.into())));
        assert_eq!(signup_mode(Some("Open")).0, Signup::Invite, "the names are exact");
    }
}
