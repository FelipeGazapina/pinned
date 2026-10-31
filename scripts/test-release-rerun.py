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
from datetime import datetime, timezone

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
TAG = "v0.17.0"
REPO = "FelipeGazapina/pinned"
NOTES = (
    "On a Mac, install the .dmg. On Windows, run the _x64-setup.exe. "
    "Or click the update button in a running pinned."
)
ARM_NAME = "pinned-server-aarch64-unknown-linux-musl"
X86_NAME = "pinned-server-x86_64-unknown-linux-musl"
BUNDLE = "pinned_aarch64.app.tar.gz"
BUNDLE_URL = f"https://github.com/{REPO}/releases/download/{TAG}/{BUNDLE}"
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
STAMP_EPOCH = 1700000000
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

def next_stamp(state):
    import datetime
    state["clock"] = int(state.get("clock") or 0) + 1
    moment = datetime.datetime.fromtimestamp(1700000000 + state["clock"], datetime.timezone.utc)
    return moment.strftime("%Y-%m-%dT%H:%M:%SZ")

def record(state, gap):
    if gap:
        state["violations"].append(gap)

def main():
    argv = sys.argv[1:]
    if len(argv) < 2 or argv[0] != "release":
        sys.stderr.write("fake gh: " + " ".join(sys.argv) + "\n")
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
            fields = (flags.get("--json") or [""])[0]
            if "assets" in [item.strip() for item in fields.split(",")]:
                updated = state.get("updated") or {}
                rows = [
                    {"name": name, "updatedAt": updated.get(name, "")}
                    for name in state["assets"]
                ]
                print(json.dumps({"assets": rows}))
                return 0
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
        if draft is not None:
            state["draft"] = draft
        latest = flag_bool(flags, "--latest")
        if latest is None and was_draft and not state["draft"]:
            latest = True
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
        state.setdefault("updated", {})
        if "--clobber" in flags:
            for name, _data in prepared:
                if name not in state["assets"]:
                    continue
                if not state["draft"]:
                    state["violations"].append(f"delete {name} while the release is public")
                del state["assets"][name]
                state["updated"].pop(name, None)
                record(state, public_mismatch(state))
                save(state)
        lands = os.environ.get("REPLAY_UPLOAD_LANDS")
        for name, data in sorted(prepared, key=lambda item: len(item[1])):
            if lands is not None and name not in lands.split(","):
                continue
            if not state["draft"]:
                state["violations"].append(f"upload {name} while the release is public")
            state["assets"][name] = base64.b64encode(data).decode()
            state["updated"][name] = next_stamp(state)
            state["uploads"].append({"name": name, "draft": bool(state["draft"])})
            record(state, public_mismatch(state))
            save(state)
        if lands is not None:
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


def parse_workflow(text):
    lines = text.splitlines()
    jobs = {}
    current = None
    in_jobs = False
    in_steps = False
    step = None
    workflow_shell = None
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
            current = {"steps": [], "runs_on": [], "shell": None}
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
            step = {"uses": None, "run": None, "includeUpdaterJson": None, "shell": None}
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
        if stripped.startswith("uses:"):
            step["uses"] = stripped.split(":", 1)[1].strip()
            i += 1
            continue
        if stripped.startswith("shell:"):
            step["shell"] = stripped.split(":", 1)[1].strip().strip("'\"")
            i += 1
            continue
        if stripped.startswith("includeUpdaterJson:"):
            step["includeUpdaterJson"] = stripped.split(":", 1)[1].strip().strip("'\"")
            i += 1
            continue
        if stripped.startswith("run:"):
            value = stripped.split(":", 1)[1].strip()
            if value in {"|", "|-", ">", ">-"}:
                run_indent = len(line) - len(line.lstrip(" "))
                body = []
                content_indent = None
                i += 1
                while i < len(lines):
                    nxt = lines[i]
                    if nxt.strip() == "":
                        body.append("")
                        i += 1
                        continue
                    indent = len(nxt) - len(nxt.lstrip(" "))
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
    return workflow_shell, jobs


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


def asset_stamp(counter):
    return datetime.fromtimestamp(STAMP_EPOCH + counter, timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def bundle_bytes(build):
    return f"desktop bundle {build}\n".encode() * 2048


def fragment(signature):
    platforms = {key: {"signature": signature, "url": BUNDLE_URL} for key in PLATFORMS}
    return {"version": TAG[1:], "platforms": platforms}


def manifest(signature, notes):
    body = {
        "version": TAG[1:],
        "notes": notes,
        "pub_date": "2026-10-30T00:00:00.000Z",
        "platforms": fragment(signature)["platforms"],
    }
    return json.dumps(body, indent=2).encode()


def empty_state():
    return {
        "exists": False,
        "draft": False,
        "latest": False,
        "assets": {},
        "updated": {},
        "clock": 0,
        "violations": [],
        "uploads": [],
        "calls": [],
        "remote_latest": None,
        "builds": 0,
        "fragment": None,
    }


def seeded_state(release, draft, replace=None):
    files, built = release
    files = {**files, **(replace or {})}
    state = empty_state()
    state["exists"] = True
    state["draft"] = draft
    state["fragment"] = built
    state["assets"] = {name: b64(data) for name, data in files.items()}
    state["updated"] = {name: asset_stamp(0) for name in files}
    return state


def rewrite(run):
    replaced = []
    for line in run.splitlines(keepends=True):
        stripped = line.lstrip()
        indent = line[: len(line) - len(stripped)]
        if stripped.startswith("bash scripts/"):
            rest = stripped[len("bash scripts/") :]
            name, _, tail = rest.partition(" ")
            line = f'{indent}bash "{ROOT / "scripts" / name}" {tail}'
            if not line.endswith("\n"):
                line += "\n"
        replaced.append(line)
    return "".join(replaced)


def write_fragment(work, built):
    path = work / "updater-fragments" / "updater-aarch64-apple-darwin" / "updater-fragment.json"
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
    def __init__(self, jobs, root):
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
        subprocess.run(["node", str(self.signer), "generate", str(self.key)], check=True)
        self.pubkey = json.loads(self.key.read_text())["pubkey"]

    def minisig(self, data, name):
        scratch = self.root / "to-sign"
        if scratch.exists():
            shutil.rmtree(scratch)
        scratch.mkdir()
        path = scratch / name
        path.write_bytes(data)
        proc = subprocess.run(
            ["node", str(self.signer), "minisig", str(self.key), str(path)],
            check=True,
            capture_output=True,
        )
        return proc.stdout

    def tauri_signature(self, data, name):
        return b64(self.minisig(data, name))

    def release(self, build=0, notes="old"):
        bundle = bundle_bytes(build)
        signature = self.tauri_signature(bundle, BUNDLE)
        latest = manifest(signature, notes)
        sums = sums_for(latest, ARM, X86)
        files = {
            BUNDLE: bundle,
            f"{BUNDLE}.sig": signature.encode(),
            "latest.json": latest,
            ARM_NAME: ARM,
            X86_NAME: X86,
            "SHA256SUMS": sums,
            "SHA256SUMS.minisig": self.minisig(sums, "SHA256SUMS"),
        }
        return files, fragment(signature)

    def run(self, job_names, state, extra_env):
        state_path = self.root / "state.json"
        state_path.write_text(json.dumps(state))
        work = self.root / "work"
        if work.exists():
            shutil.rmtree(work)
        work.mkdir()
        prepare_work(work, state)
        env = os.environ.copy()
        env["PATH"] = str(self.bin) + os.pathsep + env.get("PATH", "")
        env["GITHUB_REF_NAME"] = TAG
        env["GITHUB_REPOSITORY"] = REPO
        env["RELEASE_NOTES"] = NOTES
        env["GH_TOKEN"] = "replay"
        env["REPLAY_STATE"] = str(state_path)
        env["REPLAY_SIGNER"] = str(self.signer)
        env["REPLAY_KEY"] = str(self.key)
        env["PINNED_RELEASE_PUBKEY"] = self.pubkey
        env["TAURI_SIGNING_PRIVATE_KEY"] = "replay"
        env.update(extra_env)
        stderr = []
        code = 0
        for name in job_names:
            code, err = self.job(name, env, work)
            if err.strip():
                stderr.append(err)
            if code != 0:
                break
        state = json.loads(state_path.read_text())
        assembled = work / "node-release" / "latest.json"
        return code, "".join(stderr), state, assembled.read_bytes() if assembled.exists() else None

    def job(self, name, env, work):
        stderr = []
        for step in self.jobs[name]["steps"]:
            if not release_step(step):
                continue
            if step.get("uses") and "tauri-action" in step["uses"]:
                code, err = self.tauri(step, env, work)
            else:
                code, err = self.shell(rewrite(step["run"]), env, work)
            if err.strip():
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

    def tauri(self, step, env, work):
        state_path = pathlib.Path(env["REPLAY_STATE"])
        build = json.loads(state_path.read_text())["builds"] + 1
        out = work / "tauri-out"
        out.mkdir(exist_ok=True)
        bundle = out / BUNDLE
        bundle.write_bytes(bundle_bytes(build))
        signature = self.tauri_signature(bundle.read_bytes(), BUNDLE)
        (out / f"{BUNDLE}.sig").write_text(signature)
        code, err = self.shell(
            f'gh release upload "$GITHUB_REF_NAME" "{bundle}" "{bundle}.sig" --clobber\n',
            env,
            work,
        )
        if code != 0:
            return code, err
        state = json.loads(state_path.read_text())
        state["builds"] = build
        state["fragment"] = fragment(signature)
        state_path.write_text(json.dumps(state))
        write_fragment(work, state["fragment"])
        flag = (step.get("includeUpdaterJson") or "true").lower()
        if flag == "false":
            return code, err
        latest = out / "latest.json"
        latest.write_bytes(b'{"version":"0.17.0","pub_date":"tauri"}\n')
        code2, err2 = self.shell(
            f'gh release upload "$GITHUB_REF_NAME" "{latest}" --clobber\n',
            env,
            work,
        )
        return code2, err + err2


def step_labels(job):
    labels = []
    for step in job["steps"]:
        if not release_step(step):
            continue
        if step.get("uses"):
            flag = step.get("includeUpdaterJson")
            extra = f" includeUpdaterJson={flag}" if flag else ""
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
        elif key == "latest_flag":
            if bool(state["latest"]) != value:
                errors.append(f"latest={state['latest']}, want {value}")
        elif key == "unchanged_from":
            if not unchanged(value, state):
                errors.append("signer failure replaced latest.json or SHA256SUMS")
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
    ):
        message_path = root / f"vector-{label}"
        signature_path = root / f"vector-{label}.minisig"
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


def public_clobber_errors(replay):
    state = seeded_state(replay.release(), draft=False)
    state_path = replay.root / "clobber-state.json"
    state_path.write_text(json.dumps(state))
    work = replay.root / "clobber-work"
    work.mkdir()
    newer, _ = replay.release(notes="new")
    latest = work / "latest.json"
    sums = work / "SHA256SUMS"
    latest.write_bytes(newer["latest.json"])
    sums.write_bytes(newer["SHA256SUMS"])
    env = os.environ.copy()
    env["REPLAY_STATE"] = str(state_path)
    env["GITHUB_REF_NAME"] = TAG
    proc = subprocess.run(
        [str(replay.bin / "gh"), "release", "upload", TAG, str(latest), str(sums), "--clobber"],
        env=env,
        text=True,
        capture_output=True,
    )
    after = json.loads(state_path.read_text())
    errors = []
    if proc.returncode != 0:
        errors.append(f"clobber upload exit {proc.returncode}: {proc.stderr.strip()}")
    if not any("while the release is public" in item for item in after["violations"]):
        errors.append(f"public clobber recorded no violation: {after['violations']}")
    if asset_bytes(after, "latest.json") != newer["latest.json"]:
        errors.append("clobber did not replace latest.json")
    names = [item["name"] for item in after["uploads"]]
    if names != ["SHA256SUMS", "latest.json"]:
        errors.append(f"clobber upload landed {names}, want the smaller SHA256SUMS first")
    stamps = [after["updated"][name] for name in ("SHA256SUMS", "latest.json")]
    if stamps[0] >= stamps[1]:
        errors.append(f"clobber upload stamps {stamps} do not follow completion order")
    if not consistent(after):
        errors.append("clobber did not finish consistent")
    return errors


def main():
    workflow_shell, jobs = parse_workflow(WORKFLOW.read_text())
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

    failed = 0
    with tempfile.TemporaryDirectory(prefix="release-replay-") as tmp:
        replay = Replay(jobs, tmp)
        vector_errors = crate_vector_errors(replay.root)
        if vector_errors:
            failed += 1
            print("FAIL minisign-verify crate vectors")
            for error in vector_errors:
                print(f"  {error}")
        else:
            print("ok: minisign-verify crate vectors")
        clobber_errors = public_clobber_errors(replay)
        if clobber_errors:
            failed += 1
            print("FAIL public clobber")
            for error in clobber_errors:
                print(f"  {error}")
        else:
            print("ok: public clobber")

        current = replay.release()
        newer = replay.release(notes="new")

        def public():
            return seeded_state(current, draft=False)

        def drafted(remote_latest=None, replace=None, release=current):
            state = seeded_state(release, draft=True, replace=replace)
            state["remote_latest"] = remote_latest
            return state

        cases = []

        def add(title, runs, state, expect):
            cases.append((title, runs, state, expect))

        add(
            "full run, then the same run again",
            [(FULL_RUN, {}), (FULL_RUN, {})],
            empty_state(),
            {"code": 0, "draft": False, "consistent": True, "latest_assembled": True},
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
            drafted(remote_latest="v0.17.0-rc.1"),
            {"code": 0, "draft": False, "latest_flag": True, "consistent": True},
        )
        for landed, missing in (
            ([], "SHA256SUMS"),
            (["SHA256SUMS"], "SHA256SUMS.minisig"),
            (["SHA256SUMS", "SHA256SUMS.minisig"], "latest.json"),
            (["SHA256SUMS", "SHA256SUMS.minisig", "latest.json"], ARM_NAME),
            (["SHA256SUMS", "SHA256SUMS.minisig", "latest.json", X86_NAME], ARM_NAME),
        ):
            add(
                f"sign-node cancelled after {len(landed)} of 5 files land, then publish",
                [(["sign-node"], {"REPLAY_UPLOAD_LANDS": ",".join(landed)}), (["publish"], {})],
                public(),
                {"draft": True, "stderr_has": f"{missing} is missing from the release"},
            )
        add(
            "build re-run cancelled before the bundle lands, then publish",
            [(["build"], {"REPLAY_UPLOAD_LANDS": ""}), (["publish"], {})],
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

        for title, runs, state, expect in cases:
            code, stderr, after, assembled = 0, "", state, None
            for job_names, extra_env in runs:
                carried = {**after, "violations": [], "uploads": [], "calls": []}
                code, stderr, after, assembled = replay.run(job_names, carried, extra_env)
                if after["violations"]:
                    break
            errors = scenario_errors(code, stderr, after, assembled, expect)
            if errors:
                failed += 1
                print(f"FAIL {title}")
                for error in errors:
                    print(f"  {error}")
            else:
                print(f"ok: {title}")
    if failed:
        print(f"{failed} scenario(s) failed")
    else:
        print(f"ok: {len(cases)} scenarios")
    if failed or shell_failed:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
