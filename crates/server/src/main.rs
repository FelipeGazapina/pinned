use pinned_core::account::{self, Credentials};
use pinned_core::link;
use pinned_core::machine::{Launch, Machine};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const USAGE: &str = "usage:
  pinned-server hub --db <postgres url> --listen <addr> --ui <dir> --downloads <dir> [--public-url <url>] [--key-file <file>] [--signup first|open]
  pinned-server pair <hub url> <code> [--name <name>] [--config-dir <dir>]
  pinned-server login <hub url> <email> [--name <name>] [--config-dir <dir>]   (password on stdin or in PINNED_PASSWORD)
  pinned-server node [--config-dir <dir>]";
const UNWATCHED: (u16, u16) = (120, 32);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Claude Code calls back the binary that started the session, which on a node is this one.
    if let Some(code) = pinned_core::cli::dispatch(&args) {
        std::process::exit(code);
    }
    // reqwest and the hub link use rustls, built here with no default crypto provider.
    let _ = rustls::crypto::ring::default_provider().install_default();
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
    let args = Args::parse(args, &["db", "listen", "ui", "downloads", "public-url", "key-file", "signup"], 0)?;
    let open_signup = match args.flags.get("signup").map(String::as_str) {
        None | Some("first") => false,
        Some("open") => true,
        Some(other) => return Err(format!("--signup takes first or open, not {other}\n{USAGE}")),
    };
    let config = pinned_server::Config {
        db: args.required("db")?,
        listen: args.required("listen")?,
        ui: args.required("ui")?.into(),
        downloads: args.required("downloads")?.into(),
        public_url: args.flags.get("public-url").cloned(),
        key_file: args.flags.get("key-file").map_or_else(|| args.config_dir().join("hub.key"), PathBuf::from),
        open_signup,
    };
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let running = pinned_server::start(config).await?;
        running.server.await.map_err(|e| e.to_string())
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
    let creds = Credentials { hub: hub.trim_end_matches('/').to_owned(), email: account.email, user_token, machine_id: machine.id, machine_token };
    creds.save(&args.config_dir())?;
    println!("{}", machine.name);
    Ok(())
}

fn node(args: &[String]) -> Result<(), String> {
    let args = Args::parse(args, &["config-dir"], 0)?;
    let dir = args.config_dir();
    let creds = Credentials::load(&dir).ok_or_else(|| format!("{} has no account.json: run `pinned-server pair <hub url> <code>` first", dir.display()))?;
    let exe = std::env::current_exe().map_err(|e| format!("could not find pinned-server's own binary: {e}"))?;
    let mut launch = Launch::prepare(&exe).map_err(|e| format!("could not prepare claude's settings: {e}"))?;
    // Reminders notify on a desktop, which a node doesn't have.
    launch.mcp_config = None;
    let machine = Machine::new(launch, Some(dir), None, |_| {});
    let link = Arc::new(link::spawn(creds, machine.clone(), VERSION));
    let forward = link.clone();
    machine.listen(move |message| forward.forward(message)).map_err(|e| format!("could not listen for claude's hooks: {e}"))?;
    for tab in machine.restore_tabs() {
        if let Err(e) = machine.start_claude(&tab, UNWATCHED.0, UNWATCHED.1) {
            eprintln!("pinned-server: could not resume {tab}: {e}");
        }
    }

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(|e| e.to_string())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
        Ok::<_, String>(())
    })?;
    // The sessions stay saved, so the next start resumes them.
    machine.quit();
    let _ = std::fs::remove_file(&machine.launch().sock);
    Ok(())
}
