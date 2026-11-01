use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use pinned_core::machine::{Machine, SelfUpdate};
use pinned_core::wire::UpdateStatus;
use sha2::{Digest, Sha256};
use std::fs::Permissions;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::io::Write;
use std::process::Command;
use std::time::Duration;

pub const RELEASE_BASE: &str = "https://github.com/FelipeGazapina/pinned/releases/latest/download";

/// The minisign public key in `tauri.conf.json`, base64 of the public key file.
pub const RELEASE_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEIwQ0ZBQUFCNDgxNzI3NTIKUldSU0p4ZElxNnJQc0hENytBanJ6K25Xd0pOVmdEWlpwZzJRak9VemN3Rk1TcmNFUWdtOWNVOGoK";

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const DOWNLOAD_CAP: u64 = 128 * 1024 * 1024;
const READY_WAIT: Duration = Duration::from_secs(60);
const ALLOW_LEGACY_SIGNATURES: bool = false;

pub struct NodeUpdate {
    base: String,
}

impl NodeUpdate {
    pub fn production() -> Self {
        Self { base: release_base() }
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
    pub proof: String,
}

impl SelfUpdate for NodeUpdate {
    fn install(&self, machine: &Machine) -> UpdateStatus {
        let status = UpdateStatus::Updating { to: String::new() };
        machine.report(status.clone());
        machine.keep_updating();
        let base = self.base.clone();
        let machine = machine.arc();
        std::thread::spawn(move || {
            let finished = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| NodeUpdate { base }.finish(&machine)));
            if finished.is_err() {
                machine.report(UpdateStatus::Failed { to: String::new(), reason: "the update stopped unexpectedly".into() });
            }
        });
        status
    }
}

impl NodeUpdate {
    fn finish(&self, machine: &Machine) {
        let triple = match install_triple() {
            Ok(triple) => triple,
            Err(status) => {
                machine.report(status);
                return;
            }
        };
        let dest = match std::env::current_exe() {
            Ok(dest) => dest,
            Err(error) => {
                machine.report(UpdateStatus::Failed { to: String::new(), reason: format!("could not find this binary: {error}") });
                return;
            }
        };
        let (status, plan) = self.prepare(&triple, &dest, &release_pubkey(), &|status| {
            machine.report(status);
        });
        let UpdateStatus::Restarting { to } = status else { return };
        let Some(plan) = plan else { return };
        if let Err(reason) = hand_over(machine, &plan, &dest, &to) {
            machine.report(UpdateStatus::Failed { to, reason });
        }
    }
}

fn release_base() -> String {
    std::env::var("PINNED_RELEASE_BASE").unwrap_or_else(|_| RELEASE_BASE.to_owned())
}

fn release_pubkey() -> String {
    std::env::var("PINNED_RELEASE_PUBKEY").unwrap_or_else(|_| RELEASE_PUBKEY.to_owned())
}

fn install_triple() -> Result<String, UpdateStatus> {
    if let Ok(triple) = std::env::var("PINNED_RELEASE_TRIPLE") {
        if !triple.is_empty() {
            return Ok(triple);
        }
    }
    release_target(std::env::consts::OS, std::env::consts::ARCH).map(str::to_owned)
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
        if published <= running {
            let status = UpdateStatus::UpToDate { version: current.to_owned() };
            report(status.clone());
            return (status, None);
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
        match stage_planned(dest, &bytes, working_dir()) {
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
pub(crate) fn apply(dest: &Path, file_name: &str, bytes: &[u8], sums: &str, signature: &str, pubkey: &str) -> Result<String, String> {
    verify(pubkey, sums, signature, file_name, bytes)?;
    stage(dest, bytes)
}

pub(crate) fn stage(dest: &Path, bytes: &[u8]) -> Result<String, String> {
    let name = dest.file_name().ok_or_else(|| format!("{} has no name", dest.display()))?;
    let name = name.to_string_lossy();
    let bak = dest.with_file_name(format!("{name}.bak"));
    let tmp = dest.with_file_name(format!("{name}.tmp"));
    let marker = unproven_path(dest);
    // A crash after the bytes are replaced must not let the next stage copy them over the backup.
    if marker.is_file() {
        if !bak.is_file() {
            return Err(format!("could not back up {}: the previous bytes are missing", dest.display()));
        }
        restore(dest)?;
    }
    copy_durable(dest, &bak)?;
    let proof = sha256(bytes);
    write_durable(&marker, format!("{proof}\n").as_bytes()).map_err(|error| format!("could not mark {} unproven: {error}", dest.display()))?;
    let staged = (|| {
        std::fs::write(&tmp, bytes).map_err(|error| format!("could not write {}: {error}", tmp.display()))?;
        std::fs::set_permissions(&tmp, Permissions::from_mode(0o755)).map_err(|error| format!("could not mark {} executable: {error}", tmp.display()))?;
        std::fs::rename(&tmp, dest).map_err(|error| format!("could not replace {}: {error}", dest.display()))
    })();
    if let Err(error) = staged {
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(&marker);
        return Err(error);
    }
    Ok(proof)
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
    read_capped(response, url, DOWNLOAD_CAP)
}

fn read_capped(mut reader: impl Read, url: &str, cap: u64) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    let mut buf = [0; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|error| format!("could not download {url}: {error}"))?;
        if n == 0 {
            return Ok(body);
        }
        if body.len() as u64 + n as u64 > cap {
            return Err(format!("could not download {url}: larger than {cap} bytes"));
        }
        body.extend_from_slice(&buf[..n]);
    }
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

fn stage_planned(dest: &Path, bytes: &[u8], cwd: Result<PathBuf, String>) -> Result<Plan, String> {
    let cwd = cwd?;
    let proof = stage(dest, bytes)?;
    Ok(Plan { program: dest.to_path_buf(), args: std::env::args_os().skip(1).collect(), cwd, proof })
}

fn working_dir() -> Result<PathBuf, String> {
    std::env::current_dir().map_err(|error| format!("could not read the working directory: {error}"))
}

fn unproven_path(dest: &Path) -> PathBuf {
    let name = dest.file_name().unwrap_or_default().to_string_lossy();
    dest.with_file_name(format!("{name}.unproven"))
}

fn restore(dest: &Path) -> Result<(), String> {
    let name = dest.file_name().unwrap_or_default().to_string_lossy();
    let bak = dest.with_file_name(format!("{name}.bak"));
    if bak.is_file() {
        std::fs::rename(&bak, dest).map_err(|error| format!("could not restore {}: {error}", dest.display()))?;
    }
    let _ = std::fs::remove_file(unproven_path(dest));
    Ok(())
}

fn copy_durable(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to).map_err(|error| format!("could not back up {}: {error}", from.display()))?;
    if let Ok(file) = std::fs::File::open(to) {
        let _ = file.sync_all();
    }
    Ok(())
}

fn write_durable(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!("{name}.writing"));
    {
        let mut file = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        if let Ok(parent) = std::fs::File::open(dir) {
            let _ = parent.sync_all();
        }
    }
    Ok(())
}

pub fn prove_or_restore() {
    let Some(marker) = pinned_core::boot::marker_path() else { return };
    if !marker.exists() {
        return;
    }
    if !pinned_core::boot::proving() {
        rollback(&marker);
    }
    let wait = proof_wait();
    std::thread::spawn(move || {
        std::thread::sleep(wait);
        if pinned_core::boot::leave_if_starting() {
            rollback(&marker);
        }
    });
}

pub async fn roll_back_if_refused(machine: &Machine) {
    loop {
        pinned_core::boot::refused().await;
        if !pinned_core::boot::leave_if_running() {
            continue;
        }
        machine.quit();
        if let Some(marker) = pinned_core::boot::marker_path() {
            rollback(&marker);
        }
    }
}

fn proof_wait() -> Duration {
    std::env::var(pinned_core::boot::WAIT_ENV).ok().and_then(|text| text.parse::<u64>().ok()).map(Duration::from_secs).unwrap_or(READY_WAIT)
}

fn binary_of(marker: &Path) -> PathBuf {
    let name = marker.file_name().unwrap_or_default().to_string_lossy();
    let bare = name.strip_suffix(".unproven").unwrap_or(&name);
    marker.with_file_name(bare)
}

fn rollback(marker: &Path) -> ! {
    let dest = binary_of(marker);
    if let Err(error) = restore(&dest) {
        eprintln!("pinned-server: {error}");
        std::process::exit(1);
    }
    let error = Command::new(&dest)
        .args(std::env::args_os().skip(1))
        .env_remove(pinned_core::boot::PROOF_ENV)
        .env_remove(pinned_core::boot::MARKER_ENV)
        .env_remove(pinned_core::boot::VERSION_ENV)
        .env_remove(pinned_core::boot::WAIT_ENV)
        .exec();
    eprintln!("pinned-server: could not start the previous binary: {error}");
    std::process::exit(1);
}

fn hand_over(machine: &Machine, plan: &Plan, dest: &Path, to: &str) -> Result<(), String> {
    machine.report(UpdateStatus::Restarting { to: to.to_owned() });
    machine.quit();
    let error = exec_planned(plan, dest);
    let restored = restore(dest);
    for tab in machine.reopen_sessions() {
        let _ = machine.start_claude(&tab, 120, 32);
    }
    match restored {
        Ok(()) => Err(format!("could not replace this process: {error}")),
        Err(restore_error) => Err(format!("could not replace this process: {error}; {restore_error}")),
    }
}

fn exec_planned(plan: &Plan, dest: &Path) -> std::io::Error {
    Command::new(&plan.program)
        .args(&plan.args)
        .current_dir(&plan.cwd)
        .env(pinned_core::boot::PROOF_ENV, &plan.proof)
        .env(pinned_core::boot::MARKER_ENV, unproven_path(dest))
        .env_remove(pinned_core::boot::VERSION_ENV)
        .exec()
}

#[cfg(test)]
pub(crate) fn start_replacement(plan: &Plan, dest: &Path) -> Result<(), String> {
    let error = exec_planned(plan, dest);
    let start = format!("could not start {}: {error}", plan.program.display());
    match restore(dest) {
        Ok(()) => Err(start),
        Err(restore_error) => Err(format!("{start}; {restore_error}")),
    }
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

    struct Clean(PathBuf);

    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn checked(cmd: &mut Command) {
        let output = cmd.stdin(std::process::Stdio::null()).output().unwrap_or_else(|error| panic!("could not run {cmd:?}: {error}"));
        assert!(
            output.status.success(),
            "command failed: {cmd:?}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn sign_release(dir: &Path, version: &str, private_key: &str) {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        std::fs::create_dir_all(dir).unwrap();
        let latest = format!(r#"{{"version":"{version}"}}"#);
        let x86_name = "pinned-server-x86_64-unknown-linux-musl";
        let arm_name = "pinned-server-aarch64-unknown-linux-musl";
        let x86 = b"x86-binary";
        let arm = b"arm-binary";
        std::fs::write(dir.join("latest.json"), latest).unwrap();
        std::fs::write(dir.join(x86_name), x86).unwrap();
        std::fs::write(dir.join(arm_name), arm).unwrap();
        // A leftover list that omits latest.json must be replaced. Signing that file is not a release.
        let stale = format!("{}{}", sums_for(x86_name, x86), sums_for(arm_name, arm));
        std::fs::write(dir.join("SHA256SUMS"), stale).unwrap();
        checked(
            Command::new(repo.join("scripts/sign-node-sums.sh"))
                .arg(dir)
                .env("TAURI_SIGNING_PRIVATE_KEY", private_key)
                .env("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "")
                .current_dir(&repo),
        );
    }

    fn served(dir: &Path) -> HashMap<String, Vec<u8>> {
        ["latest.json", "pinned-server-x86_64-unknown-linux-musl", "pinned-server-aarch64-unknown-linux-musl", "SHA256SUMS", "SHA256SUMS.minisig"]
            .into_iter()
            .map(|name| (format!("/{name}"), std::fs::read(dir.join(name)).unwrap()))
            .collect()
    }

    fn install_from(home: &Path, label: &str, files: HashMap<String, Vec<u8>>, pubkey: &str) -> (UpdateStatus, Vec<u8>) {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let base = serve(files);
        let dir = home.join(label);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("pinned-server");
        std::fs::write(&dest, b"old-binary").unwrap();
        let (status, _) = NodeUpdate::at(&base).prepare("x86_64-unknown-linux-musl", &dest, pubkey, &|_| {});
        (status, std::fs::read(&dest).unwrap())
    }

    #[test]
    fn a_signed_release_installs_a_newer_version_and_refuses_a_tampered_or_older_one() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let home = std::env::temp_dir().join(format!("pinned-release-sign-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let _clean = Clean(home.clone());
        let key = home.join("release.key");
        checked(Command::new("npx").args(["tauri", "signer", "generate", "--ci", "--force", "-w"]).arg(&key).current_dir(&repo));
        let private_key = std::fs::read_to_string(&key).unwrap();
        let pubkey = std::fs::read_to_string(home.join("release.key.pub")).unwrap();
        let private_key = private_key.trim();
        let pubkey = pubkey.trim();

        let current = home.join("current");
        sign_release(&current, "9.9.9", private_key);
        let signature = std::fs::read_to_string(current.join("SHA256SUMS.minisig")).unwrap();
        let raw = base64::engine::general_purpose::STANDARD.decode(signature.lines().nth(1).unwrap()).unwrap();
        assert_eq!(&raw[..2], b"ED");
        let sums = std::fs::read_to_string(current.join("SHA256SUMS")).unwrap();
        let wrapped = std::fs::read_to_string(current.join("SHA256SUMS.sig")).unwrap();
        assert_eq!(
            verify(pubkey, &sums, &wrapped, "latest.json", &std::fs::read(current.join("latest.json")).unwrap()).unwrap_err(),
            "could not read the release signature: Invalid encoding in minisign data"
        );

        let files = served(&current);
        let (status, bytes) = install_from(&home, "valid", files.clone(), pubkey);
        assert_eq!(status, UpdateStatus::Restarting { to: "9.9.9".into() });
        assert_eq!(bytes, b"x86-binary");

        let mut tampered_latest = files.clone();
        tampered_latest.insert("/latest.json".into(), b"{\"version\":\"9.9.9\"} ".to_vec());
        let (status, bytes) = install_from(&home, "tampered-latest", tampered_latest, pubkey);
        assert_eq!(status, UpdateStatus::Failed { to: String::new(), reason: "the release checksum does not match".into() });
        assert_eq!(bytes, b"old-binary");

        let mut tampered_binary = files;
        tampered_binary.insert("/pinned-server-x86_64-unknown-linux-musl".into(), b"tampered".to_vec());
        let (status, bytes) = install_from(&home, "tampered-binary", tampered_binary, pubkey);
        assert_eq!(status, UpdateStatus::Failed { to: "9.9.9".into(), reason: "the release checksum does not match".into() });
        assert_eq!(bytes, b"old-binary");

        let legacy_key = "untrusted comment: minisign public key\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3\n";
        let legacy_pubkey = base64::engine::general_purpose::STANDARD.encode(legacy_key);
        let legacy_sig = "untrusted comment: signature from minisign secret key\nRWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\ntrusted comment: timestamp:1555779966\tfile:test\nQtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";
        let legacy = verify(&legacy_pubkey, "test", legacy_sig, "latest.json", b"test").unwrap_err();
        assert_eq!(legacy, "the release signature does not match");

        let older = home.join("older");
        sign_release(&older, "0.0.1", private_key);
        let (status, bytes) = install_from(&home, "older", served(&older), pubkey);
        assert_eq!(status, UpdateStatus::UpToDate { version: env!("CARGO_PKG_VERSION").into() });
        assert_eq!(bytes, b"old-binary");
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
        assert_eq!(plan.proof, "3e2b640c4a45777e3e885018961a7f04d722259075f43de7d0b0511d1547b2d8");
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
        assert_eq!(status, UpdateStatus::UpToDate { version: env!("CARGO_PKG_VERSION").into() });
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
    fn a_download_stops_when_the_body_passes_the_cap() {
        let err = read_capped(std::io::Cursor::new(vec![1; 32]), "http://release/binary", 8).unwrap_err();
        assert!(err.contains("larger than 8 bytes"), "{err}");
    }

    #[test]
    fn a_restore_without_a_backup_keeps_the_bytes_that_are_there() {
        let dir = std::env::temp_dir().join(format!("pinned-restore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("pinned-server");
        std::fs::write(&dest, b"new-binary").unwrap();
        std::fs::write(dir.join("pinned-server.unproven"), b"1\nabc\n").unwrap();
        assert_eq!(restore(&dest), Ok(()));
        assert_eq!(std::fs::read(&dest).unwrap(), b"new-binary");
        assert!(!dir.join("pinned-server.unproven").exists(), "the marker stayed after a restore that had no backup");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_staged_binary_is_marked_with_its_checksum() {
        let dir = std::env::temp_dir().join(format!("pinned-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("pinned-server");
        std::fs::write(&dest, b"old-binary").unwrap();
        assert_eq!(stage(&dest, b"new-binary").unwrap(), "3e2b640c4a45777e3e885018961a7f04d722259075f43de7d0b0511d1547b2d8");
        assert_eq!(std::fs::read_to_string(dir.join("pinned-server.unproven")).unwrap(), "3e2b640c4a45777e3e885018961a7f04d722259075f43de7d0b0511d1547b2d8\n");
        assert_eq!(std::fs::read(dir.join("pinned-server.bak")).unwrap(), b"old-binary");
        assert_eq!(std::fs::read(&dest).unwrap(), b"new-binary");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_stage_does_not_replace_a_good_backup_with_unproven_bytes() {
        let dir = std::env::temp_dir().join(format!("pinned-restage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("pinned-server");
        std::fs::write(&dest, b"running").unwrap();
        stage(&dest, b"unproven").unwrap();
        assert_eq!(std::fs::read(dir.join("pinned-server.bak")).unwrap(), b"running");
        stage(&dest, b"newer").unwrap();
        assert_eq!(std::fs::read(dir.join("pinned-server.bak")).unwrap(), b"running");
        assert_eq!(std::fs::read(&dest).unwrap(), b"newer");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_binary_that_cannot_start_puts_the_old_bytes_back() {
        let dir = std::env::temp_dir().join(format!("pinned-spawn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("pinned-server");
        std::fs::write(&dest, b"not-a-binary").unwrap();
        std::fs::write(dir.join("pinned-server.bak"), b"old-binary").unwrap();
        let mut perms = std::fs::metadata(&dest).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&dest, perms).unwrap();
        // A failed exec has already entered this directory. Keep it on the process's own.
        let plan = Plan { program: dest.clone(), args: vec!["--version".into()], cwd: std::env::current_dir().unwrap(), proof: String::new() };
        let err = start_replacement(&plan, &dest).unwrap_err();
        assert!(err.contains("could not start"), "{err}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"old-binary");
        assert!(!dir.join("pinned-server.unproven").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_hand_over_keeps_nudging_the_tabs_it_reopens() {
        use pinned_core::accounts::Provider;
        use pinned_core::machine::{Event, Launch};
        use pinned_core::wire::Call;
        let dir = std::env::temp_dir().join(format!("pinned-hand-over-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("app")).unwrap();
        let claude = dir.join("claude");
        std::fs::write(&claude, "#!/bin/sh\n[ \"$1 $2\" = \"auth status\" ] && { echo '{\"loggedIn\":true}'; exit 0; }\nexec cat\n").unwrap();
        std::fs::set_permissions(&claude, Permissions::from_mode(0o755)).unwrap();
        let launch = Launch {
            exe: "/opt/pinned/bin/pinned-server".into(),
            bins: std::collections::BTreeMap::from([(Provider::Claude, claude)]),
            path_env: None,
            settings: dir.join("settings.json"),
            mcp_config: None,
            reminders: false,
            plugin_dir: dir.join("plugin"),
            tutorial_helper: dir.join("plugin/tutorial.mjs"),
            sock: dir.join("sock"),
            vector_search: false,
        };
        let (nudge, nudges) = std::sync::mpsc::channel();
        let nudge = Mutex::new(nudge);
        let machine = Machine::new(launch, Some(dir.clone()), Some(dir.clone()), move |event| {
            if let Event::Thread { tab } = event {
                let _ = nudge.lock().unwrap().send(tab);
            }
        });
        machine.call(Call::AddAccount { provider: Provider::Claude, name: "Work".into() }).unwrap();
        let projects = dir.join("accounts/work/projects/app");
        std::fs::create_dir_all(&projects).unwrap();
        let saved = |session: &str| {
            std::fs::write(projects.join(format!("{session}.jsonl")), "{\"type\":\"user\"}\n").unwrap();
            pinned_core::sessions::Saved {
                cwd: dir.join("app"),
                session: session.into(),
                env: vec![],
                bypass: false,
                interrupted: false,
                resume_at: None,
                held: false,
                account: "work".into(),
                model: None,
                permission_mode: None,
            }
        };
        pinned_core::sessions::save(&dir.join("sessions.json"), &(1..=8).map(|n| saved(&format!("s{n}"))).collect::<Vec<_>>());
        let before = machine.restore_tabs();
        for tab in &before {
            machine.start_claude(tab, 80, 24).unwrap();
        }
        machine.call(Call::WatchThreads { tabs: before.clone() }).unwrap();
        let mut first: Vec<String> = (0..before.len()).map(|_| nudges.recv_timeout(Duration::from_secs(2)).unwrap()).collect();
        first.sort();
        assert_eq!(first, before, "the first look");

        let dest = dir.join("bin/pinned-server");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"not-a-binary").unwrap();
        std::fs::set_permissions(&dest, Permissions::from_mode(0o644)).unwrap();
        let plan = Plan { program: dest.clone(), args: vec![], cwd: std::env::current_dir().unwrap(), proof: String::new() };
        let failed = hand_over(&machine, &plan, &dest, "9.9.9").unwrap_err();
        assert!(failed.starts_with("could not replace this process"), "{failed}");

        let s1 = machine.tabs().into_iter().find(|tab| tab.session.as_deref() == Some("s1")).expect("s1 did not reopen").id;
        assert!(!before.contains(&s1), "the reopened tab {s1} has a new id");
        while nudges.try_recv().is_ok() {}
        machine.call(Call::WatchThreads { tabs: vec![s1.clone()] }).unwrap();
        assert_eq!(nudges.recv_timeout(Duration::from_secs(2)), Ok(s1.clone()), "the reopened tab is looked at once it is watched");
        std::fs::OpenOptions::new().append(true).open(projects.join("s1.jsonl")).unwrap().write_all(b"{\"type\":\"user\"}\n").unwrap();
        assert_eq!(nudges.recv_timeout(Duration::from_secs(2)), Ok(s1), "a line appended after the failed hand-over nudges");
        machine.quit();
        let _ = std::fs::remove_dir_all(&dir);
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
