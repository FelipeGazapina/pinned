#!/usr/bin/env python3

from __future__ import annotations

import base64
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
TAG = "v0.17.0"
RC_TAG = "v0.17.0-rc.1"
REPO = "FelipeGazapina/pinned"
LEG = "aarch64-apple-darwin"
ARM_NAME = "pinned-server-aarch64-unknown-linux-musl"
X86_NAME = "pinned-server-x86_64-unknown-linux-musl"
BUNDLE = "pinned_aarch64.app.tar.gz"
EARLIER_ATTEMPT_ARTIFACTS = [f"updater-{LEG}", "linux-node-binaries"]
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
PLATFORMS = (
    "darwin-aarch64",
    "darwin-aarch64-app",
    "darwin-x86_64",
    "darwin-x86_64-app",
    "windows-x86_64",
    "windows-x86_64-nsis",
)
ARM = b"arm-old " * 2048
X86 = b"x86-old " * 2048
SUM_NAMES = ("latest.json", ARM_NAME, X86_NAME)
FULL_RUN = ["draft", "build", "linux-node", "sign-node", "publish"]
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
        state["prerelease"] = flag_bool(flags, "--prerelease") is True
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
        clobber = "--clobber" in flags
        for name, _data in prepared:
            if name in state["assets"] and not clobber:
                sys.stderr.write(f"asset under the same name already exists: [{name}]\n")
                return 1
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


PWSH = {"pwsh", "powershell", "pwsh.exe", "powershell.exe"}
BASH_SYNTAX = re.compile(r"\$\{?[A-Za-z_]|\$(?=\()|\[\[")


def is_bash_syntax(script):
    without_pwsh_env = re.sub(r"\$env:[A-Za-z_][A-Za-z0-9_]*", "", script)
    return BASH_SYNTAX.search(without_pwsh_env) is not None


def shell_token(value):
    return value.strip().split()[0]


def runner_shell(label):
    lowered = label.lower()
    if "windows" in lowered:
        return "pwsh"
    if "ubuntu" in lowered or "macos" in lowered or "linux" in lowered:
        return "bash"
    return None


def effective_shells(workflow_shell, job, step):
    if step.get("shell"):
        return [shell_token(step["shell"])]
    if job.get("shell"):
        return [shell_token(job["shell"])]
    if workflow_shell:
        return [shell_token(workflow_shell)]
    shells = []
    for label in job.get("runs_on") or []:
        if "${{" in label:
            shells.append(None)
        else:
            shells.append(runner_shell(label))
    return shells or [None]


def shell_problems(workflow_shell, jobs):
    problems = []
    for name, job in jobs.items():
        for index, step in enumerate(job["steps"], start=1):
            script = step.get("run") or ""
            if not script.strip() or not is_bash_syntax(script):
                continue
            bad = []
            for shell in effective_shells(workflow_shell, job, step):
                if shell in PWSH or shell is None:
                    shown = shell or "unresolved"
                    if shown not in bad:
                        bad.append(shown)
            if not bad:
                continue
            first = next((item.strip() for item in script.splitlines() if item.strip()), "")
            problems.append(
                f"{name} step {index} runs bash syntax under {', '.join(bad)}: {first}"
            )
    return problems


def indent_of(line):
    return len(line) - len(line.lstrip(" "))


def read_map(lines, i, parent_indent):
    values = {}
    key_indent = None
    while i < len(lines):
        line = lines[i]
        if not line.strip() or line.lstrip().startswith("#"):
            i += 1
            continue
        indent = indent_of(line)
        if indent <= parent_indent:
            break
        if key_indent is None:
            key_indent = indent
        if indent == key_indent:
            key, _, value = line.strip().partition(":")
            values[key.strip()] = value.strip().strip("'\"")
        i += 1
    return values, i


def parse_workflow(text):
    lines = text.splitlines()
    jobs = {}
    current = None
    in_jobs = False
    in_steps = False
    step = None
    workflow_shell = None
    workflow_env = {}
    seeing_defaults = False
    i = 0
    while i < len(lines):
        line = lines[i]
        if line.startswith("jobs:"):
            in_jobs = True
            i += 1
            continue
        if not in_jobs:
            stripped = line.strip()
            if line.rstrip() == "env:":
                workflow_env, i = read_map(lines, i + 1, 0)
                continue
            if line.startswith("defaults:"):
                seeing_defaults = True
            elif seeing_defaults and stripped.startswith("shell:"):
                workflow_shell = stripped.split(":", 1)[1].strip().strip("'\"")
            elif line and not line.startswith(" ") and not line.startswith("#"):
                seeing_defaults = False
            i += 1
            continue
        if line and not line.startswith(" ") and not line.startswith("#"):
            break
        if line.startswith("  ") and not line.startswith("   ") and line.rstrip().endswith(":"):
            name = line.strip()[:-1]
            current = {"steps": [], "runs_on": [], "shell": None, "env": {}}
            jobs[name] = current
            in_steps = False
            step = None
            i += 1
            continue
        if current is not None and line.strip() == "steps:":
            in_steps = True
            i += 1
            continue
        if current is not None and not in_steps:
            stripped = line.strip()
            if stripped == "env:":
                current["env"], i = read_map(lines, i + 1, indent_of(line))
                continue
            if stripped.startswith("runs-on:"):
                value = stripped.split(":", 1)[1].strip().strip("'\"")
                if "${{" not in value:
                    current["runs_on"] = [value]
                i += 1
                continue
            if stripped.startswith("os:") or stripped.startswith("- os:"):
                current["runs_on"].append(stripped.split(":", 1)[1].strip().strip("'\""))
                i += 1
                continue
            if stripped.startswith("shell:"):
                current["shell"] = stripped.split(":", 1)[1].strip().strip("'\"")
                i += 1
                continue
        if in_steps and line.startswith("      - "):
            step = {"uses": None, "run": None, "shell": None, "env": {}, "with": {}}
            current["steps"].append(step)
            rest = line.strip()[2:]
            if rest.startswith("uses:"):
                step["uses"] = rest.split(":", 1)[1].strip()
            i += 1
            continue
        if step is None:
            i += 1
            continue
        stripped = line.strip()
        if stripped in ("env:", "with:"):
            step[stripped[:-1]], i = read_map(lines, i + 1, indent_of(line))
            continue
        if stripped.startswith("uses:"):
            step["uses"] = stripped.split(":", 1)[1].strip()
            i += 1
            continue
        if stripped.startswith("shell:"):
            step["shell"] = stripped.split(":", 1)[1].strip().strip("'\"")
            i += 1
            continue
        if stripped.startswith("run:"):
            value = stripped.split(":", 1)[1].strip()
            if value in {"|", "|-", ">", ">-"}:
                run_indent = indent_of(line)
                body = []
                content_indent = None
                i += 1
                while i < len(lines):
                    nxt = lines[i]
                    if nxt.strip() == "":
                        body.append("")
                        i += 1
                        continue
                    indent = indent_of(nxt)
                    if indent <= run_indent:
                        break
                    if content_indent is None:
                        content_indent = indent
                    body.append(nxt[content_indent:])
                    i += 1
                step["run"] = "\n".join(body) + "\n"
                continue
            step["run"] = value + "\n"
            i += 1
            continue
        i += 1
    return workflow_shell, workflow_env, jobs


def release_step(step):
    uses = step.get("uses") or ""
    if "tauri-action" in uses:
        return True
    run = step.get("run") or ""
    return any(
        needle in run
        for needle in (
            "gh release",
            "scripts/assemble-latest-json.sh",
            "scripts/sign-node-sums.sh",
            "scripts/release-asset.sh",
        )
    )


def is_checkout(step):
    return (step.get("uses") or "").startswith("actions/checkout@")


def is_artifact_upload(step):
    return (step.get("uses") or "").startswith("actions/upload-artifact@")


def replayed(step):
    return is_artifact_upload(step) or release_step(step)


def expand(value, tag):
    known = {"github.repository": REPO, "github.ref_name": tag, "matrix.target": LEG}

    def one(match):
        expr = match.group(1)
        if expr in known:
            return known[expr]
        if expr.startswith("secrets."):
            return "replay"
        raise SystemExit(f"the replay cannot expand ${{{{ {expr} }}}}")

    return re.sub(r"\$\{\{\s*(.*?)\s*\}\}", one, value)


def expanded(values, tag):
    return {key: expand(value, tag) for key, value in values.items()}


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


def bundle_bytes(build):
    return f"desktop bundle {build}\n".encode() * 2048


def fragment(signature, tag):
    url = f"https://github.com/{REPO}/releases/download/{tag}/{BUNDLE}"
    platforms = {key: {"signature": signature, "url": url} for key in PLATFORMS}
    return {"version": tag[1:], "platforms": platforms}


def manifest(signature, notes, tag):
    body = {
        "version": tag[1:],
        "notes": notes,
        "pub_date": "2026-10-30T00:00:00.000Z",
        "platforms": fragment(signature, tag)["platforms"],
    }
    return json.dumps(body, indent=2).encode()


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
        "fragment": None,
        "artifacts": [],
    }


def seeded_state(release, draft, replace=None):
    files, built = release
    files = {**files, **(replace or {})}
    state = empty_state()
    state["exists"] = True
    state["draft"] = draft
    state["fragment"] = built
    state["assets"] = {name: b64(data) for name, data in files.items()}
    state["artifacts"] = list(EARLIER_ATTEMPT_ARTIFACTS)
    return state


def write_fragment(work, built):
    path = work / "updater-fragments" / f"updater-{LEG}" / "updater-fragment.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(built))


def prepare_work(work, state):
    node = work / "node-release"
    node.mkdir(parents=True)
    (node / ARM_NAME).write_bytes(ARM)
    (node / X86_NAME).write_bytes(X86)
    if state["fragment"]:
        write_fragment(work, state["fragment"])


class Replay:
    def __init__(self, workflow_env, jobs, root):
        self.workflow_env = workflow_env
        self.jobs = jobs
        self.root = pathlib.Path(root)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name, source in (("gh", GH_SOURCE), ("npx", NPX_SOURCE)):
            path = self.bin / name
            path.write_text(source)
            path.chmod(0o755)
        self.signer = self.root / "signer.mjs"
        self.signer.write_text(SIGNER_SOURCE)
        self.key = self.root / "key.json"
        self.forger = self.root / "forger.json"
        for key in (self.key, self.forger):
            subprocess.run(["node", str(self.signer), "generate", str(key)], check=True)
        self.pubkey = json.loads(self.key.read_text())["pubkey"]
        self.forged_pubkey = json.loads(self.forger.read_text())["pubkey"]

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
        bundle = bundle_bytes(build)
        signature = self.tauri_signature(bundle, BUNDLE, key)
        latest = manifest(signature, notes, tag)
        sums = sums_for(latest, ARM, X86)
        files = {
            BUNDLE: bundle,
            f"{BUNDLE}.sig": signature.encode(),
            "latest.json": latest,
            ARM_NAME: ARM,
            X86_NAME: X86,
            "SHA256SUMS": sums,
            "SHA256SUMS.minisig": self.minisig(sums, "SHA256SUMS", key),
        }
        return files, fragment(signature, tag)

    def run(self, job_names, state, extra_env, tag):
        state_path = self.root / "state.json"
        state_path.write_text(json.dumps(state))
        work = self.root / "work"
        if work.exists():
            shutil.rmtree(work)
        work.mkdir()
        prepare_work(work, state)
        env = runner_env()
        env["PATH"] = str(self.bin) + os.pathsep + env.get("PATH", "")
        env["GITHUB_REF_NAME"] = tag
        env["GITHUB_REPOSITORY"] = REPO
        env["REPLAY_ROOT"] = str(self.root)
        env["REPLAY_REPO"] = REPO
        env["REPLAY_STATE"] = str(state_path)
        env["REPLAY_SIGNER"] = str(self.signer)
        env["REPLAY_KEY"] = str(self.key)
        env.update(expanded(self.workflow_env, tag))
        stderr = []
        code = 0
        if isinstance(job_names, str):
            self.checkout(work, True)
            code, err = self.shell(job_names, {**env, **extra_env}, work)
            stderr.append(err)
        else:
            for name in job_names:
                code, err = self.job(name, env, extra_env, work, tag)
                stderr.append(err)
                if code != 0:
                    break
        state = json.loads(state_path.read_text())
        assembled = work / "node-release" / "latest.json"
        return code, "".join(stderr), state, assembled.read_bytes() if assembled.exists() else None

    def checkout(self, work, present):
        for name in (".git", "scripts", "tauri.conf.json"):
            path = work / name
            if path.is_dir():
                shutil.rmtree(path)
            elif path.exists():
                path.unlink()
        if not present:
            return
        (work / ".git").mkdir()
        shutil.copytree(ROOT / "scripts", work / "scripts")
        conf = {"plugins": {"updater": {"pubkey": self.pubkey}}}
        (work / "tauri.conf.json").write_text(json.dumps(conf))

    def job(self, name, runner, extra_env, work, tag):
        job = self.jobs[name]
        self.checkout(work, any(is_checkout(step) for step in job["steps"]))
        job_env = {**runner, **expanded(job["env"], tag)}
        stderr = []
        for step in job["steps"]:
            if not replayed(step):
                continue
            env = {**job_env, **expanded(step["env"], tag), **extra_env}
            if is_artifact_upload(step):
                code, err = self.upload_artifact(step, env, tag)
            elif "tauri-action" in (step.get("uses") or ""):
                code, err = self.tauri(step, env, work, tag)
            else:
                code, err = self.shell(step["run"], env, work)
            stderr.append(err)
            if code != 0:
                return code, "".join(stderr)
        return 0, "".join(stderr)

    def shell(self, script, env, work):
        path = work / "step.sh"
        path.write_text(script)
        proc = subprocess.run(
            ["bash", "--noprofile", "--norc", "-eo", "pipefail", str(path)],
            cwd=work,
            env=env,
            text=True,
            capture_output=True,
        )
        return proc.returncode, proc.stderr

    def upload_artifact(self, step, env, tag):
        state_path = pathlib.Path(env["REPLAY_STATE"])
        state = json.loads(state_path.read_text())
        name = expand(step["with"]["name"], tag)
        if name in state["artifacts"] and step["with"].get("overwrite") != "true":
            return 1, (
                "Failed to CreateArtifact: Received non-retryable error: Failed request: (409) Conflict: "
                f"an artifact with this name already exists on the workflow run: {name}\n"
            )
        if name not in state["artifacts"]:
            state["artifacts"].append(name)
        state_path.write_text(json.dumps(state))
        return 0, ""

    def tauri(self, step, env, work, tag):
        state_path = pathlib.Path(env["REPLAY_STATE"])
        build = json.loads(state_path.read_text())["builds"] + 1
        out = work / "tauri-out"
        out.mkdir(exist_ok=True)
        bundle = out / BUNDLE
        bundle.write_bytes(bundle_bytes(build))
        signature = self.tauri_signature(bundle.read_bytes(), BUNDLE)
        (out / f"{BUNDLE}.sig").write_text(signature)
        action_env = {**env, "GH_REPO": REPO}
        code, err = self.shell(
            f'gh release upload "$GITHUB_REF_NAME" "{bundle}" "{bundle}.sig" --clobber\n',
            action_env,
            work,
        )
        if code != 0:
            return code, err
        state = json.loads(state_path.read_text())
        state["builds"] = build
        state["fragment"] = fragment(signature, tag)
        state_path.write_text(json.dumps(state))
        write_fragment(work, state["fragment"])
        flag = (step["with"].get("includeUpdaterJson") or "true").lower()
        if flag == "false":
            return code, err
        latest = out / "latest.json"
        latest.write_bytes(json.dumps({"version": tag[1:], "pub_date": "tauri"}).encode())
        code2, err2 = self.shell(
            f'gh release upload "$GITHUB_REF_NAME" "{latest}" --clobber\n',
            action_env,
            work,
        )
        return code2, err + err2


def step_labels(job):
    labels = []
    for step in job["steps"]:
        if not replayed(step):
            continue
        if step.get("uses"):
            shown = ("includeUpdaterJson", "name", "overwrite")
            extra = "".join(f" {key}={step['with'][key]}" for key in shown if key in step["with"])
            labels.append(step["uses"] + extra)
            continue
        line = next((item.strip() for item in step["run"].splitlines() if item.strip()), "")
        labels.append(line)
    return labels


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
    state = seeded_state((old, None), draft=False)
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


def main():
    workflow_shell, workflow_env, jobs = parse_workflow(WORKFLOW.read_text())
    shell_failed = 0
    problems = shell_problems(workflow_shell, jobs)
    if problems:
        shell_failed = len(problems)
        for problem in problems:
            print(f"FAIL shell: {problem}")
    else:
        print("ok: shells")
    for name in FULL_RUN:
        if name not in jobs:
            sys.exit(f"release.yml has no {name} job")
        if name != "linux-node" and not any(release_step(step) for step in jobs[name]["steps"]):
            sys.exit(f"{name} has no release steps")
    sha = subprocess.check_output(
        ["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, text=True
    ).strip()
    print(f"replay at {sha}")
    for name in FULL_RUN:
        print(f"  {name}: {' | '.join(step_labels(jobs[name])) or '(no release write)'}")

    failed = report("version order", version_order_errors())
    with tempfile.TemporaryDirectory(prefix="release-replay-") as tmp:
        replay = Replay(workflow_env, jobs, tmp)
        failed += report("minisign-verify crate vectors", crate_vector_errors(replay.root))
        failed += report("public clobber", public_clobber_errors(replay))

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
            [(FULL_RUN, {})],
            empty_state(),
            {"code": 0, "draft": False, "prerelease": False, "latest_flag": True, "consistent": True, "latest_assembled": True},
        )
        add(
            "full run, then re-run all jobs",
            [(FULL_RUN, {}), (FULL_RUN, {})],
            empty_state(),
            {"code": 0, "draft": False, "consistent": True, "latest_assembled": True},
        )
        add(
            "full run of a release candidate",
            [(FULL_RUN, {})],
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
                "uploaded": BUNDLE,
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
                "stderr_has": f"{BUNDLE} does not match its signature in latest.json",
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
            [(["build"], {"REPLAY_UPLOAD_CANCEL": json.dumps({BUNDLE: missing})}), (["publish"], {})],
            public(),
            {"draft": True, "stderr_has": f"{BUNDLE} is missing from the release"},
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
                stray_latest + f'bash scripts/release-asset.sh upload "$GITHUB_REF_NAME" node-release/latest.json node-release/{ARM_NAME}\n',
                {"GH_TOKEN": "replay"},
            )],
            public(),
            {"draft": False, "stderr_has": "SHA256SUMS is missing from the upload", "unchanged_from": public()},
        )
        add(
            "upload latest.json with SHA256SUMS that does not list it",
            [(
                stray_latest
                + "printf '%064d  latest.json\\n' 0 > node-release/SHA256SUMS\n"
                + 'bash scripts/release-asset.sh upload "$GITHUB_REF_NAME" node-release/SHA256SUMS node-release/latest.json\n',
                {"GH_TOKEN": "replay"},
            )],
            public(),
            {"draft": False, "stderr_has": "SHA256SUMS does not list this latest.json", "unchanged_from": public()},
        )

        for title, runs, state, expect, tag in cases:
            code, stderr, after, assembled = 0, "", state, None
            for job_names, extra_env in runs:
                carried = {**after, "violations": [], "uploads": [], "calls": []}
                code, stderr, after, assembled = replay.run(job_names, carried, extra_env, tag)
                if after["violations"]:
                    break
            failed += report(title, scenario_errors(code, stderr, after, assembled, expect))
    if failed:
        print(f"{failed} check(s) failed")
    else:
        print(f"ok: {len(cases)} scenarios")
    if failed or shell_failed:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
