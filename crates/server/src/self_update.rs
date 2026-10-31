use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use pinned_core::machine::{Machine, SelfUpdate};
use pinned_core::wire::UpdateStatus;
use sha2::{Digest, Sha256};
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub const RELEASE_BASE: &str = "https://github.com/FelipeGazapina/pinned/releases/latest/download";

/// The minisign public key in `tauri.conf.json`, base64 of the public key file.
pub const RELEASE_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEIwQ0ZBQUFCNDgxNzI3NTIKUldSU0p4ZElxNnJQc0hENytBanJ6K25Xd0pOVmdEWlpwZzJRak9VemN3Rk1TcmNFUWdtOWNVOGoK";

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const CONNECT_WAIT: Duration = Duration::from_secs(60);
const ALLOW_LEGACY_SIGNATURES: bool = false;

pub struct NodeUpdate {
    base: String,
}

impl NodeUpdate {
    pub fn production() -> Self {
        Self { base: RELEASE_BASE.to_owned() }
    }

    pub fn at(base: impl Into<String>) -> Self {
        Self { base: base.into() }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub program: PathBuf,
    pub args: Vec<std::ffi::OsString>,
    pub cwd: PathBuf,
}

impl SelfUpdate for NodeUpdate {
    fn install(&self, machine: &Machine) -> UpdateStatus {
        let triple = match release_target(std::env::consts::OS, std::env::consts::ARCH) {
            Ok(triple) => triple,
            Err(status) => {
                machine.report(status.clone());
                return status;
            }
        };
        let dest = match std::env::current_exe() {
            Ok(dest) => dest,
            Err(error) => {
                let status = UpdateStatus::Failed { to: String::new(), reason: format!("could not find this binary: {error}") };
                machine.report(status.clone());
                return status;
            }
        };
        let (status, plan) = self.prepare(triple, &dest, RELEASE_PUBKEY, &|status| {
            machine.report(status);
        });
        let UpdateStatus::Restarting { ref to } = status else { return status };
        let Some(plan) = plan else { return status };
        let to = to.clone();
        let restarting = machine.arc();
        let reporting = restarting.clone();
        let dest = dest.clone();
        restarting.schedule_restart(Box::new(move || {
            match watch_new_binary(&plan, &dest, CONNECT_WAIT) {
                Ok(()) => std::process::exit(0),
                Err(reason) => {
                    reporting.report(UpdateStatus::Failed { to, reason });
                }
            }
        }));
        let status = status.clone();
        machine.report(status.clone());
        status
    }
}

pub(crate) fn release_target(os: &str, arch: &str) -> Result<&'static str, UpdateStatus> {
    match (os, arch) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-musl"),
        _ => Err(UpdateStatus::Cannot { reason: format!("no release binary for {os} {arch}") }),
    }
}

impl NodeUpdate {
    pub(crate) fn prepare(&self, triple: &str, dest: &Path, pubkey: &str, report: &dyn Fn(UpdateStatus)) -> (UpdateStatus, Option<Plan>) {
        let base = self.base.trim_end_matches('/');
        let latest = match download(&format!("{base}/latest.json")) {
            Ok(body) => body,
            Err(reason) => return failed(report, "", reason),
        };
        let sums = match download(&format!("{base}/SHA256SUMS")) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(reason) => return failed(report, "", reason),
        };
        let signature = match download(&format!("{base}/SHA256SUMS.minisig")) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(reason) => return failed(report, "", reason),
        };
        if let Err(reason) = verify(pubkey, &sums, &signature, "latest.json", &latest) {
            return failed(report, "", reason);
        }
        let version = match version_of(&latest) {
            Ok(version) => version,
            Err(reason) => return failed(report, "", reason),
        };
        let published = match release_number(&version) {
            Ok(published) => published,
            Err(reason) => return failed(report, "", reason),
        };
        let current = env!("CARGO_PKG_VERSION");
        let running = match release_number(current) {
            Ok(running) => running,
            Err(reason) => return failed(report, &version, reason),
        };
        if published == running {
            let status = UpdateStatus::UpToDate { version: current.to_owned() };
            report(status.clone());
            return (status, None);
        }
        if published < running {
            return failed(report, &version, format!("the published release {version} is older than {current}"));
        }
        report(UpdateStatus::Updating { to: version.clone() });
        let name = format!("pinned-server-{triple}");
        let bytes = match download(&format!("{base}/{name}")) {
            Ok(bytes) => bytes,
            Err(reason) => return failed(report, &version, reason),
        };
        if let Err(reason) = verify(pubkey, &sums, &signature, &name, &bytes) {
            return failed(report, &version, reason);
        }
        match stage_planned(dest, &bytes, plan_for(dest)) {
            Ok(plan) => (UpdateStatus::Restarting { to: version }, Some(plan)),
            Err(reason) => failed(report, &version, reason),
        }
    }
}

pub(crate) fn verify(pubkey: &str, sums: &str, signature: &str, file_name: &str, bytes: &[u8]) -> Result<(), String> {
    let key = public_key(pubkey)?;
    let signature = Signature::decode(signature).map_err(|error| format!("could not read the release signature: {error}"))?;
    key.verify(sums.as_bytes(), &signature, ALLOW_LEGACY_SIGNATURES).map_err(|_| "the release signature does not match".to_owned())?;
    let want = sha256(bytes);
    match listed(sums, file_name) {
        Some(hash) if hash.eq_ignore_ascii_case(&want) => Ok(()),
        Some(_) => Err("the release checksum does not match".into()),
        None => Err(format!("SHA256SUMS has no {file_name}")),
    }
}

#[cfg(test)]
pub(crate) fn apply(dest: &Path, file_name: &str, bytes: &[u8], sums: &str, signature: &str, pubkey: &str) -> Result<(), String> {
    verify(pubkey, sums, signature, file_name, bytes)?;
    stage(dest, bytes)
}

pub(crate) fn stage(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = dest.file_name().ok_or_else(|| format!("{} has no name", dest.display()))?;
    let name = name.to_string_lossy();
    let bak = dest.with_file_name(format!("{name}.bak"));
    let tmp = dest.with_file_name(format!("{name}.tmp"));
    std::fs::copy(dest, &bak).map_err(|error| format!("could not back up {}: {error}", dest.display()))?;
    let staged = (|| {
        std::fs::write(&tmp, bytes).map_err(|error| format!("could not write {}: {error}", tmp.display()))?;
        std::fs::set_permissions(&tmp, Permissions::from_mode(0o755)).map_err(|error| format!("could not mark {} executable: {error}", tmp.display()))?;
        std::fs::rename(&tmp, dest).map_err(|error| format!("could not replace {}: {error}", dest.display()))
    })();
    if staged.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    staged
}

fn failed(report: &dyn Fn(UpdateStatus), to: &str, reason: String) -> (UpdateStatus, Option<Plan>) {
    let status = UpdateStatus::Failed { to: to.to_owned(), reason };
    report(status.clone());
    (status, None)
}

fn version_of(body: &[u8]) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_slice(body).map_err(|error| format!("could not read latest.json: {error}"))?;
    value["version"].as_str().map(str::to_owned).ok_or_else(|| "latest.json has no version".into())
}

pub(crate) fn release_number(raw: &str) -> Result<semver::Version, String> {
    let trimmed = raw.trim();
    let bare = trimmed.strip_prefix('v').unwrap_or(trimmed);
    semver::Version::parse(bare).map_err(|error| format!("latest.json version is not a release number: {error}"))
}

pub(crate) fn same_release(left: &str, right: &str) -> bool {
    match (release_number(left), release_number(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => {
            let left = left.trim();
            let right = right.trim();
            left.strip_prefix('v').unwrap_or(left) == right.strip_prefix('v').unwrap_or(right)
        }
    }
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    download_within(url, DOWNLOAD_TIMEOUT)
}

fn download_within(url: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let client = reqwest::blocking::Client::builder().timeout(timeout).build().map_err(|error| format!("could not download {url}: {error}"))?;
    let response = client.get(url).send().map_err(|error| download_error(url, error))?;
    if !response.status().is_success() {
        return Err(format!("could not download {url}: {}", response.status()));
    }
    response.bytes().map(|bytes| bytes.to_vec()).map_err(|error| download_error(url, error))
}

fn download_error(url: &str, error: reqwest::Error) -> String {
    if error.is_timeout() {
        format!("could not download {url}: timed out")
    } else {
        format!("could not download {url}: {error}")
    }
}

fn public_key(pubkey: &str) -> Result<PublicKey, String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(pubkey).map_err(|error| format!("could not read the release public key: {error}"))?;
    let text = String::from_utf8(bytes).map_err(|error| format!("could not read the release public key: {error}"))?;
    PublicKey::decode(&text).map_err(|error| format!("could not read the release public key: {error}"))
}

fn listed<'a>(sums: &'a str, file_name: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?;
        let name = name.strip_prefix('*').unwrap_or(name);
        (name == file_name).then_some(hash)
    })
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn stage_planned(dest: &Path, bytes: &[u8], plan: Result<Plan, String>) -> Result<Plan, String> {
    let plan = plan?;
    stage(dest, bytes)?;
    Ok(plan)
}

fn ready_path(dest: &Path) -> PathBuf {
    let name = dest.file_name().unwrap_or_default().to_string_lossy();
    dest.with_file_name(format!("{name}.ready"))
}

pub(crate) fn watch_new_binary(plan: &Plan, dest: &Path, wait: Duration) -> Result<(), String> {
    let ready = ready_path(dest);
    let _ = std::fs::remove_file(&ready);
    let mut child = Command::new(&plan.program)
        .args(&plan.args)
        .current_dir(&plan.cwd)
        .env(pinned_core::link::POST_UPDATE_ENV, "1")
        .env(pinned_core::link::UPDATE_READY_ENV, &ready)
        .spawn()
        .map_err(|error| format!("could not start {}: {error}", plan.program.display()))?;
    let started = std::time::Instant::now();
    loop {
        if ready.is_file() {
            let _ = child.try_wait();
            return Ok(());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if ready.is_file() {
                    return Ok(());
                }
                restore(dest)?;
                return Err(format!("the new binary exited before it connected ({status})"));
            }
            Ok(None) if started.elapsed() >= wait => {
                let _ = child.kill();
                let _ = child.wait();
                if ready.is_file() {
                    return Ok(());
                }
                restore(dest)?;
                return Err(format!("the new binary did not connect within {}s", wait.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                restore(dest)?;
                return Err(format!("could not watch {}: {error}", plan.program.display()));
            }
        }
    }
}

fn plan_for(dest: &Path) -> Result<Plan, String> {
    Ok(Plan {
        program: dest.to_path_buf(),
        args: std::env::args_os().skip(1).collect(),
        cwd: std::env::current_dir().map_err(|error| format!("could not read the working directory: {error}"))?,
    })
}

fn restore(dest: &Path) -> Result<(), String> {
    let name = dest.file_name().unwrap_or_default().to_string_lossy();
    let bak = dest.with_file_name(format!("{name}.bak"));
    std::fs::rename(&bak, dest).map_err(|error| format!("could not restore {}: {error}", dest.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use std::io::{Cursor, Read, Write};
    use std::sync::Mutex;

    fn sign(sums: &str) -> (String, String) {
        let minisign::KeyPair { pk, sk } = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let signature = minisign::sign(Some(&pk), &sk, Cursor::new(sums.as_bytes()), None, None).unwrap().into_string();
        let pubkey = base64::engine::general_purpose::STANDARD.encode(pk.to_box().unwrap().into_string());
        (pubkey, signature)
    }

    fn sums_for(name: &str, bytes: &[u8]) -> String {
        format!("{}  {name}\n", sha256(bytes))
    }

    #[test]
    fn the_release_pubkey_is_the_updater_key() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../../../tauri.conf.json")).unwrap();
        assert_eq!(conf["plugins"]["updater"]["pubkey"], RELEASE_PUBKEY);
    }

    #[test]
    fn a_linux_musl_build_is_the_release_this_node_can_install() {
        assert_eq!(release_target("linux", "x86_64").unwrap(), "x86_64-unknown-linux-musl");
        assert_eq!(release_target("linux", "aarch64").unwrap(), "aarch64-unknown-linux-musl");
        assert_eq!(release_target("macos", "aarch64"), Err(UpdateStatus::Cannot { reason: "no release binary for macos aarch64".into() }));
    }

    #[test]
    fn a_signature_over_the_checksums_accepts_only_those_bytes() {
        let name = "pinned-server-x86_64-unknown-linux-musl";
        let bytes = b"new-binary";
        let sums = sums_for(name, bytes);
        let (pubkey, signature) = sign(&sums);
        assert_eq!(verify(&pubkey, &sums, &signature, name, bytes), Ok(()));
        assert_eq!(verify(&pubkey, "unrelated\n", &signature, name, bytes), Err("the release signature does not match".into()));

        let wrong = sums_for(name, b"other-bytes");
        let (pubkey, signature) = sign(&wrong);
        assert_eq!(verify(&pubkey, &wrong, &signature, name, bytes), Err("the release checksum does not match".into()));
    }

    #[test]
    fn a_bad_checksum_leaves_the_binary_and_a_good_one_keeps_the_old_bytes_beside_it() {
        let dir = std::env::temp_dir().join(format!("pinned-stage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("pinned-server");
        std::fs::write(&dest, b"old-binary").unwrap();
        let name = "pinned-server-x86_64-unknown-linux-musl";
        let bad = sums_for(name, b"other-bytes");
        let (pubkey, signature) = sign(&bad);
        assert_eq!(apply(&dest, name, b"new-binary", &bad, &signature, &pubkey).unwrap_err(), "the release checksum does not match");
        assert_eq!(std::fs::read(&dest).unwrap(), b"old-binary");

        let good_bytes = b"new-binary";
        let good = sums_for(name, good_bytes);
        let (pubkey, signature) = sign(&good);
        apply(&dest, name, good_bytes, &good, &signature, &pubkey).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new-binary");
        assert_eq!(std::fs::read(dir.join("pinned-server.bak")).unwrap(), b"old-binary");
        let mode = std::fs::metadata(&dest).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn serve(files: HashMap<String, Vec<u8>>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(mut socket) = incoming else { continue };
                let _ = socket.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut buf = [0; 8192];
                let Ok(n) = socket.read(&mut buf) else { continue };
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let Some(body) = files.get(path) else {
                    let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                };
                let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = socket.write_all(header.as_bytes());
                let _ = socket.write_all(body);
            }
        });
        format!("http://{address}")
    }

    fn release(label: &str, version: &str, asset: Option<&[u8]>, corrupt: bool) -> (String, String, std::path::PathBuf) {
        release_at(label, version, asset, corrupt, true)
    }

    fn release_at(label: &str, version: &str, asset: Option<&[u8]>, corrupt: bool, list_latest: bool) -> (String, String, std::path::PathBuf) {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("pinned-node-update-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("pinned-server");
        std::fs::write(&dest, b"old-binary").unwrap();
        let name = "pinned-server-x86_64-unknown-linux-musl";
        let latest = json!({ "version": version }).to_string().into_bytes();
        let mut files = HashMap::new();
        files.insert("/latest.json".into(), latest.clone());
        let mut entries: Vec<(&str, &[u8])> = Vec::new();
        if list_latest {
            entries.push(("latest.json", latest.as_slice()));
        }
        if let Some(asset) = asset {
            let listed = if corrupt { b"other-bytes".as_slice() } else { asset };
            entries.push((name, listed));
            files.insert(format!("/{name}"), asset.to_vec());
        }
        if entries.is_empty() {
            return (serve(files), String::new(), dest);
        }
        let sums: String = entries.iter().map(|(entry, bytes)| sums_for(entry, bytes)).collect();
        let (pubkey, signature) = sign(&sums);
        files.insert("/SHA256SUMS".into(), sums.into_bytes());
        files.insert("/SHA256SUMS.minisig".into(), signature.into_bytes());
        (serve(files), pubkey, dest)
    }

    #[test]
    fn a_good_release_replaces_the_binary_and_plans_the_same_restart() {
        let asset = b"new-binary";
        let (base, pubkey, dest) = release("good", "9.9.9", Some(asset), false);
        let reports = Mutex::new(Vec::new());
        let (status, plan) = NodeUpdate::at(&base).prepare("x86_64-unknown-linux-musl", &dest, &pubkey, &|status| {
            reports.lock().unwrap().push(status);
        });
        assert_eq!(reports.lock().unwrap().as_slice(), &[UpdateStatus::Updating { to: "9.9.9".into() }]);
        assert_eq!(status, UpdateStatus::Restarting { to: "9.9.9".into() });
        let plan = plan.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), asset);
        assert_eq!(plan.program, dest);
        assert_eq!(plan.args, std::env::args_os().skip(1).collect::<Vec<_>>());
        assert_eq!(plan.cwd, std::env::current_dir().unwrap());
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    #[test]
    fn a_corrupt_checksum_keeps_the_old_binary_and_plans_no_restart() {
        let (base, pubkey, dest) = release("corrupt", "9.9.9", Some(b"new-binary"), true);
        let (status, plan) = NodeUpdate::at(&base).prepare("x86_64-unknown-linux-musl", &dest, &pubkey, &|_| {});
        assert_eq!(status, UpdateStatus::Failed { to: "9.9.9".into(), reason: "the release checksum does not match".into() });
        assert_eq!(plan, None);
        assert_eq!(std::fs::read(&dest).unwrap(), b"old-binary");
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    #[test]
    fn a_checksum_line_may_mark_the_name_as_binary() {
        assert_eq!(listed("abc  pinned-server\n", "pinned-server"), Some("abc"));
        assert_eq!(listed("abc *pinned-server\n", "pinned-server"), Some("abc"));
    }

    #[test]
    fn a_release_whose_version_is_not_in_the_signed_checksums_is_not_installed() {
        let (base, pubkey, dest) = release_at("unlisted", "9.9.9", Some(b"new-binary"), false, false);
        let (status, plan) = NodeUpdate::at(&base).prepare("x86_64-unknown-linux-musl", &dest, &pubkey, &|_| {});
        assert_eq!(status, UpdateStatus::Failed { to: String::new(), reason: "SHA256SUMS has no latest.json".into() });
        assert_eq!(plan, None);
        assert_eq!(std::fs::read(&dest).unwrap(), b"old-binary");
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    #[test]
    fn an_older_release_does_not_replace_the_binary() {
        let (base, pubkey, dest) = release("older", "0.0.1", Some(b"new-binary"), false);
        let (status, plan) = NodeUpdate::at(&base).prepare("x86_64-unknown-linux-musl", &dest, &pubkey, &|_| {});
        assert_eq!(
            status,
            UpdateStatus::Failed {
                to: "0.0.1".into(),
                reason: format!("the published release 0.0.1 is older than {}", env!("CARGO_PKG_VERSION")),
            },
        );
        assert_eq!(plan, None);
        assert_eq!(std::fs::read(&dest).unwrap(), b"old-binary");
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    #[test]
    fn a_release_tagged_with_v_for_this_version_does_not_replace_the_binary() {
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        let (base, pubkey, dest) = release("vcurrent", &version, None, false);
        let (status, plan) = NodeUpdate::at(&base).prepare("x86_64-unknown-linux-musl", &dest, &pubkey, &|_| {});
        assert_eq!(status, UpdateStatus::UpToDate { version: env!("CARGO_PKG_VERSION").into() });
        assert_eq!(plan, None);
        assert_eq!(std::fs::read(&dest).unwrap(), b"old-binary");
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    #[test]
    fn a_plan_that_cannot_be_made_leaves_the_binary() {
        let dir = std::env::temp_dir().join(format!("pinned-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("pinned-server");
        std::fs::write(&dest, b"old-binary").unwrap();
        let err = stage_planned(&dest, b"new-binary", Err("could not read the working directory".into())).unwrap_err();
        assert_eq!(err, "could not read the working directory");
        assert_eq!(std::fs::read(&dest).unwrap(), b"old-binary");
        assert!(!dir.join("pinned-server.bak").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_binary_that_exits_immediately_puts_the_old_one_back() {
        let dir = std::env::temp_dir().join(format!("pinned-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("pinned-server");
        std::fs::write(&dest, b"new-binary").unwrap();
        std::fs::write(dir.join("pinned-server.bak"), b"old-binary").unwrap();
        let plan = Plan { program: "/bin/sh".into(), args: vec!["-c".into(), "exit 7".into()], cwd: dir.clone() };
        let err = watch_new_binary(&plan, &dest, std::time::Duration::from_secs(2)).unwrap_err();
        assert!(err.contains("exited"), "{err}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"old-binary");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_binary_that_signals_ready_stays_in_place() {
        let dir = std::env::temp_dir().join(format!("pinned-ready-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("pinned-server");
        std::fs::write(&dest, b"new-binary").unwrap();
        std::fs::write(dir.join("pinned-server.bak"), b"old-binary").unwrap();
        let script = format!("touch \"${}\"", pinned_core::link::UPDATE_READY_ENV);
        let plan = Plan { program: "/bin/sh".into(), args: vec!["-c".into(), script.into()], cwd: dir.clone() };
        watch_new_binary(&plan, &dest, std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new-binary");
        assert_eq!(std::fs::read(dir.join("pinned-server.bak")).unwrap(), b"old-binary");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_download_that_stalls_ends_before_the_body_arrives() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(mut socket) = incoming else { continue };
                let mut buf = [0; 1024];
                let _ = socket.read(&mut buf);
                std::thread::sleep(std::time::Duration::from_secs(10));
            }
        });
        let started = std::time::Instant::now();
        let err = download_within(&format!("http://{address}/pinned-server"), std::time::Duration::from_millis(200)).unwrap_err();
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "{err} after {:?}", started.elapsed());
        assert!(err.contains("timed out"), "{err}");
    }

    #[test]
    fn a_release_of_this_version_does_not_replace_the_binary() {
        let (base, pubkey, dest) = release("current", env!("CARGO_PKG_VERSION"), None, false);
        let (status, plan) = NodeUpdate::at(&base).prepare("x86_64-unknown-linux-musl", &dest, &pubkey, &|_| {});
        assert_eq!(status, UpdateStatus::UpToDate { version: env!("CARGO_PKG_VERSION").into() });
        assert_eq!(plan, None);
        assert_eq!(std::fs::read(&dest).unwrap(), b"old-binary");
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }
}
