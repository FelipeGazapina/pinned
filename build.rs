fn main() {
    // The web UI is embedded at build time, so a change there must rebuild the app.
    println!("cargo:rerun-if-changed=ui");
    tauri_build::build()
}
