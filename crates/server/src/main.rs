use pinned_core::account::{self, Credentials, Signup};
use pinned_core::link;
use pinned_core::space::Space;
use pinned_core::machine::{Launch, Machine};
use pinned_core::reminders;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const USAGE: &str = "usage:
  pinned-server hub --db <postgres url> --listen <addr> --ui <dir> --downloads <dir> [--public-url <url>] [--key-file <file>] [--signup first|open|invite] [--learning-db <postgres url>]
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
    let args = Args::parse(args, &["db", "listen", "ui", "downloads", "public-url", "key-file", "signup", "learning-db"], 0)?;
    let signup = match args.flags.get("signup").map(String::as_str) {
        None | Some("first") => Signup::First,
        Some("open") => Signup::Open,
        Some("invite") => Signup::Invite,
        Some(other) => return Err(format!("--signup takes first, open or invite, not {other}\n{USAGE}")),
    };
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
    };
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let pinned_server::Running { hub, server, .. } = pinned_server::start(config).await?;
        let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(|e| e.to_string())?;
        let mut server = server;
        tokio::select! {
            ended = &mut server => return ended.map_err(|e| e.to_string()),
            _ = terminate.recv() => eprintln!("pinned-server: SIGTERM, closing every socket before stopping"),
            _ = tokio::signal::ctrl_c() => eprintln!("pinned-server: interrupted, closing every socket before stopping"),
        }
        hub.going_away();
        server.await.map_err(|e| e.to_string())
    })
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
    let machine = Machine::new(launch, Space::at(&dir), None, |_| {});
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
