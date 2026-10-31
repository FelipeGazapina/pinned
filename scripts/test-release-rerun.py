#!/usr/bin/env python3

from __future__ import annotations

import base64
import fnmatch
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

import workflow

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
TAG = "v0.17.0"
NEXT_TAG = "v0.17.1"
RC_TAG = "v0.17.0-rc.1"
REPO = "FelipeGazapina/pinned"
ARM_NAME = "pinned-server-aarch64-unknown-linux-musl"
X86_NAME = "pinned-server-x86_64-unknown-linux-musl"
ARM_BUNDLE = "pinned_aarch64.app.tar.gz"
VERSION_ORDER = (
    ("v0.17.0", "v0.17.0-rc.1", True),
    ("v0.17.0-rc.1", "v0.17.0", False),
    ("v0.17.0-rc.2", "v0.17.0-rc.1", True),
    ("v0.17.0-rc.1", "v0.17.0-rc.2", False),
    ("v0.17.0-rc.10", "v0.17.0-rc.9", True),
    ("v0.17.0-rc.9", "v0.17.0-rc.10", False),
    ("v0.17.0-rc.1", "v0.17.0-rc", True),
    ("v0.17.0-rc.1", "v0.17.0-beta.2", True),
    ("v0.17.0-rc.1", "v0.16.9", True),
    ("v0.16.10", "v0.16.9", True),
    ("v1.0.0", "v0.99.0", True),
    ("v0.17.0", "v0.17.0", False),
    ("v0.17.0", "windows-preview", True),
)
TAURI_BUNDLES = {
    "aarch64-apple-darwin": ("macos/pinned.app.tar.gz", ARM_BUNDLE, ("darwin-aarch64", "darwin-aarch64-app")),
    "x86_64-apple-darwin": ("macos/pinned.app.tar.gz", "pinned_x64.app.tar.gz", ("darwin-x86_64", "darwin-x86_64-app")),
    "x86_64-pc-windows-msvc": (
        "nsis/pinned_{version}_x64-setup.exe",
        "pinned_{version}_x64-setup.exe",
        ("windows-x86_64", "windows-x86_64-nsis"),
    ),
}
ARM = b"arm-old " * 2048
X86 = b"x86-old " * 2048
SUM_NAMES = ("latest.json", ARM_NAME, X86_NAME)
FULL = "*"
JOB_KEYS = {"needs", "runs-on", "strategy", "defaults", "steps", "env", "concurrency", "permissions"}
STEP_KEYS = {"name", "id", "uses", "run", "with", "env", "shell"}
VERIFIER = ROOT / "scripts" / "minisign-verify.mjs"
CRATE_KEY = "untrusted comment: minisign public key\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3\n"
CRATE_PREHASHED = (
    "untrusted comment: signature from minisign secret key\n"
    "RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\n"
    "trusted comment: timestamp:1556193335\tfile:test\n"
    "y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==\n"
)
CRATE_LEGACY = (
    "untrusted comment: signature from minisign secret key\n"
    "RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\n"
    "trusted comment: timestamp:1555779966\tfile:test\n"
    "QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==\n"
)
CRATE_STRAY_SIGNATURE = CRATE_PREHASHED.replace("RUQf6LRCGA9i559r", "RUQf6LRCGA9i559r!")
CRATE_STRAY_GLOBAL = CRATE_PREHASHED.replace("y/rUw2y8", "y/rUw2y8!")

GH_SOURCE = r'''#!/usr/bin/env python3
import base64, hashlib, json, os, pathlib, sys

STATE = pathlib.Path(os.environ["REPLAY_STATE"])
NODE_FILES = (
    "latest.json",
    "pinned-server-aarch64-unknown-linux-musl",
    "pinned-server-x86_64-unknown-linux-musl",
)

def load():
    return json.loads(STATE.read_text())

def save(state):
    tmp = STATE.with_suffix(".tmp")
    tmp.write_text(json.dumps(state))
    tmp.replace(STATE)

def public_mismatch(state):
    if not state["exists"] or state["draft"]:
        return None
    assets = state["assets"]
    if "latest.json" not in assets:
        return None
    latest = base64.b64decode(assets["latest.json"])
    digest = hashlib.sha256(latest).hexdigest()
    sums = base64.b64decode(assets["SHA256SUMS"]).decode() if "SHA256SUMS" in assets else ""
    listed = {}
    for line in sums.splitlines():
        parts = line.split()
        if len(parts) >= 2:
            listed.setdefault(parts[1].lstrip("*"), parts[0])
    if listed.get("latest.json") != digest:
        return f"public latest.json {digest} missing from SHA256SUMS"
    for name in NODE_FILES:
        if name not in listed:
            return f"public SHA256SUMS does not list {name}"
        if name not in assets:
            return f"public {name} is missing"
        if hashlib.sha256(base64.b64decode(assets[name])).hexdigest() != listed[name]:
            return f"public {name} does not match SHA256SUMS"
    return None

def flag_map(args):
    flags = {}
    positional = []
    i = 0
    take_value = {
        "--title", "--notes", "--notes-file", "--target", "--discussion-category",
        "-t", "-n", "-D", "--dir", "-O", "--output", "-R", "--repo", "-p", "--pattern",
        "--json", "--jq",
    }
    while i < len(args):
        arg = args[i]
        if arg.startswith("-"):
            if "=" in arg:
                key, value = arg.split("=", 1)
                flags.setdefault(key, []).append(value)
            elif arg in take_value:
                flags.setdefault(arg, []).append(args[i + 1])
                i += 2
                continue
            else:
                flags.setdefault(arg, []).append(True)
        else:
            positional.append(arg)
        i += 1
    return flags, positional

def flag_bool(flags, name):
    if name not in flags:
        return None
    value = flags[name][-1]
    if value is True or value == "true":
        return True
    if value == "false":
        return False
    return True

def split_tag(args):
    if args and not str(args[0]).startswith("-"):
        return args[0], args[1:]
    return None, args

def record(state, gap):
    if gap:
        state["violations"].append(gap)

def resolved_repo():
    if os.environ.get("GH_REPO"):
        return os.environ["GH_REPO"]
    root = pathlib.Path(os.environ["REPLAY_ROOT"]).resolve()
    here = pathlib.Path.cwd().resolve()
    for folder in (here, *here.parents):
        if (folder / ".git").exists():
            return os.environ["REPLAY_REPO"]
        if folder == root:
            break
    return None

def main():
    argv = sys.argv[1:]
    if len(argv) < 2 or argv[0] != "release":
        sys.stderr.write("fake gh: " + " ".join(sys.argv) + "\n")
        return 2
    if not (os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")):
        sys.stderr.write("gh: To use GitHub CLI in a GitHub Actions workflow, set the GH_TOKEN environment variable.\n")
        return 4
    repo = resolved_repo()
    if repo is None:
        sys.stderr.write("failed to run git: fatal: not a git repository (or any of the parent directories): .git\n")
        return 1
    if repo != os.environ["REPLAY_REPO"]:
        sys.stderr.write(f"fake gh: {repo} is not the release repository\n")
        return 2
    sub = argv[1]
    tag, rest = split_tag(argv[2:])
    flags, positional = flag_map(rest)
    state = load()
    state.setdefault("calls", []).append(sys.argv[1:])
    save(state)
    if tag is None:
        if sub != "view":
            sys.stderr.write("fake gh: missing tag\n")
            return 2
    elif tag != os.environ.get("GITHUB_REF_NAME"):
        sys.stderr.write(f"fake gh: unexpected tag {tag}\n")
        return 2

    if sub == "view":
        if tag is None:
            if os.environ.get("REPLAY_LATEST_ERROR"):
                sys.stderr.write(os.environ["REPLAY_LATEST_ERROR"] + "\n")
                return 1
            name = state.get("remote_latest") or None
            if not name and state["exists"] and state["latest"] and not state["draft"]:
                name = os.environ.get("GITHUB_REF_NAME")
            if not name:
                sys.stderr.write("release not found\n")
                return 1
            jq = (flags.get("--jq") or [None])[0]
            if jq == ".tagName":
                print(name)
                return 0
            print(json.dumps({"tagName": name}))
            return 0
        if not state["exists"]:
            sys.stderr.write("release not found\n")
            return 1
        if "--json" in flags:
            jq = (flags.get("--jq") or [None])[0]
            if jq == ".isDraft":
                print("true" if state["draft"] else "false")
            elif jq == ".isLatest":
                print("true" if state["latest"] else "false")
            else:
                print(json.dumps({"isDraft": state["draft"], "isLatest": state["latest"]}))
            return 0
        print(f"pinned {tag}")
        return 0

    if sub == "create":
        if state["exists"]:
            sys.stderr.write("release already exists\n")
            return 1
        state["exists"] = True
        state["draft"] = flag_bool(flags, "--draft") is not False
        state["latest"] = False
        save(state)
        return 0

    if sub == "edit":
        if not state["exists"]:
            sys.stderr.write("release not found\n")
            return 1
        was_draft = state["draft"]
        draft = flag_bool(flags, "--draft")
        if draft is not None and not (draft and os.environ.get("REPLAY_DRAFT_IGNORED")):
            state["draft"] = draft
        prerelease = flag_bool(flags, "--prerelease")
        if prerelease is not None:
            state["prerelease"] = prerelease
        latest = flag_bool(flags, "--latest")
        if state["prerelease"] and latest is True:
            record(state, f"asked --latest=true for prerelease {tag}")
        if latest is None and was_draft and not state["draft"]:
            latest = True
        if state["prerelease"]:
            latest = False
        if latest is True:
            state["latest"] = True
            state["remote_latest"] = os.environ.get("GITHUB_REF_NAME")
        elif latest is False:
            state["latest"] = False
        record(state, public_mismatch(state))
        save(state)
        return 0

    if sub == "upload":
        if not state["exists"]:
            sys.stderr.write("release not found\n")
            return 1
        if not positional:
            sys.stderr.write("no files to upload\n")
            return 1
        prepared = []
        for raw in positional:
            path = pathlib.Path(raw)
            data = path.read_bytes()
            if not data:
                sys.stderr.write(f"{raw} is empty\n")
                return 1
            prepared.append((path.name, data))
        cancel = os.environ.get("REPLAY_UPLOAD_CANCEL")
        fates = json.loads(cancel) if cancel else {}
        for name, data in prepared:
            fate = fates.get(name, "old") if cancel else "new"
            if fate == "old":
                continue
            if name in state["assets"]:
                if not state["draft"]:
                    state["violations"].append(f"delete {name} while the release is public")
                del state["assets"][name]
                record(state, public_mismatch(state))
                save(state)
            if fate == "missing":
                continue
            if not state["draft"]:
                state["violations"].append(f"upload {name} while the release is public")
            state["assets"][name] = base64.b64encode(data).decode()
            state["uploads"].append({"name": name, "draft": bool(state["draft"])})
            record(state, public_mismatch(state))
            save(state)
        if cancel:
            sys.stderr.write("upload cancelled\n")
            return 1
        return 0

    if sub == "download":
        if not state["exists"]:
            sys.stderr.write("release not found\n")
            return 1
        dest = pathlib.Path((flags.get("-D") or flags.get("--dir") or ["."])[0])
        patterns = (flags.get("-p") or []) + (flags.get("--pattern") or [])
        import fnmatch
        hits = [
            name for name in state["assets"]
            if not patterns or any(fnmatch.fnmatch(name, pattern) for pattern in patterns)
        ]
        if not hits:
            sys.stderr.write("no assets match the file pattern\n" if state["assets"] else "no assets to download\n")
            return 1
        dest.mkdir(parents=True, exist_ok=True)
        for name in hits:
            (dest / name).write_bytes(base64.b64decode(state["assets"][name]))
        return 0

    sys.stderr.write("fake gh: unsupported " + " ".join(sys.argv) + "\n")
    return 2

if __name__ == "__main__":
    sys.exit(main())
'''

NPX_SOURCE = r'''#!/bin/bash
set -euo pipefail
[[ "$*" == *"tauri signer sign "* ]] || {
  echo "fake npx: $*" >&2
  exit 2
}
if [[ "${REPLAY_SIGN_FAIL:-0}" == 1 ]]; then
  echo "tauri signer sign failed" >&2
  exit 1
fi
file="${!#}"
node "$REPLAY_SIGNER" tauri "$REPLAY_KEY" "$file" > "$file.sig"
'''

SIGNER_SOURCE = r'''import { createHash, createPrivateKey, generateKeyPairSync, randomBytes, sign } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { basename } from "node:path";

const [command, keyFile, file] = process.argv.slice(2);
if (command === "generate") {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const keyId = randomBytes(8);
  const raw = Buffer.from(publicKey.export({ format: "jwk" }).x, "base64url");
  const text = `untrusted comment: minisign public key: replay\n${Buffer.concat([Buffer.from("Ed"), keyId, raw]).toString("base64")}\n`;
  writeFileSync(keyFile, JSON.stringify({
    keyId: keyId.toString("hex"),
    secret: privateKey.export({ format: "pem", type: "pkcs8" }),
    pubkey: Buffer.from(text).toString("base64"),
  }));
} else {
  const key = JSON.parse(readFileSync(keyFile, "utf8"));
  const secret = createPrivateKey(key.secret);
  const body = sign(null, createHash("blake2b512").update(readFileSync(file)).digest(), secret);
  const trusted = `timestamp:1700000000\tfile:${basename(file)}\tprehashed`;
  const global = sign(null, Buffer.concat([body, Buffer.from(trusted)]), secret);
  const text = [
    "untrusted comment: signature from replay secret key",
    Buffer.concat([Buffer.from("ED"), Buffer.from(key.keyId, "hex"), body]).toString("base64"),
    `trusted comment: ${trusted}`,
    global.toString("base64"),
    "",
  ].join("\n");
  process.stdout.write(command === "tauri" ? Buffer.from(text).toString("base64") : text);
}
'''


TOOL_SOURCES = {
    "npm": '#!/bin/bash\n[[ "$*" == ci ]] || { echo "fake npm: $*" >&2; exit 2; }\n',
    "sudo": "#!/bin/bash\n",
    "curl": '#!/bin/bash\necho "the replay has no network: curl $*" >&2\nexit 2\n',
    "wget": '#!/bin/bash\necho "the replay has no network: wget $*" >&2\nexit 2\n',
    "docker": r'''#!/bin/bash
set -euo pipefail
case "$1" in
  build | rm) ;;
  create) echo replay-container ;;
  cp)
    [[ "$2" == replay-container:/downloads/. ]] || { echo "fake docker: $*" >&2; exit 2; }
    cp -R "$REPLAY_BINARIES/." "$3"
    ;;
  *) echo "fake docker: $*" >&2; exit 2 ;;
esac
''',
}

PWSH = {"pwsh", "powershell", "pwsh.exe", "powershell.exe"}
BASH_SYNTAX = re.compile(r"\$\{?[A-Za-z_]|\$(?=\()|\[\[")


def is_bash_syntax(script):
    without_pwsh_env = re.sub(r"\$env:[A-Za-z_][A-Za-z0-9_]*", "", script)
    return BASH_SYNTAX.search(without_pwsh_env) is not None


def runner_shell(label):
    lowered = label.lower()
    if "windows" in lowered:
        return "pwsh"
    if "ubuntu" in lowered or "macos" in lowered or "linux" in lowered:
        return "bash"
    return None


def context(tag, matrix=None):
    return {"tag": tag, "matrix": matrix or {}, "env": {}, "steps": {}}


def expand(value, ctx):
    def one(match):
        expr = match.group(1)
        head, _, rest = expr.partition(".")
        step_id, _, output = rest.partition(".outputs.")
        if expr == "github.repository":
            return REPO
        if expr == "github.ref_name":
            return ctx["tag"]
        if head == "secrets":
            return "replay"
        if head in ("matrix", "env") and rest in ctx[head]:
            return ctx[head][rest]
        if head == "steps" and output in ctx["steps"].get(step_id, {}):
            return ctx["steps"][step_id][output]
        raise SystemExit(f"the replay cannot expand ${{{{ {expr} }}}}")

    return re.sub(r"\$\{\{\s*(.*?)\s*\}\}", one, value)


def expanded(values, ctx):
    return {key: expand(value, ctx) for key, value in (values or {}).items()}


def needs_of(job):
    needs = job.get("needs") or []
    return [needs] if isinstance(needs, str) else needs


def legs_of(job):
    matrix = (job.get("strategy") or {}).get("matrix")
    if matrix is None:
        return [{}]
    if set(matrix) != {"include"}:
        raise SystemExit(f"the replay expands matrix include lists only, not {sorted(matrix)}")
    return matrix["include"]


def run_orders(jobs):
    for name, job in jobs.items():
        for need in needs_of(job):
            if need not in jobs:
                raise SystemExit(f"{name} needs {need}, which release.yml does not define")
    orders = []

    def extend(order):
        if len(order) == len(jobs):
            orders.append(order)
            return
        for name, job in jobs.items():
            if name not in order and all(need in order for need in needs_of(job)):
                extend(order + [name])

    extend([])
    if not orders:
        raise SystemExit("the needs of release.yml form a cycle")
    return orders


def runs_one_at_a_time(wf, job_name, tags):
    for level in (wf, wf["jobs"][job_name]):
        concurrency = level.get("concurrency")
        if concurrency is None:
            continue
        if isinstance(concurrency, str):
            concurrency = {"group": concurrency}
        groups = {expand(concurrency["group"], context(tag)) for tag in tags}
        if len(groups) == 1 and concurrency.get("cancel-in-progress", "false") == "false":
            return True
    return False


def shell_problems(wf):
    problems = []
    for name, job in wf["jobs"].items():
        runners = [expand(job["runs-on"], context(TAG, matrix)) for matrix in legs_of(job)]
        for index, step in enumerate(job["steps"], start=1):
            script = step.get("run") or ""
            if not script.strip() or not is_bash_syntax(script):
                continue
            bad = []
            for runner in runners:
                shell = workflow.shell_of(wf, job, step) or runner_shell(runner)
                shown = shell or "unresolved"
                if (shell in PWSH or shell is None) and shown not in bad:
                    bad.append(shown)
            if bad:
                first = next((item.strip() for item in script.splitlines() if item.strip()), "")
                problems.append(f"{name} step {index} runs bash syntax under {', '.join(bad)}: {first}")
    return problems


def runner_env():
    return {
        key: value
        for key, value in os.environ.items()
        if not key.startswith(("GH_", "GITHUB_", "PINNED_", "TAURI_", "REPLAY_")) and key != "RELEASE_NOTES"
    }


def sums_for(latest, arm, x86, names=SUM_NAMES):
    files = {"latest.json": latest, ARM_NAME: arm, X86_NAME: x86}
    lines = [f"{hashlib.sha256(files[name]).hexdigest()}  {name}" for name in names]
    return ("\n".join(lines) + "\n").encode()


def b64(data):
    return base64.b64encode(data).decode()


def asset_bytes(state, name):
    raw = state["assets"].get(name)
    if raw is None:
        return None
    return base64.b64decode(raw)


def bundle_bytes(target, build):
    return f"desktop bundle {target} {build}\n".encode() * 256


def empty_state():
    return {
        "exists": False,
        "draft": False,
        "prerelease": False,
        "latest": False,
        "assets": {},
        "violations": [],
        "uploads": [],
        "calls": [],
        "remote_latest": None,
        "builds": 0,
        "artifacts": {},
    }


def seeded_state(release, draft, replace=None):
    files, artifacts = release
    files = {**files, **(replace or {})}
    state = empty_state()
    state["exists"] = True
    state["draft"] = draft
    state["assets"] = {name: b64(data) for name, data in files.items()}
    state["artifacts"] = json.loads(json.dumps(artifacts))
    return state


class Replay:
    def __init__(self, wf, root):
        self.wf = wf
        self.jobs = wf["jobs"]
        self.root = pathlib.Path(root)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name, source in (("gh", GH_SOURCE), ("npx", NPX_SOURCE), *TOOL_SOURCES.items()):
            path = self.bin / name
            path.write_text(source)
            path.chmod(0o755)
        self.binaries = self.root / "docker-downloads"
        self.binaries.mkdir()
        (self.binaries / ARM_NAME).write_bytes(ARM)
        (self.binaries / X86_NAME).write_bytes(X86)
        self.signer = self.root / "signer.mjs"
        self.signer.write_text(SIGNER_SOURCE)
        self.key = self.root / "key.json"
        self.forger = self.root / "forger.json"
        for key in (self.key, self.forger):
            subprocess.run(["node", str(self.signer), "generate", str(key)], check=True)
        self.pubkey = json.loads(self.key.read_text())["pubkey"]
        self.forged_pubkey = json.loads(self.forger.read_text())["pubkey"]
        self.state_path = self.root / "state.json"
        self.actions = {
            "actions/checkout": self.checkout,
            "actions/setup-node": self.nothing,
            "dtolnay/rust-toolchain": self.nothing,
            "swatinem/rust-cache": self.nothing,
            "actions/upload-artifact": self.upload_artifact,
            "actions/download-artifact": self.download_artifact,
            "tauri-apps/tauri-action": self.tauri,
        }

    def minisig(self, data, name, key=None):
        scratch = self.root / "to-sign"
        if scratch.exists():
            shutil.rmtree(scratch)
        scratch.mkdir()
        path = scratch / name
        path.write_bytes(data)
        proc = subprocess.run(
            ["node", str(self.signer), "minisig", str(key or self.key), str(path)],
            check=True,
            capture_output=True,
        )
        return proc.stdout

    def tauri_signature(self, data, name, key=None):
        return b64(self.minisig(data, name, key))

    def release(self, build=0, notes="old", tag=TAG, key=None):
        version = tag[1:]
        files = {}
        platforms = {}
        artifacts = {}
        for target, (local, asset, keys) in TAURI_BUNDLES.items():
            asset = asset.format(version=version)
            bundle = bundle_bytes(target, build)
            signature = self.tauri_signature(bundle, pathlib.Path(local).name.format(version=version), key)
            files[asset] = bundle
            files[f"{asset}.sig"] = signature.encode()
            entry = {"signature": signature, "url": f"https://github.com/{REPO}/releases/download/{tag}/{asset}"}
            leg = {platform: entry for platform in keys}
            platforms.update(leg)
            fragment = json.dumps({"version": version, "platforms": leg}).encode()
            artifacts[f"updater-{target}"] = {"updater-fragment.json": b64(fragment)}
        latest = json.dumps(
            {"version": version, "notes": notes, "pub_date": "2026-10-30T00:00:00.000Z", "platforms": platforms},
            indent=2,
        ).encode()
        sums = sums_for(latest, ARM, X86)
        files.update(
            {
                "latest.json": latest,
                ARM_NAME: ARM,
                X86_NAME: X86,
                "SHA256SUMS": sums,
                "SHA256SUMS.minisig": self.minisig(sums, "SHA256SUMS", key),
            }
        )
        artifacts["linux-node-binaries"] = {ARM_NAME: b64(ARM), X86_NAME: b64(X86)}
        return files, artifacts

    def load_state(self):
        return json.loads(self.state_path.read_text())

    def save_state(self, state):
        self.state_path.write_text(json.dumps(state))

    def runner(self, tag):
        env = runner_env()
        env["PATH"] = str(self.bin) + os.pathsep + env.get("PATH", "")
        for tool in ("gh", "npx", *TOOL_SOURCES):
            found = shutil.which(tool, path=env["PATH"])
            if found is None or pathlib.Path(found).parent != self.bin:
                raise SystemExit(f"{tool} resolves to {found}, not the replay's fake")
        env.update(
            {
                "GITHUB_REF_NAME": tag,
                "GITHUB_REPOSITORY": REPO,
                "REPLAY_ROOT": str(self.root),
                "REPLAY_REPO": REPO,
                "REPLAY_STATE": str(self.state_path),
                "REPLAY_SIGNER": str(self.signer),
                "REPLAY_KEY": str(self.key),
                "REPLAY_BINARIES": str(self.binaries),
            }
        )
        return env

    def fresh_work(self):
        work = self.root / "work"
        if work.exists():
            shutil.rmtree(work)
        work.mkdir()
        return work

    def run(self, job_names, state, extra_env, tag):
        self.save_state(state)
        work = self.fresh_work()
        env = self.runner(tag)
        for index, name in enumerate(job_names):
            if name not in self.jobs:
                raise SystemExit(f"release.yml has no {name} job")
            early = [need for need in needs_of(self.jobs[name]) if need in job_names[index + 1 :]]
            if early:
                raise SystemExit(f"a scenario runs {name} before {early[0]}, which it needs")
        self.assembled = None
        failed = set()
        code = 0
        stderr = []
        for name in job_names:
            if any(need in failed for need in needs_of(self.jobs[name])):
                failed.add(name)
                continue
            job_code, err = self.job(name, env, extra_env, tag, work)
            stderr.append(err)
            if job_code:
                failed.add(name)
                code = code or job_code
        return code, "".join(stderr), self.load_state(), self.assembled

    def run_script(self, script, state, extra_env, tag):
        self.save_state(state)
        work = self.fresh_work()
        self.checkout({}, context(tag), {}, work)
        code, err = self.shell(["bash", "-e"], script, {**self.runner(tag), **extra_env}, work)
        return code, err, self.load_state(), None

    def job(self, name, env, extra_env, tag, work):
        job = self.jobs[name]
        unknown = sorted(set(job) - JOB_KEYS)
        if unknown:
            return 1, f"the replay does not model {', '.join(unknown)} on {name}\n"
        fail_fast = (job.get("strategy") or {}).get("fail-fast", "true") != "false"
        code = 0
        stderr = []
        for index, matrix in enumerate(legs_of(job)):
            leg_work = work / f"{name}-{index}"
            leg_work.mkdir()
            leg_code, err = self.leg(name, job, matrix, env, extra_env, tag, leg_work)
            stderr.append(err)
            assembled = leg_work / "node-release" / "latest.json"
            if assembled.exists():
                self.assembled = assembled.read_bytes()
            code = code or leg_code
            if leg_code and fail_fast:
                break
        return code, "".join(stderr)

    def leg(self, name, job, matrix, env, extra_env, tag, work):
        ctx = context(tag, matrix)
        ctx["env"] = expanded(self.wf.get("env"), ctx)
        ctx["env"].update(expanded(job.get("env"), ctx))
        job_env = {**env, **ctx["env"]}
        stderr = []
        for index, step in enumerate(job["steps"], start=1):
            unknown = sorted(set(step) - STEP_KEYS)
            if unknown:
                return 1, "".join(stderr) + f"the replay does not model {', '.join(unknown)} on {name} step {index}\n"
            step_env = {**job_env, **expanded(step.get("env"), ctx), **extra_env}
            outputs = {}
            if step.get("uses"):
                action = self.actions.get(step["uses"].split("@")[0])
                if action is None:
                    stderr.append(f"{name} step {index} uses {step['uses']}, which the replay cannot tell from a release write\n")
                    return 1, "".join(stderr)
                code, err, outputs = action(step, ctx, step_env, work)
            else:
                argv = workflow.bash_argv(workflow.shell_of(self.wf, job, step))
                code, err = self.shell(argv, expand(step["run"], ctx), step_env, work)
            stderr.append(err)
            if step.get("id"):
                ctx["steps"][step["id"]] = outputs
            if code != 0:
                return code, "".join(stderr)
        return 0, "".join(stderr)

    def shell(self, argv, script, env, work):
        path = self.root / "step.sh"
        path.write_text(script)
        proc = subprocess.run([*argv, str(path)], cwd=work, env=env, text=True, capture_output=True)
        return proc.returncode, proc.stderr

    def nothing(self, step, ctx, env, work):
        return 0, "", {}

    def checkout(self, step, ctx, env, work):
        (work / ".git").mkdir()
        shutil.copytree(ROOT / "scripts", work / "scripts", ignore=shutil.ignore_patterns("__pycache__"))
        conf = {"plugins": {"updater": {"pubkey": self.pubkey}}}
        (work / "tauri.conf.json").write_text(json.dumps(conf))
        return 0, "", {}

    def upload_artifact(self, step, ctx, env, work):
        inputs = expanded(step.get("with"), ctx)
        name = inputs["name"]
        paths = [line.strip() for line in inputs["path"].splitlines() if line.strip()]
        files = [work / path for path in paths]
        missing = [path for path, file in zip(paths, files) if not file.is_file()]
        if missing:
            if inputs.get("if-no-files-found", "warn") == "error":
                return 1, f"No files were found with the provided path: {' '.join(missing)}. No artifacts will be uploaded.\n", {}
            return 0, "", {}
        state = self.load_state()
        if name in state["artifacts"] and inputs.get("overwrite") != "true":
            return 1, (
                "Failed to CreateArtifact: Received non-retryable error: Failed request: (409) Conflict: "
                f"an artifact with this name already exists on the workflow run: {name}\n"
            ), {}
        base = pathlib.Path(os.path.commonpath([str(file.parent) for file in files]))
        state["artifacts"][name] = {str(file.relative_to(base)): b64(file.read_bytes()) for file in files}
        self.save_state(state)
        return 0, "", {}

    def download_artifact(self, step, ctx, env, work):
        inputs = expanded(step.get("with"), ctx)
        dest = work / inputs.get("path", ".")
        artifacts = self.load_state()["artifacts"]
        if "name" in inputs:
            if inputs["name"] not in artifacts:
                return 1, f"Unable to download artifact(s): Artifact not found for name: {inputs['name']}\n", {}
            chosen = {inputs["name"]: dest}
        else:
            pattern = inputs.get("pattern", "*")
            merge = inputs.get("merge-multiple") == "true"
            chosen = {name: dest if merge else dest / name for name in sorted(artifacts) if fnmatch.fnmatch(name, pattern)}
        for name, folder in chosen.items():
            for relative, data in artifacts[name].items():
                path = folder / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(base64.b64decode(data))
        return 0, "", {}

    def tauri(self, step, ctx, env, work):
        inputs = expanded(step.get("with"), ctx)
        if inputs.get("tagName") != ctx["tag"]:
            return 1, f"tauri-action tagName {inputs.get('tagName')} is not {ctx['tag']}\n", {}
        args = inputs.get("args", "").split()
        target = args[args.index("--target") + 1] if "--target" in args else None
        if target not in TAURI_BUNDLES:
            return 1, f"the replay does not build {target}\n", {}
        version = ctx["tag"][1:]
        local, asset, _ = TAURI_BUNDLES[target]
        local = local.format(version=version)
        asset = asset.format(version=version)
        build = self.load_state()["builds"] + 1
        bundle = work / "target" / target / "release" / "bundle" / local
        bundle.parent.mkdir(parents=True, exist_ok=True)
        bundle.write_bytes(bundle_bytes(target, build))
        signature = bundle.parent / f"{bundle.name}.sig"
        signature.write_text(self.tauri_signature(bundle.read_bytes(), bundle.name))
        staged = work / "tauri-upload"
        staged.mkdir()
        shutil.copyfile(bundle, staged / asset)
        shutil.copyfile(signature, staged / f"{asset}.sig")
        uploads = [staged / asset, staged / f"{asset}.sig"]
        if (inputs.get("includeUpdaterJson") or "true").lower() != "false":
            (staged / "latest.json").write_text(json.dumps({"version": version, "pub_date": "tauri"}))
            uploads.append(staged / "latest.json")
        action_env = {**env, "GH_REPO": REPO}
        code, err = 0, ""
        for upload in uploads:
            step_code, step_err = self.shell(
                ["bash", "-e"], f'gh release upload "$GITHUB_REF_NAME" "{upload}" --clobber\n', action_env, work
            )
            err += step_err
            if step_code:
                return step_code, err, {}
        state = self.load_state()
        state["builds"] = build
        self.save_state(state)
        return code, err, {"artifactPaths": json.dumps([str(bundle), str(signature)]), "appVersion": version}


def unchanged(before, after):
    for name in ("latest.json", "SHA256SUMS"):
        if asset_bytes(before, name) != asset_bytes(after, name):
            return False
    return True


def consistent(state):
    latest = asset_bytes(state, "latest.json")
    sums = asset_bytes(state, "SHA256SUMS")
    if latest is None or sums is None:
        return False
    digest = hashlib.sha256(latest).hexdigest()
    for line in sums.decode().splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[0] == digest and parts[-1].lstrip("*") == "latest.json":
            return True
    return False


def scenario_errors(code, stderr, state, assembled, expect):
    errors = list(state["violations"])
    for key, value in expect.items():
        if key == "code":
            if code != value:
                errors.append(f"exit {code}, want {value}: {stderr.strip()}")
        elif key == "draft":
            if bool(state["draft"]) != value:
                errors.append(f"draft={state['draft']}, want {value}")
        elif key == "prerelease":
            if bool(state["prerelease"]) != value:
                errors.append(f"prerelease={state['prerelease']}, want {value}")
        elif key == "latest_flag":
            if bool(state["latest"]) != value:
                errors.append(f"latest={state['latest']}, want {value}")
        elif key == "unchanged_from":
            if not unchanged(value, state):
                errors.append("latest.json or SHA256SUMS changed on the release")
        elif key == "latest_assembled":
            if assembled is None or asset_bytes(state, "latest.json") != assembled:
                errors.append("latest.json on the release is not the manifest sign-node assembled")
        elif key == "consistent":
            if consistent(state) != value:
                errors.append(f"consistent={consistent(state)}, want {value}")
        elif key == "stderr_has":
            if value not in stderr:
                errors.append(f"stderr missing [{value}]: {stderr.strip()}")
        elif key == "uploaded":
            names = [item["name"] for item in state["uploads"]]
            if value not in names:
                errors.append(f"did not upload {value}")
        else:
            raise AssertionError(key)
    return errors
def crate_vector_errors(root):
    pubkey = b64(CRATE_KEY.encode())
    errors = []
    for label, message, signature, want in (
        ("prehashed", b"test", CRATE_PREHASHED, 0),
        ("tampered", b"tesT", CRATE_PREHASHED, 1),
        ("legacy", b"test", CRATE_LEGACY, 1),
        ("stray byte in the signature line", b"test", CRATE_STRAY_SIGNATURE, 1),
        ("stray byte in the trusted comment signature", b"test", CRATE_STRAY_GLOBAL, 1),
    ):
        slug = re.sub(r"\W+", "-", label)
        message_path = root / f"vector-{slug}"
        signature_path = root / f"vector-{slug}.minisig"
        message_path.write_bytes(message)
        signature_path.write_text(signature)
        proc = subprocess.run(
            ["node", str(VERIFIER), pubkey, str(message_path), str(signature_path)],
            text=True,
            capture_output=True,
        )
        if proc.returncode != want:
            errors.append(f"{label} vector exit {proc.returncode}, want {want}: {proc.stderr.strip()}")
    return errors


def version_order_errors():
    source = (ROOT / "scripts" / "release-asset.sh").read_text()
    match = re.search(r"^version_is_newer\(\) \{\n.*?^\}\n", source, re.S | re.M)
    if match is None:
        return ["scripts/release-asset.sh has no version_is_newer"]
    errors = []
    for left, right, want in VERSION_ORDER:
        proc = subprocess.run(
            ["bash", "-c", match.group(0) + 'version_is_newer "$1" "$2"', "order", left, right],
            text=True,
            capture_output=True,
        )
        if proc.stderr.strip() or (proc.returncode == 0) != want:
            errors.append(f"{left} newer than {right}: exit {proc.returncode}, want {want} {proc.stderr.strip()}")
    return errors


def public_clobber_errors(replay):
    old, _ = replay.release()
    newer, _ = replay.release(notes="new")
    state = seeded_state((old, {}), draft=False)
    state_path = replay.root / "clobber-state.json"
    work = replay.root / "clobber-work"
    work.mkdir()
    for name in ("latest.json", "SHA256SUMS", "SHA256SUMS.minisig"):
        (work / name).write_bytes(newer[name])
    env = runner_env()
    env.update(
        {
            "REPLAY_STATE": str(state_path),
            "REPLAY_ROOT": str(replay.root),
            "REPLAY_REPO": REPO,
            "GH_REPO": REPO,
            "GH_TOKEN": "replay",
            "GITHUB_REF_NAME": TAG,
        }
    )
    errors = []
    for cancel, want in (
        (None, {"latest.json": "new", "SHA256SUMS": "new", "SHA256SUMS.minisig": "new"}),
        (
            {"latest.json": "new", "SHA256SUMS": "missing"},
            {"latest.json": "new", "SHA256SUMS": "missing", "SHA256SUMS.minisig": "old"},
        ),
    ):
        state_path.write_text(json.dumps(state))
        run_env = {**env, "REPLAY_UPLOAD_CANCEL": json.dumps(cancel)} if cancel else env
        proc = subprocess.run(
            [str(replay.bin / "gh"), "release", "upload", TAG, "latest.json", "SHA256SUMS", "SHA256SUMS.minisig", "--clobber"],
            cwd=work,
            env=run_env,
            text=True,
            capture_output=True,
        )
        after = json.loads(state_path.read_text())
        if proc.returncode != (1 if cancel else 0):
            errors.append(f"clobber upload exit {proc.returncode}: {proc.stderr.strip()}")
        if "delete latest.json while the release is public" not in after["violations"]:
            errors.append(f"public clobber recorded no delete: {after['violations']}")
        for name, fate in want.items():
            got = asset_bytes(after, name)
            seen = "missing" if got is None else "new" if got == newer[name] else "old" if got == old[name] else "other"
            if seen != fate:
                errors.append(f"cancel {cancel}: {name} is {seen}, want {fate}")
    return errors


def report(label, errors):
    if errors:
        print(f"FAIL {label}")
        for error in errors:
            print(f"  {error}")
        return 1
    print(f"ok: {label}")
    return 0




def concurrent_publish_errors(replay):
    tags = (NEXT_TAG, TAG)
    serial = runs_one_at_a_time(replay.wf, "publish", tags)
    before = "v0.16.0"
    latest = before
    errors = []
    for tag in tags:
        state = seeded_state(replay.release(tag=tag), draft=True)
        state["remote_latest"] = latest if serial else before
        code, stderr, after, _ = replay.run(["publish"], state, {}, tag)
        errors += [f"{tag}: {violation}" for violation in after["violations"]]
        if code != 0:
            errors.append(f"{tag} publish exit {code}: {stderr.strip()}")
        if after["latest"]:
            latest = tag
    if latest != NEXT_TAG:
        errors.append(f"latest is {latest} after {NEXT_TAG} and then {TAG} publish, want {NEXT_TAG}")
    return errors


def expand_runs(runs, orders):
    expansions = [[]]
    for job_names, extra_env in runs:
        choices = orders if job_names == FULL else [job_names]
        expansions = [done + [(choice, extra_env)] for done in expansions for choice in choices]
    return expansions


def main():
    wf = workflow.load(WORKFLOW)
    problems = shell_problems(wf)
    for problem in problems:
        print(f"FAIL shell: {problem}")
    if not problems:
        print("ok: shells")
    orders = run_orders(wf["jobs"])
    sha = subprocess.check_output(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, text=True).strip()
    print(f"replay at {sha}")
    for order in orders:
        print(f"  full run: {' > '.join(order)}")

    failed = report("version order", version_order_errors())
    with tempfile.TemporaryDirectory(prefix="release-replay-") as tmp:
        replay = Replay(wf, tmp)
        failed += report("minisign-verify crate vectors", crate_vector_errors(replay.root))
        failed += report("public clobber", public_clobber_errors(replay))
        failed += report(f"{NEXT_TAG} and {TAG} publish at once", concurrent_publish_errors(replay))

        current = replay.release()
        newer = replay.release(notes="new")
        candidate = replay.release(tag=RC_TAG)

        def public():
            return seeded_state(current, draft=False)

        def drafted(remote_latest=None, replace=None, release=current):
            state = seeded_state(release, draft=True, replace=replace)
            state["remote_latest"] = remote_latest
            return state

        cases = []

        def add(title, runs, state, expect, tag=TAG):
            cases.append((title, runs, state, expect, tag))

        add(
            "full run on a new tag",
            [(FULL, {})],
            empty_state(),
            {"code": 0, "draft": False, "prerelease": False, "latest_flag": True, "consistent": True, "latest_assembled": True},
        )
        add(
            "full run, then re-run all jobs",
            [(FULL, {}), (FULL, {})],
            empty_state(),
            {"code": 0, "draft": False, "consistent": True, "latest_assembled": True},
        )
        add(
            "full run of a release candidate",
            [(FULL, {})],
            empty_state(),
            {"code": 0, "draft": False, "prerelease": True, "latest_flag": False, "consistent": True},
            tag=RC_TAG,
        )
        add(
            "sign-node re-run, signer fails",
            [(["sign-node"], {"REPLAY_SIGN_FAIL": "1"})],
            public(),
            {
                "stderr_has": "tauri signer sign failed",
                "unchanged_from": public(),
                "consistent": True,
            },
        )
        add(
            "sign-node re-run, signer succeeds",
            [(["sign-node"], {})],
            public(),
            {"code": 0, "draft": True, "consistent": True, "latest_assembled": True},
        )
        add(
            "sign-node re-run when gh leaves the release public",
            [(["sign-node"], {"REPLAY_DRAFT_IGNORED": "1"})],
            public(),
            {
                "draft": False,
                "stderr_has": f"gh release edit --draft=true left {TAG} public",
                "unchanged_from": public(),
                "consistent": True,
            },
        )
        add(
            "sign-node re-run then publish",
            [(["sign-node", "publish"], {})],
            public(),
            {
                "code": 0,
                "draft": False,
                "latest_flag": True,
                "consistent": True,
                "latest_assembled": True,
            },
        )
        add(
            "build re-run",
            [(["build"], {})],
            public(),
            {
                "code": 0,
                "draft": True,
                "uploaded": ARM_BUNDLE,
                "unchanged_from": public(),
                "consistent": True,
            },
        )
        add(
            "publish re-run",
            [(["publish"], {})],
            public(),
            {
                "code": 0,
                "draft": False,
                "latest_flag": True,
                "consistent": True,
                "unchanged_from": public(),
            },
        )
        add(
            "publish re-run on a draft whose latest.json hash is not in SHA256SUMS",
            [(["publish"], {})],
            drafted(replace={"latest.json": newer[0]["latest.json"]}),
            {
                "draft": True,
                "stderr_has": "latest.json does not match SHA256SUMS",
                "consistent": False,
            },
        )
        add(
            "build re-run then publish without sign-node",
            [(["build", "publish"], {})],
            public(),
            {
                "draft": True,
                "stderr_has": f"pinned_{TAG[1:]}_x64-setup.exe does not match its signature in latest.json",
                "consistent": True,
                "unchanged_from": public(),
            },
        )
        add(
            "publish an older tag does not take latest",
            [(["publish"], {})],
            drafted(remote_latest="v0.18.0"),
            {
                "code": 0,
                "draft": False,
                "latest_flag": False,
                "consistent": True,
                "unchanged_from": public(),
            },
        )
        add(
            "publish a newer tag takes latest",
            [(["publish"], {})],
            drafted(remote_latest="v0.16.0"),
            {
                "code": 0,
                "draft": False,
                "latest_flag": True,
                "consistent": True,
                "unchanged_from": public(),
            },
        )
        add(
            "publish when the latest release cannot be read",
            [(["publish"], {"REPLAY_LATEST_ERROR": f"HTTP 503: Service Unavailable (https://api.github.com/repos/{REPO}/releases/latest)"})],
            drafted(),
            {"draft": True, "latest_flag": False, "stderr_has": "HTTP 503: Service Unavailable"},
        )
        add(
            "linux-node re-run",
            [(["linux-node"], {})],
            public(),
            {
                "code": 0,
                "draft": False,
                "consistent": True,
                "unchanged_from": public(),
            },
        )
        add(
            "publish the final tag after its release candidate takes latest",
            [(["publish"], {})],
            drafted(remote_latest=RC_TAG),
            {"code": 0, "draft": False, "prerelease": False, "latest_flag": True, "consistent": True},
        )
        add(
            "publish a release candidate after its final tag",
            [(["publish"], {})],
            drafted(remote_latest=TAG, release=candidate),
            {"code": 0, "draft": False, "prerelease": True, "latest_flag": False, "consistent": True},
            tag=RC_TAG,
        )
        add(
            "publish a release candidate newer than latest",
            [(["publish"], {})],
            drafted(remote_latest="v0.16.0", release=candidate),
            {"code": 0, "draft": False, "prerelease": True, "latest_flag": False, "consistent": True},
            tag=RC_TAG,
        )
        new, old, missing = "new", "old", "missing"
        for fates, refusal in (
            ({"SHA256SUMS": missing}, "SHA256SUMS is missing from the release"),
            ({"SHA256SUMS": new, "latest.json": new}, "SHA256SUMS.minisig does not verify SHA256SUMS"),
            ({"SHA256SUMS": new, "SHA256SUMS.minisig": missing, "latest.json": new}, "SHA256SUMS.minisig is missing from the release"),
            ({"latest.json": new, ARM_NAME: missing}, "latest.json does not match SHA256SUMS"),
            ({"SHA256SUMS": new, "SHA256SUMS.minisig": new}, "latest.json does not match SHA256SUMS"),
            ({"SHA256SUMS": new, "SHA256SUMS.minisig": new, "latest.json": missing}, "latest.json is missing from the release"),
            ({"SHA256SUMS": new, "SHA256SUMS.minisig": new, "latest.json": new, X86_NAME: missing}, f"{X86_NAME} is missing from the release"),
        ):
            shown = ", ".join(f"{name} {fate}" for name, fate in fates.items())
            add(
                f"sign-node cancelled with {shown}, the rest {old}, then publish",
                [(["sign-node"], {"REPLAY_UPLOAD_CANCEL": json.dumps(fates)}), (["publish"], {})],
                public(),
                {"draft": True, "stderr_has": refusal},
            )
        add(
            "build re-run cancelled after the old bundle is deleted, then publish",
            [(["build"], {"REPLAY_UPLOAD_CANCEL": json.dumps({ARM_BUNDLE: missing})}), (["publish"], {})],
            public(),
            {"draft": True, "stderr_has": f"{ARM_BUNDLE} is missing from the release"},
        )
        add(
            "publish re-run on a draft whose SHA256SUMS.minisig signs older sums",
            [(["publish"], {})],
            drafted(release=newer, replace={"SHA256SUMS.minisig": current[0]["SHA256SUMS.minisig"]}),
            {"draft": True, "stderr_has": "SHA256SUMS.minisig does not verify SHA256SUMS"},
        )
        add(
            "publish re-run on a draft whose binary does not match SHA256SUMS",
            [(["publish"], {})],
            drafted(replace={ARM_NAME: b"arm-new " * 2048}),
            {"draft": True, "stderr_has": f"{ARM_NAME} does not match SHA256SUMS"},
        )
        partial = sums_for(current[0]["latest.json"], ARM, X86, names=("latest.json", X86_NAME))
        add(
            "publish re-run on a draft whose SHA256SUMS does not list a binary",
            [(["publish"], {})],
            drafted(replace={"SHA256SUMS": partial, "SHA256SUMS.minisig": replay.minisig(partial, "SHA256SUMS")}),
            {"draft": True, "stderr_has": f"SHA256SUMS does not list {ARM_NAME}"},
        )
        add(
            "publish ignores PINNED_RELEASE_PUBKEY",
            [(["publish"], {"PINNED_RELEASE_PUBKEY": replay.forged_pubkey})],
            drafted(release=replay.release(notes="forged", key=replay.forger)),
            {"draft": True, "stderr_has": "SHA256SUMS.minisig does not verify SHA256SUMS"},
        )
        stray_latest = "printf '{\"version\":\"0.17.0\"}\\n' > node-release/latest.json\n"
        add(
            "upload latest.json without SHA256SUMS",
            [(
                "mkdir -p node-release\n"
                + stray_latest
                + f"printf 'arm\\n' > node-release/{ARM_NAME}\n"
                + f'bash scripts/release-asset.sh upload "$GITHUB_REF_NAME" node-release/latest.json node-release/{ARM_NAME}\n',
                {"GH_TOKEN": "replay"},
            )],
            public(),
            {"draft": False, "stderr_has": "SHA256SUMS is missing from the upload", "unchanged_from": public()},
        )
        add(
            "upload latest.json with SHA256SUMS that does not list it",
            [(
                "mkdir -p node-release\n"
                + stray_latest
                + "printf '%064d  latest.json\\n' 0 > node-release/SHA256SUMS\n"
                + 'bash scripts/release-asset.sh upload "$GITHUB_REF_NAME" node-release/SHA256SUMS node-release/latest.json\n',
                {"GH_TOKEN": "replay"},
            )],
            public(),
            {"draft": False, "stderr_has": "SHA256SUMS does not list this latest.json", "unchanged_from": public()},
        )

        ran = 0
        for title, runs, state, expect, tag in cases:
            expansions = expand_runs(runs, orders)
            for expansion in expansions:
                code, stderr, after, assembled = 0, "", state, None
                for job_names, extra_env in expansion:
                    carried = {**after, "violations": [], "uploads": [], "calls": []}
                    if isinstance(job_names, str):
                        code, stderr, after, assembled = replay.run_script(job_names, carried, extra_env, tag)
                    else:
                        code, stderr, after, assembled = replay.run(job_names, carried, extra_env, tag)
                    if after["violations"]:
                        break
                shown = title
                if len(expansions) > 1:
                    shown += " [" + "; ".join(" > ".join(names) for names, _ in expansion) + "]"
                failed += report(shown, scenario_errors(code, stderr, after, assembled, expect))
                ran += 1
    if failed:
        print(f"{failed} check(s) failed")
    else:
        print(f"ok: {ran} scenarios")
    if failed or problems:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
