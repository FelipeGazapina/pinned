from __future__ import annotations

import pathlib
import re

KEY = re.compile(r"^([A-Za-z_][\w.\-]*):(?:\s+(.*))?$")


def indent_of(line):
    return len(line) - len(line.lstrip(" "))


class Parser:
    def __init__(self, text, path):
        self.lines = text.splitlines()
        self.path = path
        self.i = 0

    def fail(self, message):
        raise SystemExit(f"{self.path}:{self.i + 1}: the workflow reader {message}")

    def next_indent(self):
        while self.i < len(self.lines):
            line = self.lines[self.i]
            stripped = line.strip()
            if stripped and not stripped.startswith("#"):
                if "\t" in line[: len(line) - len(line.lstrip())]:
                    self.fail("does not read tabs in indentation")
                return indent_of(line)
            self.i += 1
        return -1

    def text(self):
        return self.lines[self.i].strip()

    def is_item(self):
        return self.text() == "-" or self.text().startswith("- ")

    def document(self):
        if self.next_indent() != 0:
            self.fail("expects a mapping at column 0")
        result = self.mapping(0)
        if self.next_indent() != -1:
            self.fail("found text outside the top-level mapping")
        return result

    def node(self, indent):
        return self.sequence(indent) if self.is_item() else self.mapping(indent)

    def mapping(self, indent):
        result = {}
        while self.next_indent() == indent and not self.is_item():
            match = KEY.match(self.text())
            if match is None:
                self.fail(f"does not read [{self.text()}]")
            key = match.group(1)
            if key in result:
                self.fail(f"found {key} twice")
            self.i += 1
            result[key] = self.value(match.group(2) or "", indent)
        if self.next_indent() > indent:
            self.fail("found an indented line that belongs to no key")
        return result

    def sequence(self, indent):
        result = []
        while self.next_indent() == indent and self.is_item():
            rest = self.text()[1:]
            content = rest.lstrip()
            if not content:
                self.i += 1
                result.append(self.value("", indent))
                continue
            column = indent + 1 + len(rest) - len(content)
            if KEY.match(content):
                self.lines[self.i] = " " * column + content
                result.append(self.mapping(column))
            else:
                self.i += 1
                result.append(self.value(content, indent))
        if self.next_indent() > indent:
            self.fail("found an indented line after a list item")
        return result

    def value(self, raw, indent):
        raw = raw.strip()
        if not raw:
            start = self.i
            deeper = self.next_indent()
            if deeper > indent:
                return self.node(deeper)
            if deeper == indent and self.is_item():
                return self.sequence(indent)
            self.i = start
            return None
        if raw in ("|", "|-"):
            return self.block(indent, raw == "|")
        if raw[0] in "#&*!{>%@`?":
            self.fail(f"does not read [{raw}]")
        if raw[0] == "[":
            if not raw.endswith("]"):
                self.fail(f"reads a flow list on one line only: [{raw}]")
            inner = raw[1:-1].strip()
            return [self.scalar(item) for item in inner.split(",")] if inner else []
        return self.scalar(raw)

    def scalar(self, raw):
        raw = raw.strip()
        if raw[:1] == "'":
            if len(raw) < 2 or not raw.endswith("'"):
                self.fail(f"does not read [{raw}]")
            return raw[1:-1].replace("''", "'")
        if raw[:1] == '"':
            if len(raw) < 2 or not raw.endswith('"') or "\\" in raw:
                self.fail(f"does not read [{raw}]")
            return raw[1:-1]
        if " #" in raw:
            self.fail(f"does not read an inline comment: [{raw}]")
        return raw

    def block(self, indent, keep_newline):
        body = []
        content = None
        while self.i < len(self.lines):
            line = self.lines[self.i]
            if line.strip():
                if indent_of(line) <= indent:
                    break
                if content is None:
                    content = indent_of(line)
                if indent_of(line) < content:
                    self.fail("found a block line left of the block")
                body.append(line[content:])
            else:
                body.append("")
            self.i += 1
        while body and not body[-1]:
            body.pop()
        return "\n".join(body) + ("\n" if keep_newline and body else "")


def load(path):
    path = pathlib.Path(path)
    return Parser(path.read_text(), path).document()


def shell_of(workflow, job, step):
    for source in (step, (job.get("defaults") or {}).get("run") or {}, (workflow.get("defaults") or {}).get("run") or {}):
        if source.get("shell"):
            return source["shell"]
    return None


def bash_argv(shell):
    if shell is None:
        return ["bash", "-e"]
    if shell == "bash":
        return ["bash", "--noprofile", "--norc", "-eo", "pipefail"]
    raise SystemExit(f"the replay runs bash steps only, not {shell}")
