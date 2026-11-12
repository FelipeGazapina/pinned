fn main() {
    // The web UI is embedded at build time, so a change there must rebuild the app.
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=src/dictation-helper.swift");
    build_dictation_helper();
    tauri_build::build()
}

/// Compiles the on-device dictation sidecar (macOS Speech framework) with swiftc.
/// A missing or failing swiftc only warns: the app still builds, and the mic
/// button then reports that dictation isn't in this build.
// Host-side compiler call, not an app child: exempt from the no-console-window rule.
#[cfg(target_os = "macos")]
#[allow(clippy::disallowed_methods)]
fn build_dictation_helper() {
    use std::process::Command;
    let out = std::env::var("OUT_DIR").unwrap_or_default();
    let dest = format!("{out}/dictation-helper");
    let status = Command::new("swiftc")
        .args([
            "-O",
            "src/dictation-helper.swift",
            "-o",
            &dest,
            "-framework",
            "Speech",
            "-framework",
            "AVFoundation",
        ])
        .status();
    match status {
        Ok(ok) if ok.success() => {
            // Tauri bundles `binaries/` next to the app; copy there so
            // `bundle.resources` picks the helper up. Best effort only.
            let _ = std::fs::create_dir_all("binaries");
            if let Err(e) = std::fs::copy(&dest, "binaries/dictation-helper") {
                println!("cargo:warning=pinned: could not stage dictation-helper: {e}");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(
                    "binaries/dictation-helper",
                    std::fs::Permissions::from_mode(0o755),
                );
            }
            sign_dictation_helper("binaries/dictation-helper");
        }
        Ok(ok) => println!("cargo:warning=pinned: swiftc failed ({ok}); this build has no dictation"),
        Err(e) => println!("cargo:warning=pinned: swiftc not found ({e}); this build has no dictation"),
    }
}

/// Tauri signs the app but leaves bundled resources as the linker signed them. Sign the helper
/// with the app's entitlements and the hardened runtime, under its own identifier, so it records
/// under the same rules as the app that macOS holds responsible for it.
#[cfg(target_os = "macos")]
fn sign_dictation_helper(path: &str) {
    println!("cargo:rerun-if-changed=Entitlements.plist");
    let status = std::process::Command::new("codesign")
        .args([
            "--force",
            "--sign",
            "-",
            "--options",
            "runtime",
            "--identifier",
            "dev.personal-harness.pinned.dictation-helper",
            "--entitlements",
            "Entitlements.plist",
            path,
        ])
        .status();
    match status {
        Ok(ok) if ok.success() => {}
        Ok(ok) => println!("cargo:warning=pinned: codesign of dictation-helper failed ({ok})"),
        Err(e) => println!("cargo:warning=pinned: codesign not found ({e})"),
    }
}

#[cfg(not(target_os = "macos"))]
fn build_dictation_helper() {
    // Keep the bundled resource path valid off-macOS; the helper never runs there.
    let _ = std::fs::create_dir_all("binaries");
    if !std::path::Path::new("binaries/dictation-helper").exists() {
        let _ = std::fs::write("binaries/dictation-helper", b"pinned dictation is macOS-only\n");
    }
}
