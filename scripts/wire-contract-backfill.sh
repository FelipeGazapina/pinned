#!/usr/bin/env bash
# Build crates/core/wire-contract/<version>.json from a release tag that predates the snapshot tool.
# Usage: scripts/wire-contract-backfill.sh v0.8.4
set -euo pipefail

tag="${1:?usage: scripts/wire-contract-backfill.sh <tag>}"
root=$(cd "$(dirname "$0")/.." && pwd)
version="${tag#v}"
dest="$root/crates/core/wire-contract/${version}.json"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/wire-contract.XXXXXX")
rmdir "$tmp"

cleanup() {
    git -C "$root" worktree remove --force "$tmp" >/dev/null 2>&1 || true
}
trap cleanup EXIT

git -C "$root" worktree add --detach "$tmp" "$tag" >/dev/null
export PATH="${HOME}/.rustup/toolchains/stable-aarch64-apple-darwin/bin:${HOME}/.cargo/bin:${PATH}"

python3 - "$tmp" "$version" "$dest" <<'PY'
import json, re, sys
from pathlib import Path

tree, version, dest = sys.argv[1:]
src = Path(tree) / "crates" / "core" / "src"
lib = (src / "lib.rs").read_text()

def has_mod(module: str) -> bool:
    return f"pub mod {module};" in lib or f"mod {module};" in lib

def has_type(rel: str, needle: str) -> bool:
    path = src / rel
    return path.is_file() and needle in path.read_text()

roots = [
    ("inbound", "ClientFrame", "pinned_core::wire::ClientFrame", "wire", "wire.rs", "pub enum ClientFrame"),
    ("inbound", "MachineFrame", "pinned_core::wire::MachineFrame", "wire", "wire.rs", "pub enum MachineFrame"),
    ("inbound", "SignIn", "pinned_core::account::SignIn", "account", "account.rs", "pub struct SignIn"),
    ("inbound", "NewPassword", "pinned_core::account::NewPassword", "account", "account.rs", "pub struct NewPassword"),
    ("inbound", "NewMachine", "pinned_core::account::NewMachine", "account", "account.rs", "pub struct NewMachine"),
    ("inbound", "Pairing", "pinned_core::account::Pairing", "account", "account.rs", "pub struct Pairing"),
    ("outbound", "ToClient", "pinned_core::wire::ToClient", "wire", "wire.rs", "pub enum ToClient"),
    ("outbound", "ToMachine", "pinned_core::wire::ToMachine", "wire", "wire.rs", "pub enum ToMachine"),
    ("outbound", "Session", "pinned_core::account::Session", "account", "account.rs", "pub struct Session"),
    ("outbound", "AccountView", "pinned_core::wire::AccountView", "wire", "wire.rs", "pub struct AccountView"),
    ("outbound", "Registered", "pinned_core::account::Registered", "account", "account.rs", "pub struct Registered"),
    ("outbound", "Paired", "pinned_core::account::Paired", "account", "account.rs", "pub struct Paired"),
    ("outbound", "Failure", "pinned_core::account::Failure", "account", "account.rs", "pub struct Failure"),
    ("outbound", "MachineView", "pinned_core::wire::MachineView", "wire", "wire.rs", "pub struct MachineView"),
    ("outbound", "PairCode", "pinned_core::wire::PairCode", "wire", "wire.rs", "pub struct PairCode"),
    ("outbound", "TabView", "pinned_core::wire::TabView", "wire", "wire.rs", "pub struct TabView"),
    ("outbound", "ProjectView", "pinned_core::wire::ProjectView", "wire", "wire.rs", "pub struct ProjectView"),
    ("outbound", "Settings", "pinned_core::connectors::Settings", "connectors", "connectors.rs", "pub struct Settings"),
    ("outbound", "LinearAccount", "pinned_core::linear::Account", "linear", "linear.rs", "pub struct Account"),
    ("outbound", "Suggestions", "pinned_core::connectors::Suggestions", "connectors", "connectors.rs", "pub struct Suggestions"),
    ("outbound", "SlackAccount", "pinned_core::slack::Account", "slack", "slack.rs", "pub struct Account"),
    ("outbound", "Connector", "pinned_core::connectors::Connector", "connectors", "connectors.rs", "pub struct Connector"),
    ("outbound", "SyncProject", "pinned_core::sync::Project", "sync", "sync.rs", "pub struct Project"),
    ("outbound", "SyncEntry", "pinned_core::sync::Entry", "sync", "sync.rs", "pub struct Entry"),
    ("outbound", "SyncFile", "pinned_core::sync::File", "sync", "sync.rs", "pub struct File"),
    ("outbound", "GithubAccount", "pinned_core::github::Account", "github", "github.rs", "pub struct Account"),
    ("outbound", "GithubRepo", "pinned_core::github::Repo", "github", "github.rs", "pub struct Repo"),
    ("outbound", "VercelAccount", "pinned_core::vercel::Account", "vercel", "vercel.rs", "pub struct Account"),
    ("outbound", "VercelModel", "pinned_core::vercel::Model", "vercel", "vercel.rs", "pub struct Model"),
    ("outbound", "UpstashAccount", "pinned_core::upstash::Account", "upstash", "upstash.rs", "pub struct Account"),
]
kept = [row for row in roots if has_mod(row[3]) and has_type(row[4], row[5])]
if not any(row[1] == "ClientFrame" for row in kept):
    sys.exit("this tag has no ClientFrame; refusing to write a snapshot")
skipped = [row[1] for row in roots if row not in kept]
if skipped:
    print("skipped missing types: " + ", ".join(skipped), flush=True)

cargo = Path(tree) / "crates" / "core" / "Cargo.toml"
text = cargo.read_text()
if "schemars" not in text:
    text = text.replace('serde_json = "1"\n', 'serde_json = "1"\nschemars = "=1.2.3"\n', 1)
    if "schemars" not in text:
        text = text.replace(
            'serde_json = { version = "1", features = ["preserve_order"] }\n',
            'serde_json = { version = "1", features = ["preserve_order"] }\nschemars = "=1.2.3"\n',
            1,
        )
    if "schemars" not in text:
        sys.exit("could not add schemars to crates/core/Cargo.toml")
    cargo.write_text(text)

pat = re.compile(r"#\[derive\(([^)]*)\)\]")
for path in src.rglob("*.rs"):
    body = path.read_text()
    def repl(match):
        inner = match.group(1)
        if "Serialize" not in inner or "JsonSchema" in inner:
            return match.group(0)
        return "#[derive(" + inner + ", schemars::JsonSchema)]"
    path.write_text(pat.sub(repl, body))

wire = src / "wire.rs"
wire_text = wire.read_text()
if "struct Bytes" in wire_text and "impl schemars::JsonSchema for Bytes" not in wire_text:
    wire.write_text(wire_text.rstrip() + """

impl schemars::JsonSchema for Bytes {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Bytes".into()
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        concat!(module_path!(), "::Bytes").into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string" })
    }
}
""")

http = (Path(tree) / "crates" / "server" / "src" / "http.rs").read_text()
routes = []
for match in re.finditer(r'route\(\s*"(GET|POST|PUT|DELETE) ([^"]+)"\s*\)', http):
    routes.append(f"{match.group(1)} {match.group(2)}")
if not routes:
    for match in re.finditer(r'\.route\(\s*"([^"]+)"\s*,\s*(get|post|put|delete)\b', http):
        routes.append(f"{match.group(2).upper()} {match.group(1)}")
    for match in re.finditer(r'\.route_service\(\s*"([^"]+)"', http):
        routes.append(f"GET {match.group(1)}")
if not routes:
    sys.exit("found no HTTP routes")

inserts = []
for direction, name, ty, _mod, _file, _needle in kept:
    map_name = direction
    inserts.append(
        f'    {map_name}.insert("{name}".into(), serde_json::json!({{"send": schema::<{ty}>(true), "accept": schema::<{ty}>(false)}}));'
    )

route_list = ", ".join(json.dumps(route) for route in routes)
bin_dir = src / "bin"
bin_dir.mkdir(exist_ok=True)
(bin_dir / "wire-contract.rs").write_text(f"""use std::collections::BTreeMap;

fn schema<T: schemars::JsonSchema>(serialize: bool) -> serde_json::Value {{
    let mut settings = schemars::generate::SchemaSettings::draft2020_12();
    settings.untagged_enum_variant_titles = true;
    let settings = if serialize {{ settings.for_serialize() }} else {{ settings.for_deserialize() }};
    serde_json::to_value(settings.into_generator().into_root_schema_for::<T>()).expect("schema")
}}

fn main() {{
    let version = std::env::args().nth(1).expect("version");
    let dest = std::env::args().nth(2).expect("dest");
    let routes = [{route_list}];
    let mut inbound: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut outbound: BTreeMap<String, serde_json::Value> = BTreeMap::new();
{chr(10).join(inserts)}
    let contract = serde_json::json!({{
        "version": version,
        "routes": routes,
        "inbound": inbound,
        "outbound": outbound,
    }});
    if let Some(parent) = std::path::Path::new(&dest).parent() {{
        let _ = std::fs::create_dir_all(parent);
    }}
    std::fs::write(&dest, serde_json::to_string_pretty(&contract).unwrap() + "\\n").unwrap();
    println!("{{dest}}");
}}
""")
print(f"injected {len(kept)} roots and {len(routes)} routes for {version}", flush=True)
PY

(
    cd "$tmp"
    cargo run -q -p pinned-core --bin wire-contract -- "$version" "$dest"
)
echo "wrote $dest"
