#!/usr/bin/env python3

from __future__ import annotations

import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

import workflow

ROOT = pathlib.Path(__file__).resolve().parent.parent
HUB_DEPLOY = ROOT / ".github" / "workflows" / "hub-deploy.yml"
SNAPSHOTS = "crates/core/wire-contract"

FAKE_CARGO = r'''#!/bin/bash
set -euo pipefail
if [[ "$1" == update && -n "${RACE:-}" ]]; then
  bash -c "$RACE"
fi
if [[ "$1" == run && "$*" == *" snapshot "* ]]; then
  cp crates/core/wire-contract/current.json "crates/core/wire-contract/${!#}.json"
fi
'''


def git_env(root):
    config = root / "gitconfig"
    config.write_text("")
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(
        {
            "GIT_CONFIG_GLOBAL": str(config),
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_AUTHOR_NAME": "replay",
            "GIT_AUTHOR_EMAIL": "replay@example.com",
            "GIT_COMMITTER_NAME": "replay",
            "GIT_COMMITTER_EMAIL": "replay@example.com",
        }
    )
    return env


class Remote:
    def __init__(self, root):
        self.root = root
        self.env = git_env(root)
        self.bare = root / "remote.git"
        self.git(root, "init", "-q", "--bare", "-b", "main", str(self.bare))

    def git(self, cwd, *args):
        proc = subprocess.run(["git", *args], cwd=cwd, env=self.env, text=True, capture_output=True)
        if proc.returncode != 0:
            raise SystemExit(f"git {' '.join(args)} in {cwd}: {proc.stderr.strip()}")
        return proc.stdout.strip()

    def clone(self, name):
        path = self.root / name
        self.git(self.root, "clone", "-q", str(self.bare), str(path))
        return path

    def commit(self, clone, subject):
        with (clone / "log.txt").open("a") as log:
            log.write(subject + "\n")
        self.git(clone, "add", "log.txt")
        self.git(clone, "commit", "-q", "-m", subject)
        return self.git(clone, "rev-parse", "HEAD")

    def refs(self):
        listed = self.git(self.root, "ls-remote", str(self.bare))
        return {ref: sha for sha, ref in (line.split("\t") for line in listed.splitlines())}


class ReleaseSandbox(Remote):
    def __init__(self, root):
        super().__init__(root)
        seed = root / "seed"
        seed.mkdir()
        self.git(seed, "init", "-q", "-b", "main")
        (seed / "scripts").mkdir()
        shutil.copy2(ROOT / "scripts" / "release.sh", seed / "scripts" / "release.sh")
        shutil.copy2(ROOT / "Cargo.toml", seed / "Cargo.toml")
        (seed / SNAPSHOTS).mkdir(parents=True)
        shutil.copy2(ROOT / SNAPSHOTS / "current.json", seed / SNAPSHOTS / "current.json")
        self.git(seed, "add", ".")
        self.git(seed, "commit", "-q", "-m", "start")
        self.git(seed, "push", "-q", str(self.bare), "main")
        self.start = self.git(seed, "rev-parse", "HEAD")
        self.work = self.clone("work")
        self.other = self.clone("other")
        self.bin = root / "bin"
        self.bin.mkdir()
        cargo = self.bin / "cargo"
        cargo.write_text(FAKE_CARGO)
        cargo.chmod(0o755)

    def parallel_push(self):
        return f'cd "{self.other}" && git pull -q --ff-only && echo parallel >> log.txt && git add log.txt && git commit -qm parallel && git push -q origin main'

    def label(self, sha):
        if sha == self.start:
            return "start"
        for clone in (self.work, self.other):
            proc = subprocess.run(
                ["git", "log", "-1", "--format=%s%n%P", sha], cwd=clone, env=self.env, text=True, capture_output=True
            )
            if proc.returncode == 0:
                subject, parent = proc.stdout.split("\n")[:2]
                return f"{subject} on {self.label(parent)}"
        raise SystemExit(f"no clone has {sha}")

    def release(self, version, race=None):
        env = {**self.env, "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}"}
        if race:
            env["RACE"] = race
        return subprocess.run(
            ["bash", "scripts/release.sh", version], cwd=self.work, env=env, text=True, capture_output=True
        )

    def observed(self):
        remote = {ref: self.label(sha) for ref, sha in self.refs().items() if ref != "HEAD"}
        tags = self.git(self.work, "tag", "--list").split()
        return {
            "remote": remote,
            "local": {
                "branch": self.git(self.work, "rev-parse", "--abbrev-ref", "HEAD"),
                "HEAD": self.label(self.git(self.work, "rev-parse", "HEAD")),
                "tags": tags,
                "status": self.git(self.work, "status", "--porcelain", "--untracked-files=all").splitlines(),
            },
        }

    def released_files(self, tag):
        versions = [
            line
            for line in self.git(self.root, f"--git-dir={self.bare}", "show", f"{tag}:Cargo.toml").splitlines()
            if line.startswith("version = ")
        ]
        snapshots = self.git(self.root, f"--git-dir={self.bare}", "ls-tree", "--name-only", f"{tag}:{SNAPSHOTS}").split()
        return {"versions": versions, "snapshots": snapshots}


def released(version):
    return f"Release v{version} on start"


RELEASE_CASES = (
    (
        "release from a current main",
        "0.16.1",
        None,
        False,
        {
            "code": 0,
            "remote": {"refs/heads/main": released("0.16.1"), "refs/tags/v0.16.1": released("0.16.1")},
            "local": {"branch": "main", "HEAD": released("0.16.1"), "tags": ["v0.16.1"], "status": []},
            "files": {
                "versions": ['version = "0.16.1"', 'version = "0.16.1"'],
                "snapshots": ["0.16.1.json", "current.json"],
            },
        },
    ),
    (
        "main moves while the release builds",
        "0.16.1",
        None,
        True,
        {
            "refused": "the push was rejected, so origin has no new main and no v0.16.1",
            "remote": {"refs/heads/main": "parallel on start"},
            "local": {"branch": "main", "HEAD": "start", "tags": [], "status": []},
        },
    ),
    (
        "main moved before the release",
        "0.16.1",
        "parallel",
        False,
        {
            "refused": "HEAD is not origin/main",
            "remote": {"refs/heads/main": "parallel on start"},
            "local": {"branch": "main", "HEAD": "start", "tags": [], "status": []},
        },
    ),
    (
        "release from a main with an unpushed commit",
        "0.16.1",
        "unpushed",
        False,
        {
            "refused": "HEAD is not origin/main",
            "remote": {"refs/heads/main": "start"},
            "local": {"branch": "main", "HEAD": "unpushed on start", "tags": [], "status": []},
        },
    ),
    (
        "release from a branch other than main",
        "0.16.1",
        "topic",
        False,
        {
            "refused": "run a release from main, not topic",
            "remote": {"refs/heads/main": "start"},
            "local": {"branch": "topic", "HEAD": "start", "tags": [], "status": []},
        },
    ),
    (
        "release with an uncommitted file",
        "0.16.1",
        "dirty",
        False,
        {
            "refused": "commit or stash your changes first",
            "remote": {"refs/heads/main": "start"},
            "local": {"branch": "main", "HEAD": "start", "tags": [], "status": ["?? notes.txt"]},
        },
    ),
    (
        "release candidate writes no wire snapshot",
        "0.17.0-rc.1",
        None,
        False,
        {
            "code": 0,
            "remote": {"refs/heads/main": released("0.17.0-rc.1"), "refs/tags/v0.17.0-rc.1": released("0.17.0-rc.1")},
            "local": {"branch": "main", "HEAD": released("0.17.0-rc.1"), "tags": ["v0.17.0-rc.1"], "status": []},
            "files": {
                "versions": ['version = "0.17.0-rc.1"', 'version = "0.17.0-rc.1"'],
                "snapshots": ["current.json"],
            },
        },
    ),
    (
        "version with a leading v",
        "v0.16.1",
        None,
        False,
        {
            "refused": "the version is MAJOR.MINOR.PATCH with an optional -suffix, not v0.16.1",
            "remote": {"refs/heads/main": "start"},
            "local": {"branch": "main", "HEAD": "start", "tags": [], "status": []},
        },
    ),
)


def prepare(sandbox, setup):
    if setup == "parallel":
        subprocess.run(["bash", "-c", sandbox.parallel_push()], env=sandbox.env, check=True)
    elif setup == "unpushed":
        sandbox.commit(sandbox.work, "unpushed")
    elif setup == "topic":
        sandbox.git(sandbox.work, "switch", "-q", "-c", "topic")
    elif setup == "dirty":
        (sandbox.work / "notes.txt").write_text("draft\n")


def release_errors(root, version, setup, race, expect):
    sandbox = ReleaseSandbox(root)
    prepare(sandbox, setup)
    proc = sandbox.release(version, sandbox.parallel_push() if race else None)
    seen = sandbox.observed()
    errors = []
    if "code" in expect and proc.returncode != expect["code"]:
        errors.append(f"exit {proc.returncode}, want {expect['code']}: {proc.stderr.strip()}")
    if "refused" in expect:
        if proc.returncode == 0:
            errors.append("exit 0, want a refusal")
        if expect["refused"] not in proc.stderr:
            errors.append(f"stderr missing [{expect['refused']}]: {proc.stderr.strip()}")
    for key in ("remote", "local"):
        if seen[key] != expect[key]:
            errors.append(f"{key} is {seen[key]}, want {expect[key]}")
    if "files" in expect:
        files = sandbox.released_files(f"v{version}")
        if files != expect["files"]:
            errors.append(f"released files are {files}, want {expect['files']}")
    return errors


def hub_pick(root, tags, live):
    remote = Remote(root)
    seed = root / "seed"
    seed.mkdir()
    remote.git(seed, "init", "-q", "-b", "main")
    commits = {}
    for tag in tags:
        commits[tag] = remote.commit(seed, tag)
        remote.git(seed, "tag", tag)
    remote.git(seed, "push", "-q", str(remote.bare), "main", f"{commits[live]}:refs/heads/railway-hub", "--tags")
    checkout = remote.clone("checkout")
    output = root / "output"
    summary = root / "summary"
    output.write_text("")
    summary.write_text("")
    loaded = workflow.load(HUB_DEPLOY)
    job = loaded["jobs"]["resolve"]
    step = next(step for step in job["steps"] if step.get("id") == "pick")
    script = root / "pick.sh"
    script.write_text(step["run"])
    proc = subprocess.run(
        [*workflow.bash_argv(workflow.shell_of(loaded, job, step)), str(script)],
        cwd=checkout,
        env={**remote.env, "GITHUB_OUTPUT": str(output), "GITHUB_STEP_SUMMARY": str(summary)},
        text=True,
        capture_output=True,
    )
    names = {sha: tag for tag, sha in commits.items()}
    outputs = {}
    for line in output.read_text().splitlines():
        key, _, value = line.partition("=")
        outputs[key] = names.get(value, value)
    return proc, outputs


HUB_CASES = (
    (
        "hub deploy picks the final over its release candidate",
        ["v0.16.0", "v0.16.1", "v0.16.10", "v0.17.0-rc.1", "v0.17.0"],
        "v0.16.0",
        {"sha": "v0.17.0", "tag": "v0.17.0", "live": "v0.16.0"},
    ),
    (
        "hub deploy never picks a release candidate",
        ["v0.16.0", "v0.17.0-rc.1"],
        "v0.16.0",
        {"sha": ""},
    ),
    (
        "hub deploy replaces a live release candidate with its final",
        ["v0.16.0", "v0.17.0-rc.1", "v0.17.0"],
        "v0.17.0-rc.1",
        {"sha": "v0.17.0", "tag": "v0.17.0", "live": "v0.17.0-rc.1"},
    ),
    (
        "hub deploy orders patch numbers as numbers",
        ["v0.16.0", "v0.16.9", "v0.16.10"],
        "v0.16.0",
        {"sha": "v0.16.10", "tag": "v0.16.10", "live": "v0.16.0"},
    ),
)


def report(label, errors):
    if errors:
        print(f"FAIL {label}")
        for error in errors:
            print(f"  {error}")
        return 1
    print(f"ok: {label}")
    return 0


def main():
    failed = 0
    with tempfile.TemporaryDirectory(prefix="release-tags-") as tmp:
        for index, (title, version, setup, race, expect) in enumerate(RELEASE_CASES):
            root = pathlib.Path(tmp) / f"release-{index}"
            root.mkdir()
            failed += report(title, release_errors(root, version, setup, race, expect))
        for index, (title, tags, live, want) in enumerate(HUB_CASES):
            root = pathlib.Path(tmp) / f"hub-{index}"
            root.mkdir()
            proc, outputs = hub_pick(root, tags, live)
            errors = [] if proc.returncode == 0 else [f"exit {proc.returncode}: {proc.stderr.strip()}"]
            if outputs != want:
                errors.append(f"outputs {outputs}, want {want}")
            failed += report(title, errors)
    total = len(RELEASE_CASES) + len(HUB_CASES)
    print(f"{failed} of {total} checks failed" if failed else f"ok: {total} checks")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
