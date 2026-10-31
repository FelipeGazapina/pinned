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
NOTES = (
    "On a Mac, install the .dmg. On Windows, run the _x64-setup.exe. "
    "Or click the update button in a running pinned."
)
ARM_NAME = "pinned-server-aarch64-unknown-linux-musl"
X86_NAME = "pinned-server-x86_64-unknown-linux-musl"
BUNDLE = "pinned_aarch64.app.tar.gz"
OLD_LATEST = b'{"version":"0.17.0","pub_date":"2026-10-30T00:00:00.000Z","notes":"old"}\n'
NEW_LATEST = b'{"version":"0.17.0","pub_date":"2026-10-31T18:00:00.000Z","notes":"replay"}\n'
ARM = b"arm-old"
X86 = b"x86-old"
SUM_NAMES = ("latest.json", ARM_NAME, X86_NAME)

GH_SOURCE = r'''#!/usr/bin/env python3
import base64, hashlib, json, os, pathlib, sys

STATE = pathlib.Path(os.environ["REPLAY_STATE"])

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
    for line in sums.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[0] == digest and parts[-1].lstrip("*") == "latest.json":
            return None
    return f"public latest.json {digest} missing from SHA256SUMS"

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
        draft = flag_bool(flags, "--draft")
        if draft is not None:
            state["draft"] = draft
        if flag_bool(flags, "--latest") is True:
            state["latest"] = True
            state["remote_latest"] = os.environ.get("GITHUB_REF_NAME")
        gap = public_mismatch(state)
        if gap:
            state["violations"].append(gap)
        save(state)
        return 0

    if sub == "upload":
        if not state["exists"]:
            sys.stderr.write("release not found\n")
            return 1
        if not positional:
            sys.stderr.write("no files to upload\n")
            return 1
        for raw in positional:
            path = pathlib.Path(raw)
            data = path.read_bytes()
            if not data:
                sys.stderr.write(f"{raw} is empty\n")
                return 1
            name = path.name
            if not state["draft"]:
                state["violations"].append(f"upload {name} while the release is public")
            state["assets"][name] = base64.b64encode(data).decode()
            state["uploads"].append({"name": name, "draft": bool(state["draft"])})
            gap = public_mismatch(state)
            if gap:
                state["violations"].append(gap)
            save(state)
        return 0

    if sub == "download":
        if not state["exists"]:
            sys.stderr.write("release not found\n")
            return 1
        dest = pathlib.Path((flags.get("-D") or flags.get("--dir") or ["."])[0])
        patterns = (flags.get("-p") or []) + (flags.get("--pattern") or [])
        dest.mkdir(parents=True, exist_ok=True)
        import fnmatch
        for pattern in patterns:
            hits = [name for name in state["assets"] if fnmatch.fnmatch(name, pattern)]
            if not hits:
                sys.stderr.write(f"no asset matches {pattern}\n")
                return 1
            for name in hits:
                (dest / name).write_bytes(base64.b64decode(state["assets"][name]))
        return 0

    sys.stderr.write("fake gh: unsupported " + " ".join(sys.argv) + "\n")
    return 2

if __name__ == "__main__":
    sys.exit(main())
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


def sums_for(latest, arm, x86):
    files = {"latest.json": latest, ARM_NAME: arm, X86_NAME: x86}
    lines = []
    for name in SUM_NAMES:
        digest = hashlib.sha256(files[name]).hexdigest()
        lines.append(f"{digest}  {name}")
    return ("\n".join(lines) + "\n").encode()


def b64(data):
    return base64.b64encode(data).decode()


def asset_bytes(state, name):
    raw = state["assets"].get(name)
    if raw is None:
        return None
    return base64.b64decode(raw)


def empty_state():
    return {
        "exists": False,
        "draft": False,
        "latest": False,
        "assets": {},
        "violations": [],
        "uploads": [],
        "calls": [],
        "remote_latest": None,
    }


def seeded_state(draft, latest_flag, latest, sums):
    state = empty_state()
    state["exists"] = True
    state["draft"] = draft
    state["latest"] = latest_flag
    state["assets"] = {
        "latest.json": b64(latest),
        "SHA256SUMS": b64(sums),
        "SHA256SUMS.minisig": b64(b"untrusted comment: old\nRUQold\n"),
        ARM_NAME: b64(ARM),
        X86_NAME: b64(X86),
    }
    return state


def write_stubs(directory):
    assemble = directory / "assemble.sh"
    assemble.write_text(
        "#!/bin/bash\n"
        "out=\n"
        "prev=\n"
        'for arg in "$@"; do\n'
        '  if [[ "$prev" == "--out" ]]; then out="$arg"; fi\n'
        '  prev="$arg"\n'
        "done\n"
        'test -n "$out"\n'
        "printf '%s' '" + NEW_LATEST.decode() + "' > \"$out\"\n"
    )
    sign = directory / "sign.sh"
    sign.write_text(
        "#!/bin/bash\n"
        "set -euo pipefail\n"
        'dir="${1:?}"\n'
        'if [[ "${REPLAY_SIGN_FAIL:-0}" == 1 ]]; then\n'
        '  echo "tauri signer sign failed" >&2\n'
        "  exit 1\n"
        "fi\n"
        'python3 - "$dir" << \'PY\'\n'
        "import hashlib, pathlib, sys\n"
        "d = pathlib.Path(sys.argv[1])\n"
        f"names = {list(SUM_NAMES)!r}\n"
        "lines = []\n"
        "for name in names:\n"
        "    digest = hashlib.sha256((d / name).read_bytes()).hexdigest()\n"
        "    lines.append(f'{digest}  {name}')\n"
        "(d / 'SHA256SUMS').write_text('\\n'.join(lines) + '\\n')\n"
        "(d / 'SHA256SUMS.minisig').write_text('untrusted comment: replay\\nRUQreplay\\n')\n"
        "PY\n"
    )
    assemble.chmod(0o755)
    sign.chmod(0o755)
    return assemble, sign


def rewrite(run, assemble, sign):
    run = run.replace("bash scripts/assemble-latest-json.sh", f'bash "{assemble}"')
    run = run.replace("bash scripts/sign-node-sums.sh", f'bash "{sign}"')
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


def prepare_work(work):
    node = work / "node-release"
    node.mkdir(parents=True)
    (node / ARM_NAME).write_bytes(ARM)
    (node / X86_NAME).write_bytes(X86)


class Replay:
    def __init__(self, jobs, root):
        self.jobs = jobs
        self.root = pathlib.Path(root)
        self.bin = self.root / "bin"
        self.stubs = self.root / "stubs"
        self.bin.mkdir()
        self.stubs.mkdir()
        gh = self.bin / "gh"
        gh.write_text(GH_SOURCE)
        gh.chmod(0o755)
        self.assemble, self.sign = write_stubs(self.stubs)

    def run(self, job_names, state, sign_fail):
        state_path = self.root / "state.json"
        state_path.write_text(json.dumps(state))
        work = self.root / "work"
        if work.exists():
            shutil.rmtree(work)
        work.mkdir()
        prepare_work(work)
        env = os.environ.copy()
        env["PATH"] = str(self.bin) + os.pathsep + env.get("PATH", "")
        env["GITHUB_REF_NAME"] = TAG
        env["GITHUB_REPOSITORY"] = "FelipeGazapina/pinned"
        env["RELEASE_NOTES"] = NOTES
        env["GH_TOKEN"] = "replay"
        env["REPLAY_STATE"] = str(state_path)
        env["REPLAY_SIGN_FAIL"] = "1" if sign_fail else "0"
        env["TAURI_SIGNING_PRIVATE_KEY"] = "replay"
        stderr = []
        code = 0
        for name in job_names:
            code, err = self.job(name, env, work)
            if err.strip():
                stderr.append(err)
            if code != 0:
                break
        state = json.loads(state_path.read_text())
        return code, "".join(stderr), state

    def job(self, name, env, work):
        stderr = []
        for step in self.jobs[name]["steps"]:
            if not release_step(step):
                continue
            if step.get("uses") and "tauri-action" in step["uses"]:
                code, err = self.tauri(step, env, work)
            else:
                script = rewrite(step["run"], self.assemble, self.sign)
                code, err = self.shell(script, env, work)
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
        out = work / "tauri-out"
        out.mkdir(exist_ok=True)
        bundle = out / BUNDLE
        bundle.write_bytes(b"desktop-bundle")
        code, err = self.shell(
            f'gh release upload "$GITHUB_REF_NAME" "{bundle}" --clobber\n',
            env,
            work,
        )
        flag = (step.get("includeUpdaterJson") or "true").lower()
        if code != 0 or flag == "false":
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


def scenario_errors(code, stderr, state, expect):
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
        elif key == "latest_bytes":
            if asset_bytes(state, "latest.json") != value:
                errors.append("latest.json bytes are not the assembled manifest")
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


def matching_public():
    return seeded_state(False, False, OLD_LATEST, sums_for(OLD_LATEST, ARM, X86))


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
    for name in ("draft", "build", "linux-node", "sign-node", "publish"):
        if name not in jobs:
            sys.exit(f"release.yml has no {name} job")
        if name != "linux-node" and not any(release_step(step) for step in jobs[name]["steps"]):
            sys.exit(f"{name} has no release steps")
    sha = subprocess.check_output(
        ["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, text=True
    ).strip()
    print(f"replay at {sha}")
    for name in ("draft", "build", "linux-node", "sign-node", "publish"):
        print(f"  {name}: {' | '.join(step_labels(jobs[name])) or '(no release write)'}")

    cases = []

    def add(title, job_names, state, sign_fail, expect, repeat=1):
        cases.append((title, job_names, state, sign_fail, expect, repeat))

    add(
        "full run, then the same run again",
        ["draft", "build", "linux-node", "sign-node", "publish"],
        empty_state(),
        False,
        {"code": 0, "draft": False, "consistent": True, "latest_bytes": NEW_LATEST},
        repeat=2,
    )
    add(
        "sign-node re-run, signer fails",
        ["sign-node"],
        matching_public(),
        True,
        {
            "stderr_has": "tauri signer sign failed",
            "unchanged_from": matching_public(),
            "consistent": True,
        },
    )
    add(
        "sign-node re-run, signer succeeds",
        ["sign-node"],
        matching_public(),
        False,
        {"code": 0, "draft": True, "consistent": True, "latest_bytes": NEW_LATEST},
    )
    add(
        "sign-node re-run then publish",
        ["sign-node", "publish"],
        matching_public(),
        False,
        {
            "code": 0,
            "draft": False,
            "latest_flag": True,
            "consistent": True,
            "latest_bytes": NEW_LATEST,
        },
    )
    add(
        "build re-run",
        ["build"],
        matching_public(),
        False,
        {
            "code": 0,
            "draft": True,
            "uploaded": BUNDLE,
            "unchanged_from": matching_public(),
            "consistent": True,
        },
    )
    add(
        "publish re-run",
        ["publish"],
        matching_public(),
        False,
        {
            "code": 0,
            "draft": False,
            "latest_flag": True,
            "consistent": True,
            "unchanged_from": matching_public(),
        },
    )
    mismatched = seeded_state(True, False, NEW_LATEST, sums_for(OLD_LATEST, ARM, X86))
    add(
        "publish re-run on a draft whose latest.json hash is not in SHA256SUMS",
        ["publish"],
        mismatched,
        False,
        {
            "draft": True,
            "stderr_has": "the release checksum does not match",
            "consistent": False,
        },
    )
    older_latest = matching_public()
    older_latest["remote_latest"] = "v0.18.0"
    add(
        "publish an older tag does not take latest",
        ["publish"],
        older_latest,
        False,
        {
            "code": 0,
            "draft": False,
            "latest_flag": False,
            "consistent": True,
            "unchanged_from": matching_public(),
        },
    )
    newer_latest = matching_public()
    newer_latest["remote_latest"] = "v0.16.0"
    add(
        "publish a newer tag takes latest",
        ["publish"],
        newer_latest,
        False,
        {
            "code": 0,
            "draft": False,
            "latest_flag": True,
            "consistent": True,
            "unchanged_from": matching_public(),
        },
    )
    add(
        "linux-node re-run",
        ["linux-node"],
        matching_public(),
        False,
        {
            "code": 0,
            "draft": False,
            "consistent": True,
            "unchanged_from": matching_public(),
        },
    )

    failed = 0
    with tempfile.TemporaryDirectory(prefix="release-replay-") as tmp:
        replay = Replay(jobs, tmp)
        for title, job_names, state, sign_fail, expect, repeat in cases:
            code, stderr, after = 0, "", state
            for _ in range(repeat):
                carried = {
                    "exists": after["exists"],
                    "draft": after["draft"],
                    "latest": after["latest"],
                    "assets": after["assets"],
                    "violations": [],
                    "uploads": [],
                    "calls": [],
                    "remote_latest": after.get("remote_latest"),
                }
                code, stderr, after = replay.run(job_names, carried, sign_fail)
                if after["violations"]:
                    break
            errors = scenario_errors(code, stderr, after, expect)
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
