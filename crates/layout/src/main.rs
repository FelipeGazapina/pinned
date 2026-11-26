//! `cargo run -p pinned-layout`                     checks the checkout, exits 1 on an error
//! `cargo run -p pinned-layout -- new ui <name>`     ui/gui-<name>.js, .css and <name>.test.js, loaded by index.html
//! `cargo run -p pinned-layout -- new core <name>`   crates/core/src/<name>.rs, declared in lib.rs

use std::path::Path;
use std::process::exit;

const USAGE: &str = "usage: pinned-layout [new ui <name> | new core <name>]";

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap_or(root);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let made = match args.as_slice() {
        [] => Ok(vec![]),
        ["new", "ui", name] => pinned_layout::scaffold::ui(&root, name),
        ["new", "core", name] => pinned_layout::scaffold::core(&root, name),
        _ => {
            eprintln!("{USAGE}");
            exit(2);
        }
    };
    match made {
        Ok(files) => {
            for file in files {
                println!("wrote {}", file.strip_prefix(&root).unwrap_or(&file).display());
            }
        }
        Err(e) => {
            eprintln!("pinned-layout: {e}");
            exit(1);
        }
    }
    let report = pinned_layout::check(&root);
    println!("{report}");
    if !report.errors.is_empty() {
        exit(1);
    }
}
