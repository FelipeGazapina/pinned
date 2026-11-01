fn main() {
    println!("cargo::rustc-check-cfg=cfg(root)");
    println!("cargo::rerun-if-env-changed=USER");
    let uid = std::process::Command::new("id").arg("-u").output();
    if uid.is_ok_and(|id| id.stdout.trim_ascii() == b"0") {
        println!("cargo::rustc-cfg=root");
    }
}
