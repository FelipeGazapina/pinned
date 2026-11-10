//! The never list: what only Felipe may approve, whatever a pre-approval rule says.
//!
//! `never` is a pure function of the ask and the task's context. It runs in the hook process, so it
//! needs neither the node nor the hub. A hit means "show the normal prompt and record that the ask
//! was held for Felipe". This is a deny list, so every doubt resolves toward a hit: a line the
//! parser only half understands is still checked word by word.
//!
//! One line can hit several items. `never` returns the one that comes first in [`NeverItem`]'s
//! declaration order, which is the order of the table in the design (DESIGN-preauth-approvals.md,
//! section 2).

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

/// Where and as whom the ask runs.
#[derive(Debug, Clone, Copy)]
pub struct AskCtx<'a> {
    /// The task's checkout root (canonical).
    pub worktree: &'a Path,
    /// The session's scratch dir. Writes there are fine.
    pub scratch: Option<&'a Path>,
    pub home: &'a Path,
    /// The tool call's cwd.
    pub cwd: &'a Path,
    /// `main` usually.
    pub default_branch: &'a str,
    pub current_branch: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub enum Ask<'a> {
    Bash { command: &'a str },
    /// Edit, Write, MultiEdit and NotebookEdit.
    Edit { path: &'a Path },
    Read { path: &'a Path },
    WebFetch { url: &'a str },
    /// `name` is the full `mcp__server__tool` name.
    Mcp { name: &'a str, args: &'a Value },
}

impl<'a> Ask<'a> {
    /// The ask for one of Claude Code's tool calls, from the `tool_name` and `tool_input` of a hook
    /// payload. `None` for a tool the never list has no rule for, or an input that lacks its field.
    pub fn from_tool(tool_name: &'a str, input: &'a Value) -> Option<Ask<'a>> {
        let text = |keys: &[&str]| keys.iter().find_map(|k| input.get(*k)?.as_str());
        match tool_name {
            "Bash" => Some(Ask::Bash { command: text(&["command"])? }),
            "Edit" | "Write" | "MultiEdit" => Some(Ask::Edit { path: Path::new(text(&["file_path", "path"])?) }),
            "NotebookEdit" => Some(Ask::Edit { path: Path::new(text(&["notebook_path", "file_path"])?) }),
            "Read" | "NotebookRead" => Some(Ask::Read { path: Path::new(text(&["file_path", "notebook_path", "path"])?) }),
            "Grep" | "Glob" => Some(Ask::Read { path: Path::new(text(&["path"])?) }),
            "WebFetch" => Some(Ask::WebFetch { url: text(&["url"])? }),
            name if name.starts_with("mcp__") => Some(Ask::Mcp { name, args: input }),
            _ => None,
        }
    }
}

/// The items of the never list, in the order of the design's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NeverItem {
    DeleteOutside,
    ForcePush,
    PushToMain,
    Release,
    Deploy,
    Production,
    Secrets,
    Payment,
    WriteOutside,
    SelfGrant,
    NetworkInstall,
    MachineWide,
}

impl NeverItem {
    /// For "held for Felipe: <label>".
    pub fn label(self) -> &'static str {
        match self {
            NeverItem::DeleteOutside => "delete outside the worktree",
            NeverItem::ForcePush => "force-push",
            NeverItem::PushToMain => "push to main",
            NeverItem::Release => "release",
            NeverItem::Deploy => "deploy",
            NeverItem::Production => "production",
            NeverItem::Secrets => "secrets",
            NeverItem::Payment => "payment",
            NeverItem::WriteOutside => "write outside the worktree",
            NeverItem::SelfGrant => "self-grant",
            NeverItem::NetworkInstall => "network install",
            NeverItem::MachineWide => "machine-wide change",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeverHit {
    pub item: NeverItem,
    /// Short, for the audit row: "git push --force".
    pub why: String,
}

/// Whether only Felipe may approve `ask`, and why.
pub fn never(ctx: &AskCtx, ask: &Ask) -> Option<NeverHit> {
    let mut scan = Scan::new(ctx);
    match ask {
        Ask::Bash { command } => {
            let cwd = Some(scan.abs(ctx.cwd));
            scan.line(command, cwd, 0);
            scan.finish_line();
        }
        Ask::Edit { path } => scan.edit(path),
        Ask::Read { path } => scan.read(path),
        Ask::WebFetch { url } => {
            scan.production(url);
            if url.to_ascii_lowercase().starts_with("file:") {
                scan.secret_text(url);
            }
        }
        Ask::Mcp { name, args } => scan.mcp(name, args),
    }
    scan.hits.into_iter().min_by_key(|h| h.item)
}

// ---------------------------------------------------------------------------------------------
// Paths

enum Part {
    Prefix(OsString),
    Root,
    Cur,
    Parent,
    Name(OsString),
}

fn parts(p: &Path) -> VecDeque<Part> {
    p.components()
        .map(|c| match c {
            Component::Prefix(p) => Part::Prefix(p.as_os_str().to_owned()),
            Component::RootDir => Part::Root,
            Component::CurDir => Part::Cur,
            Component::ParentDir => Part::Parent,
            Component::Normal(n) => Part::Name(n.to_owned()),
        })
        .collect()
}

/// `abs` with every symlink followed and every `..` applied, like `fs::canonicalize`, but for a path
/// that doesn't exist yet too: the part that exists resolves, the rest is appended. A dangling
/// symlink resolves to where it points, since a write through it creates that file. Walking one
/// component at a time also puts a `..` after a missing component in the right place.
fn walk(abs: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    let mut todo = parts(abs);
    let mut hops = 0;
    while let Some(part) = todo.pop_front() {
        match part {
            Part::Prefix(p) => out = PathBuf::from(p),
            Part::Root => out.push(std::path::MAIN_SEPARATOR_STR),
            Part::Cur => {}
            Part::Parent => {
                out.pop();
            }
            Part::Name(n) => {
                out.push(&n);
                let link = std::fs::symlink_metadata(&out).is_ok_and(|m| m.file_type().is_symlink());
                if link {
                    hops += 1;
                    if hops > 40 {
                        continue;
                    }
                    if let Ok(target) = std::fs::read_link(&out) {
                        out.pop();
                        for p in parts(&target).into_iter().rev() {
                            todo.push_front(p);
                        }
                    }
                }
            }
        }
    }
    out
}

/// The components of a path or of the text of one, lowercase (macOS folds case), without `.`.
fn comps(text: &str) -> Vec<String> {
    text.split(['/', '\\']).filter(|c| !c.is_empty() && *c != ".").map(str::to_ascii_lowercase).collect()
}

fn path_comps(p: &Path) -> Vec<String> {
    p.components().filter_map(|c| match c {
        Component::Normal(n) => Some(n.to_string_lossy().to_ascii_lowercase()),
        _ => None,
    }).collect()
}

/// Whether the components name a credential: reading one is on the never list.
fn secret(c: &[String]) -> Option<&'static str> {
    for (i, s) in c.iter().enumerate() {
        let next = c.get(i + 1).map(String::as_str);
        match s.as_str() {
            ".ssh" | ".aws" | ".gnupg" | ".kube" | ".netrc" | ".git-credentials" | ".pypirc" | "keychains" => return Some("credentials"),
            ".config" if matches!(next, Some("gh" | "gcloud")) => return Some("credentials"),
            ".config" | "application support" if next == Some("pinned") && c.get(i + 2).map(String::as_str) != Some("uploads") => return Some("pinned's config"),
            ".claude" if next.is_some_and(|n| n.starts_with(".credentials")) => return Some("credentials"),
            ".docker" if next == Some("config.json") => return Some("credentials"),
            "proc" if c.get(i + 2).map(String::as_str) == Some("environ") => return Some("the environment"),
            _ => {}
        }
    }
    let last = c.last()?.as_str();
    let template = [".example", ".sample", ".template"].iter().any(|e| last.ends_with(e));
    if last == ".env" || (last.starts_with(".env.") && !template) {
        return Some("an env file");
    }
    if last.ends_with(".pem") || last.ends_with(".key") {
        return Some("a key file");
    }
    if ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"].iter().any(|k| last.starts_with(k)) && !last.ends_with(".pub") {
        return Some("an ssh key");
    }
    None
}

/// Whether writing the path changes what the agent may do, or what runs on this machine.
fn self_grant(c: &[String]) -> bool {
    let Some(last) = c.last() else { return false };
    if last == ".mcp.json" || last == "claude-settings.json" || last == "claude-mcp.json" || last.starts_with("approval-policy") {
        return true;
    }
    for (i, s) in c.iter().enumerate() {
        let next = c.get(i + 1).map(String::as_str);
        match s.as_str() {
            ".claude" if next.is_some_and(|n| n == "hooks" || (n.starts_with("settings") && n.ends_with(".json"))) => return true,
            ".husky" => return true,
            ".git" => {
                let hooks_or_config = |n: Option<&str>| matches!(n, Some("config" | "hooks" | "config.worktree"));
                if hooks_or_config(next) || (next == Some("worktrees") && hooks_or_config(c.get(i + 3).map(String::as_str))) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn expand_tilde(raw: &str, home: &Path) -> PathBuf {
    match raw.strip_prefix('~') {
        Some("") => home.to_owned(),
        Some(rest) if rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(raw),
    }
}

fn short(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= 60 {
        return s;
    }
    let cut: String = s.chars().take(57).collect();
    format!("{cut}...")
}

// ---------------------------------------------------------------------------------------------
// A small shell lexer. It splits a line into simple commands with quote-aware words, and never
// fails: what it can't follow it keeps as text, so the checks still see the words.

#[derive(Clone, Debug, Default)]
struct Word {
    text: String,
    /// Byte offset of the first `$`, command substitution or brace expansion: the word names a path
    /// from there on that nobody can know before the shell runs.
    var_at: Option<usize>,
    /// Byte offset of the first unquoted `*`, `?` or `[`.
    glob_at: Option<usize>,
    /// The word holds a command substitution that runs `curl` or `wget`.
    fetch: bool,
}

#[derive(Debug)]
struct Redir {
    write: bool,
    target: Word,
}

#[derive(Debug, Default)]
struct Segment {
    id: usize,
    words: Vec<Word>,
    redirs: Vec<Redir>,
    /// Parentheses and substitutions the segment sits in. A `cd` inside doesn't leave them.
    depth: usize,
    /// After a `|`.
    piped: bool,
    /// Here-document and here-string bodies.
    heredoc: Vec<String>,
}

struct Pending {
    delim: String,
    strip_tabs: bool,
    id: usize,
    quoted: bool,
}

#[derive(Default)]
struct WordBuilder {
    word: Word,
    started: bool,
}

impl WordBuilder {
    fn push(&mut self, c: char) {
        self.started = true;
        self.word.text.push(c);
    }
    fn push_str(&mut self, s: &str) {
        self.started = true;
        self.word.text.push_str(s);
    }
    fn var(&mut self) {
        self.started = true;
        self.word.var_at.get_or_insert(self.word.text.len());
    }
    fn glob(&mut self) {
        self.started = true;
        self.word.glob_at.get_or_insert(self.word.text.len());
    }
}

struct Lexer<'h> {
    c: Vec<char>,
    i: usize,
    home: &'h str,
    out: Vec<Segment>,
    next_id: usize,
    pending: Vec<Pending>,
}

fn blank(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r')
}

fn lex(src: &str, home: &str) -> Vec<Segment> {
    let mut lx = Lexer::new(src, home, 0);
    lx.list(0, false);
    lx.out
}

fn is_fetch(seg: &Segment) -> bool {
    first_prog(&seg.words).is_some_and(|p| p == "curl" || p == "wget")
}

impl<'h> Lexer<'h> {
    fn new(src: &str, home: &'h str, next_id: usize) -> Self {
        Lexer { c: src.chars().collect(), i: 0, home, out: vec![], next_id, pending: vec![] }
    }

    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    fn at(&self, k: usize) -> Option<char> {
        self.c.get(self.i + k).copied()
    }

    fn fresh(&mut self, depth: usize, piped: bool) -> Segment {
        self.next_id += 1;
        Segment { id: self.next_id, depth, piped, ..Segment::default() }
    }

    fn end(&mut self, cur: &mut Segment, depth: usize, piped_next: bool) {
        if cur.words.is_empty() && cur.redirs.is_empty() && cur.heredoc.is_empty() {
            cur.piped = piped_next;
            cur.depth = depth;
            return;
        }
        let next = self.fresh(depth, piped_next);
        let done = std::mem::replace(cur, next);
        self.out.push(done);
    }

    /// Commands until the end, or until the `)` that closes a substitution.
    fn list(&mut self, base: usize, in_sub: bool) {
        let mut d = base;
        let mut cur = self.fresh(d, false);
        loop {
            while self.peek().is_some_and(blank) {
                self.i += 1;
            }
            let Some(ch) = self.peek() else { break };
            match ch {
                '#' => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.i += 1;
                    }
                }
                '\n' => {
                    self.i += 1;
                    self.end(&mut cur, d, false);
                    self.heredocs(d);
                }
                ';' => {
                    self.i += 1;
                    self.end(&mut cur, d, false);
                }
                '&' => match self.at(1) {
                    Some('&') => {
                        self.i += 2;
                        self.end(&mut cur, d, false);
                    }
                    Some('>') => {
                        self.i += 1;
                        self.redir(&mut cur, d, true);
                    }
                    _ => {
                        self.i += 1;
                        self.end(&mut cur, d, false);
                    }
                },
                '|' => {
                    self.i += 1;
                    let or = self.peek() == Some('|');
                    if or || self.peek() == Some('&') {
                        self.i += 1;
                    }
                    self.end(&mut cur, d, !or);
                }
                '(' => {
                    self.i += 1;
                    self.end(&mut cur, d, false);
                    d += 1;
                    cur.depth = d;
                }
                ')' => {
                    self.i += 1;
                    self.end(&mut cur, d, false);
                    if d > base {
                        d -= 1;
                        cur.depth = d;
                    } else if in_sub {
                        return;
                    }
                }
                '<' | '>' => self.redir(&mut cur, d, false),
                _ => {
                    let w = self.word(d);
                    cur.words.push(w);
                }
            }
        }
        self.end(&mut cur, d, false);
    }

    /// The bodies of the here-documents the line just ended opened.
    fn heredocs(&mut self, d: usize) {
        for p in std::mem::take(&mut self.pending) {
            let mut body = String::new();
            while self.i < self.c.len() {
                let start = self.i;
                while self.peek().is_some_and(|c| c != '\n') {
                    self.i += 1;
                }
                let line: String = self.c[start..self.i].iter().collect();
                self.i += 1;
                let cmp = if p.strip_tabs { line.trim_start_matches('\t') } else { line.as_str() };
                if cmp == p.delim {
                    break;
                }
                body.push_str(&line);
                body.push('\n');
            }
            if !p.quoted {
                // An unquoted body runs its `$(...)` and backticks, and nothing else of it.
                let mut sub = Lexer::new(&body, self.home, self.next_id);
                let mut w = WordBuilder::default();
                sub.dq(&mut w, d + 1, false);
                self.next_id = sub.next_id;
                self.out.append(&mut sub.out);
            }
            if let Some(seg) = self.out.iter_mut().rev().find(|s| s.id == p.id) {
                seg.heredoc.push(body);
            }
        }
    }

    fn word_after_blanks(&mut self, d: usize) -> Word {
        while self.peek().is_some_and(blank) {
            self.i += 1;
        }
        match self.peek() {
            None | Some('\n' | ';' | '&' | '|' | '<' | '>' | '(' | ')') => Word::default(),
            Some(_) => self.word(d),
        }
    }

    /// A redirection. `self.i` is at the `<` or `>`.
    fn redir(&mut self, cur: &mut Segment, d: usize, amp: bool) {
        if !amp && self.i > 0 && !blank(self.c[self.i - 1]) {
            if let Some(w) = cur.words.last() {
                if !w.text.is_empty() && w.var_at.is_none() && w.text.chars().all(|c| c.is_ascii_digit()) {
                    cur.words.pop();
                }
            }
        }
        if self.peek() == Some('>') {
            self.i += 1;
            match self.peek() {
                Some('>' | '|') => self.i += 1,
                Some('&') => {
                    self.i += 1;
                    let w = self.word_after_blanks(d);
                    if !(w.text == "-" || w.text.chars().all(|c| c.is_ascii_digit())) {
                        cur.redirs.push(Redir { write: true, target: w });
                    }
                    return;
                }
                Some('(') => {
                    self.i += 1;
                    self.sub(d + 1);
                    return;
                }
                _ => {}
            }
            let target = self.word_after_blanks(d);
            if !target.text.is_empty() {
                cur.redirs.push(Redir { write: true, target });
            }
            return;
        }
        match self.at(1) {
            Some('(') => {
                self.i += 2;
                let fetch = self.sub(d + 1);
                cur.words.push(Word { text: "<()".into(), var_at: Some(0), fetch, ..Word::default() });
            }
            Some('<') if self.at(2) == Some('<') => {
                self.i += 3;
                let w = self.word_after_blanks(d);
                cur.heredoc.push(w.text);
            }
            Some('<') => {
                self.i += 2;
                let strip_tabs = self.peek() == Some('-');
                if strip_tabs {
                    self.i += 1;
                }
                while self.peek().is_some_and(blank) {
                    self.i += 1;
                }
                let start = self.i;
                let w = self.word(d);
                let raw: String = self.c[start..self.i].iter().collect();
                let quoted = raw.contains(['\'', '"', '\\']);
                self.pending.push(Pending { delim: w.text, strip_tabs, id: cur.id, quoted });
            }
            Some('&') => {
                self.i += 2;
                self.word_after_blanks(d);
            }
            _ => {
                self.i += 1;
                if self.peek() == Some('>') {
                    self.i += 1;
                    let target = self.word_after_blanks(d);
                    cur.redirs.push(Redir { write: true, target });
                    return;
                }
                let target = self.word_after_blanks(d);
                if !target.text.is_empty() {
                    cur.redirs.push(Redir { write: false, target });
                }
            }
        }
    }

    /// A substitution's commands, up to its `)`. They join the output, one level deeper.
    /// Whether they fetch from the network.
    fn sub(&mut self, d: usize) -> bool {
        let n0 = self.out.len();
        self.list(d, true);
        self.out[n0..].iter().any(is_fetch)
    }

    fn word(&mut self, d: usize) -> Word {
        let mut w = WordBuilder::default();
        while let Some(ch) = self.peek() {
            match ch {
                c if blank(c) => break,
                '\n' | ';' | '&' | '|' | '<' | '>' | '(' | ')' => break,
                '\'' => {
                    self.i += 1;
                    w.started = true;
                    while let Some(c) = self.peek() {
                        self.i += 1;
                        if c == '\'' {
                            break;
                        }
                        w.push(c);
                    }
                }
                '"' => {
                    self.i += 1;
                    w.started = true;
                    self.dq(&mut w, d, true);
                }
                '\\' => {
                    self.i += 1;
                    match self.peek() {
                        Some('\n') => self.i += 1,
                        Some(c) => {
                            self.i += 1;
                            w.push(c);
                        }
                        None => {}
                    }
                }
                '$' => self.dollar(&mut w, d, false),
                '`' => self.backtick(&mut w, d),
                '*' | '?' | '[' => {
                    w.glob();
                    w.push(ch);
                    self.i += 1;
                }
                '{' => {
                    if self.brace_expansion() {
                        w.var();
                    }
                    w.push(ch);
                    self.i += 1;
                }
                '~' if !w.started => {
                    self.i += 1;
                    match self.peek() {
                        None | Some('/') => w.push_str(self.home),
                        Some(c) if blank(c) || matches!(c, '\n' | ';' | '&' | '|' | '<' | '>' | '(' | ')') => w.push_str(self.home),
                        Some(_) => {
                            // ~user, ~+, ~-: a place nobody can name from here.
                            w.var();
                            w.push('~');
                        }
                    }
                }
                c => {
                    self.i += 1;
                    w.push(c);
                }
            }
        }
        w.word
    }

    /// Whether the `{` at `self.i` opens a brace expansion such as `{a,b}` or `{1..3}`.
    fn brace_expansion(&self) -> bool {
        let mut k = self.i + 1;
        let mut list = false;
        while let Some(&c) = self.c.get(k) {
            match c {
                '}' => return list,
                ',' => list = true,
                '.' if self.c.get(k + 1) == Some(&'.') => list = true,
                c if blank(c) || matches!(c, '\n' | ';' | '&' | '|' | '<' | '>' | '(' | ')' | '{') => return false,
                _ => {}
            }
            k += 1;
        }
        false
    }

    /// Inside double quotes (or a here-document body), up to the closing quote when `quoted`.
    fn dq(&mut self, w: &mut WordBuilder, d: usize, quoted: bool) {
        while let Some(ch) = self.peek() {
            match ch {
                '"' if quoted => {
                    self.i += 1;
                    return;
                }
                '\\' => {
                    self.i += 1;
                    match self.peek() {
                        Some('\n') => self.i += 1,
                        Some(c) if matches!(c, '"' | '\\' | '$' | '`') => {
                            self.i += 1;
                            w.push(c);
                        }
                        _ => w.push('\\'),
                    }
                }
                '$' => self.dollar(w, d, true),
                '`' => self.backtick(w, d),
                c => {
                    self.i += 1;
                    w.push(c);
                }
            }
        }
    }

    /// A `$` at `self.i`.
    fn dollar(&mut self, w: &mut WordBuilder, d: usize, in_dq: bool) {
        match self.at(1) {
            Some('(') if self.at(2) == Some('(') => {
                // $(( arithmetic )): it runs no command of its own.
                self.i += 3;
                let mut depth = 1;
                while let Some(c) = self.peek() {
                    self.i += 1;
                    match c {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if self.peek() == Some(')') {
                    self.i += 1;
                }
                w.var();
                w.push_str("$(())");
            }
            Some('(') => {
                self.i += 2;
                let fetch = self.sub(d + 1);
                w.word.fetch |= fetch;
                w.var();
                w.push_str("$()");
            }
            Some('{') => {
                self.i += 2;
                let mut name = String::new();
                let mut depth = 1;
                while let Some(c) = self.peek() {
                    self.i += 1;
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    name.push(c);
                }
                if name == "HOME" {
                    w.push_str(self.home);
                } else {
                    w.var();
                    w.push_str(&format!("${{{name}}}"));
                }
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                self.i += 1;
                let mut name = String::new();
                while let Some(c) = self.peek().filter(|c| c.is_ascii_alphanumeric() || *c == '_') {
                    self.i += 1;
                    name.push(c);
                }
                if name == "HOME" {
                    w.push_str(self.home);
                } else {
                    w.var();
                    w.push_str(&format!("${name}"));
                }
            }
            Some('\'') if !in_dq => {
                self.i += 2;
                self.ansi(w);
            }
            Some('"') if !in_dq => self.i += 1,
            Some(c) if c.is_ascii_digit() || matches!(c, '?' | '$' | '!' | '@' | '*' | '#' | '-') => {
                self.i += 2;
                w.var();
                w.push('$');
                w.push(c);
            }
            _ => {
                self.i += 1;
                w.push('$');
            }
        }
    }

    /// A `$'...'` string, after its opening quote.
    fn ansi(&mut self, w: &mut WordBuilder) {
        w.started = true;
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                '\'' => break,
                '\\' => {
                    let Some(e) = self.peek() else { break };
                    self.i += 1;
                    match e {
                        'n' => w.push('\n'),
                        't' => w.push('\t'),
                        'r' => w.push('\r'),
                        'a' => w.push('\x07'),
                        'b' => w.push('\x08'),
                        'e' | 'E' => w.push('\x1b'),
                        'f' => w.push('\x0c'),
                        'v' => w.push('\x0b'),
                        '\\' | '\'' | '"' | '?' => w.push(e),
                        'x' | 'u' | 'U' => {
                            let max = match e {
                                'x' => 2,
                                'u' => 4,
                                _ => 8,
                            };
                            let mut v = 0u32;
                            let mut n = 0;
                            while n < max {
                                let Some(h) = self.peek().and_then(|c| c.to_digit(16)) else { break };
                                self.i += 1;
                                v = v * 16 + h;
                                n += 1;
                            }
                            w.push(char::from_u32(v).unwrap_or('?'));
                        }
                        '0'..='7' => {
                            let mut v = e.to_digit(8).unwrap_or(0);
                            let mut n = 1;
                            while n < 3 {
                                let Some(o) = self.peek().and_then(|c| c.to_digit(8)) else { break };
                                self.i += 1;
                                v = v * 8 + o;
                                n += 1;
                            }
                            w.push(char::from_u32(v).unwrap_or('?'));
                        }
                        other => {
                            w.push('\\');
                            w.push(other);
                        }
                    }
                }
                c => w.push(c),
            }
        }
    }

    fn backtick(&mut self, w: &mut WordBuilder, d: usize) {
        self.i += 1;
        let mut inner = String::new();
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                '`' => break,
                '\\' if matches!(self.peek(), Some('`' | '\\' | '$')) => {
                    inner.push(self.peek().unwrap_or('\\'));
                    self.i += 1;
                }
                c => inner.push(c),
            }
        }
        let mut sub = Lexer::new(&inner, self.home, self.next_id);
        sub.list(d + 1, false);
        self.next_id = sub.next_id;
        w.word.fetch |= sub.out.iter().any(is_fetch);
        self.out.append(&mut sub.out);
        w.var();
        w.push_str("$()");
    }
}

// ---------------------------------------------------------------------------------------------
// Commands

const KEYWORDS: &[&str] = &["if", "then", "else", "elif", "fi", "while", "until", "do", "done", "for", "case", "esac", "select", "function", "time", "!", "{", "}", "[[", "coproc"];

/// Programs that run the rest of their line as a command. Every later word is a candidate command.
const WRAPPERS: &[&str] = &[
    "sudo", "doas", "env", "command", "exec", "nohup", "nice", "ionice", "timeout", "xargs", "stdbuf", "setsid", "caffeinate", "builtin", "watch", "unbuffer", "parallel", "arch", "chronic",
];

const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "ash", "fish"];

/// Programs that run what they read, so downloaded text must not reach them.
const RUNNERS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "ash", "fish", "python", "python3", "python2", "node", "deno", "bun", "perl", "ruby", "php", "eval", "source", "."];

fn is_assign(t: &str) -> bool {
    let Some((name, _)) = t.split_once('=') else { return false };
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn first_prog(words: &[Word]) -> Option<String> {
    let w = words.iter().find(|w| !is_assign(&w.text) && !KEYWORDS.contains(&w.text.as_str()))?;
    Some(w.text.rsplit('/').next().unwrap_or("").to_owned())
}

struct Cmd {
    /// The program's file name, without its directory.
    prog: String,
    args: Vec<Word>,
    /// Its targets come from a pipe (`xargs`).
    stdin_args: bool,
}

impl Cmd {
    fn new(words: &[Word], stdin_args: bool) -> Cmd {
        let prog = words[0].text.rsplit('/').next().unwrap_or("").to_owned();
        Cmd { prog, args: words[1..].to_vec(), stdin_args }
    }

    fn flag_texts(&self) -> impl Iterator<Item = &str> {
        self.args.iter().map(|w| w.text.as_str()).take_while(|t| *t != "--").filter(|t| t.starts_with('-') && t.len() > 1)
    }

    /// Whether a flag is there: `shorts` among the letters of a `-abc` cluster, or one of `longs`.
    fn has(&self, shorts: &str, longs: &[&str]) -> bool {
        self.flag_texts().any(|t| match t.strip_prefix("--") {
            Some(l) => longs.contains(&l.split('=').next().unwrap_or("")),
            None => t[1..].chars().any(|c| shorts.contains(c)),
        })
    }

    fn positionals(&self) -> Vec<&Word> {
        let mut after = false;
        self.args
            .iter()
            .filter(|w| {
                if after {
                    return true;
                }
                if w.text == "--" {
                    after = true;
                    return false;
                }
                !(w.text.starts_with('-') && w.text.len() > 1)
            })
            .collect()
    }

    /// The value of `-X value`, `-Xvalue`'s next word, or `--long=value` and `--long value`.
    fn value(&self, short: char, long: &str) -> Option<Word> {
        let flag = format!("-{short}");
        let long_eq = format!("--{long}=");
        let long_flag = format!("--{long}");
        for (k, w) in self.args.iter().enumerate() {
            let t = w.text.as_str();
            if t == flag || t == long_flag {
                return self.args.get(k + 1).cloned();
            }
            if let Some(v) = t.strip_prefix(&long_eq) {
                return Some(Word { text: v.to_owned(), ..Word::default() });
            }
            if t.len() > 2 && t.starts_with('-') && !t.starts_with("--") && t.ends_with(short) {
                return self.args.get(k + 1).cloned();
            }
        }
        // `-XPOST`, `-ofile`: the value is stuck to the flag.
        self.args.iter().find_map(|w| w.text.strip_prefix(&flag).filter(|v| !v.is_empty() && !w.text.starts_with("--")).map(|v| Word { text: v.to_owned(), ..Word::default() }))
    }
}

/// The commands a segment runs: its program, and when it is a wrapper, every later word as a program
/// too, since a wrapper's options (`timeout 30`, `nice -n 5`, `xargs -I {}`) can't be told from its command.
fn commands(words: &[Word]) -> Vec<Cmd> {
    let mut i = 0;
    while i < words.len() && (is_assign(&words[i].text) || KEYWORDS.contains(&words[i].text.as_str())) {
        i += 1;
    }
    if i >= words.len() {
        return vec![];
    }
    let first = Cmd::new(&words[i..], false);
    let wrapper = WRAPPERS.contains(&first.prog.as_str());
    let stdin = first.prog == "xargs";
    let mut out = vec![first];
    if wrapper {
        for j in i + 1..words.len() {
            let t = words[j].text.as_str();
            if t.starts_with('-') || is_assign(t) {
                continue;
            }
            out.push(Cmd::new(&words[j..], stdin));
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The scan

/// Where a word points: the part of its path that is known, and whether the first component isn't.
struct Tgt {
    path: Option<PathBuf>,
    /// The first component holds a `$`, a substitution or a brace expansion.
    top_var: bool,
    /// The first component holds a glob. `path` is then the cwd.
    top_glob: bool,
}

struct Frame {
    cwd: Option<PathBuf>,
    /// A `curl` or `wget` ran earlier in this pipeline.
    fetched: bool,
}

struct Scan<'c, 'a> {
    ctx: &'c AskCtx<'a>,
    work: PathBuf,
    scratch: Option<PathBuf>,
    hits: Vec<NeverHit>,
    tagged: bool,
    pushed: bool,
}

const DEV_OK: &[&str] = &["/dev/null", "/dev/stdout", "/dev/stderr", "/dev/stdin", "/dev/tty", "/dev/zero"];

impl<'c, 'a> Scan<'c, 'a> {
    fn new(ctx: &'c AskCtx<'a>) -> Self {
        let mut scan = Scan { ctx, work: PathBuf::new(), scratch: None, hits: vec![], tagged: false, pushed: false };
        scan.work = walk(&scan.abs(ctx.worktree));
        scan.scratch = ctx.scratch.map(|s| walk(&scan.abs(s)));
        scan
    }

    fn abs(&self, p: &Path) -> PathBuf {
        if p.is_absolute() {
            p.to_owned()
        } else {
            self.ctx.cwd.join(p)
        }
    }

    fn hit(&mut self, item: NeverItem, why: impl Into<String>) {
        self.hits.push(NeverHit { item, why: why.into() });
    }

    fn outside(&self, p: &Path) -> bool {
        !(p.starts_with(&self.work) || self.scratch.as_ref().is_some_and(|s| p.starts_with(s)))
    }

    fn home(&self) -> String {
        self.ctx.home.to_string_lossy().into_owned()
    }

    fn resolve(&self, cwd: &Option<PathBuf>, raw: &str) -> Option<PathBuf> {
        if raw.is_empty() {
            return None;
        }
        let p = Path::new(raw);
        Some(walk(&if p.is_absolute() { p.to_owned() } else { cwd.as_ref()?.join(p) }))
    }

    fn tgt(&self, cwd: &Option<PathBuf>, w: &Word) -> Tgt {
        let t = w.text.as_str();
        let first = [w.var_at, w.glob_at].into_iter().flatten().min();
        let Some(at) = first else {
            return Tgt { path: self.resolve(cwd, t), top_var: false, top_glob: false };
        };
        let top_end = t.find('/').unwrap_or(t.len());
        let top_var = w.var_at.is_some_and(|v| v < top_end);
        let top_glob = !top_var && w.glob_at.is_some_and(|g| g < top_end);
        if top_var {
            return Tgt { path: None, top_var, top_glob: false };
        }
        if top_glob {
            return Tgt { path: cwd.clone(), top_var, top_glob };
        }
        let head = &t[..at];
        let dir = head.rfind('/').map(|k| &head[..=k]).unwrap_or("");
        Tgt { path: self.resolve(cwd, dir), top_var: false, top_glob: false }
    }

    // -- text and tools -----------------------------------------------------------------------

    fn production(&mut self, text: &str) {
        let lower = text.to_ascii_lowercase();
        for needle in ["pinned-hub-production", "railway.app"] {
            if lower.contains(needle) {
                self.hit(NeverItem::Production, format!("mentions {needle}"));
                return;
            }
        }
    }

    fn secret_text(&mut self, text: &str) {
        if let Some(what) = secret(&comps(text)) {
            self.hit(NeverItem::Secrets, format!("reads {what}: {}", short(text)));
        }
    }

    fn edit(&mut self, path: &Path) {
        let raw = path.to_string_lossy();
        let expanded = expand_tilde(&raw, self.ctx.home);
        let cwd = Some(self.abs(Path::new(".")));
        let resolved = self.resolve(&cwd, &expanded.to_string_lossy());
        self.secret_path(&raw, resolved.as_deref());
        self.write_path(&raw, resolved);
    }

    fn read(&mut self, path: &Path) {
        let raw = path.to_string_lossy();
        let expanded = expand_tilde(&raw, self.ctx.home);
        let cwd = Some(self.abs(Path::new(".")));
        let resolved = self.resolve(&cwd, &expanded.to_string_lossy());
        self.secret_path(&raw, resolved.as_deref());
    }

    fn secret_path(&mut self, text: &str, resolved: Option<&Path>) {
        let found = secret(&comps(text)).or_else(|| resolved.and_then(|p| secret(&path_comps(p))));
        if let Some(what) = found {
            self.hit(NeverItem::Secrets, format!("reads {what}: {}", short(text)));
        }
    }

    fn mcp(&mut self, name: &str, args: &Value) {
        let lower = name.to_ascii_lowercase();
        if lower.contains("railway") {
            self.hit(NeverItem::Deploy, name);
        }
        if ["payment", "checkout", "invoice", "charge"].iter().any(|w| lower.contains(w)) {
            self.hit(NeverItem::Payment, name);
        }
        self.production(name);
        let mut strings = vec![];
        json_strings(args, 0, &mut strings);
        for s in strings {
            self.production(s);
        }
    }

    // -- a Bash line --------------------------------------------------------------------------

    fn line(&mut self, line: &str, cwd: Option<PathBuf>, nest: u8) {
        if nest > 8 {
            return;
        }
        self.production(line);
        let segs = lex(line, &self.home());
        let mut frames = vec![Frame { cwd, fetched: false }];
        for seg in &segs {
            while frames.len() <= seg.depth {
                let last = frames.last().map(|f| Frame { cwd: f.cwd.clone(), fetched: false });
                frames.extend(last);
            }
            frames.truncate(seg.depth + 1);
            self.segment(seg, &mut frames, nest);
        }
    }

    /// After the whole line: a tag made and pushed in one line is a release.
    fn finish_line(&mut self) {
        if self.tagged && self.pushed {
            self.hit(NeverItem::Release, "git tag, then git push");
        }
    }

    fn segment(&mut self, seg: &Segment, frames: &mut [Frame], nest: u8) {
        let d = seg.depth;
        let cwd = frames[d].cwd.clone();
        let prior_fetch = seg.piped && frames[d].fetched;
        if !seg.piped {
            frames[d].fetched = false;
        }

        for r in &seg.redirs {
            self.secret_word(&cwd, &r.target);
            if r.write {
                self.write_word(&cwd, &r.target);
            }
        }
        for w in &seg.words {
            self.secret_word(&cwd, w);
        }

        let cmds = commands(&seg.words);
        for c in &cmds {
            self.cmd(&cwd, c, nest, prior_fetch);
        }
        if let Some(c) = cmds.first() {
            if c.prog == "curl" || c.prog == "wget" {
                frames[d].fetched = true;
            }
            if matches!(c.prog.as_str(), "cd" | "pushd") {
                frames[d].cwd = match c.positionals().first() {
                    None => Some(walk(Path::new(&self.home()))),
                    Some(w) if w.text == "-" => None,
                    Some(w) => self.tgt(&cwd, w).path,
                };
            } else if c.prog == "popd" {
                frames[d].cwd = None;
            }
        }
        if cmds.iter().any(|c| SHELLS.contains(&c.prog.as_str())) {
            for body in &seg.heredoc {
                self.line(body, cwd.clone(), nest + 1);
            }
        }
    }

    /// A word that names a credential, anywhere on the line: `cat ~/.ssh/id_ed25519`, `< ~/.netrc`,
    /// `--file=.env`, or a link in the worktree that points at one.
    fn secret_word(&mut self, cwd: &Option<PathBuf>, w: &Word) {
        let mut cands = vec![w.text.as_str()];
        if let Some((_, v)) = w.text.split_once('=') {
            cands.push(v);
        }
        for t in cands {
            if t.is_empty() || t.len() > 1024 || (t.chars().any(char::is_whitespace) && !t.starts_with('/') && !t.starts_with('~')) {
                continue;
            }
            if let Some(what) = secret(&comps(t)) {
                self.hit(NeverItem::Secrets, format!("reads {what}: {}", short(t)));
                return;
            }
            if w.var_at.is_none() && w.glob_at.is_none() && !t.starts_with('-') {
                if let Some(p) = self.resolve(cwd, t) {
                    if let Some(what) = secret(&path_comps(&p)) {
                        self.hit(NeverItem::Secrets, format!("reads {what}: {}", short(t)));
                        return;
                    }
                }
            }
        }
    }

    fn write_word(&mut self, cwd: &Option<PathBuf>, w: &Word) {
        if DEV_OK.contains(&w.text.as_str()) {
            return;
        }
        let t = self.tgt(cwd, w);
        let unresolved = t.top_var || t.path.is_none();
        let path = t.path;
        let mut text = w.text.clone();
        if unresolved {
            text = format!("{text} (unresolved)");
        }
        self.write_check(&w.text, &text, path, unresolved);
    }

    fn write_path(&mut self, text: &str, resolved: Option<PathBuf>) {
        let unresolved = resolved.is_none();
        self.write_check(text, text, resolved, unresolved);
    }

    fn write_check(&mut self, raw: &str, shown: &str, path: Option<PathBuf>, unresolved: bool) {
        if self_grant(&comps(raw)) || path.as_deref().is_some_and(|p| self_grant(&path_comps(p))) {
            self.hit(NeverItem::SelfGrant, format!("writes {}", short(shown)));
        }
        if unresolved || path.as_deref().is_some_and(|p| self.outside(p)) {
            self.hit(NeverItem::WriteOutside, format!("writes {}", short(shown)));
        }
    }

    fn delete(&mut self, cwd: &Option<PathBuf>, targets: &[&Word], risky: bool, what: &str) {
        for w in targets {
            let t = self.tgt(cwd, w);
            let bad = match &t.path {
                _ if t.top_var => true,
                None => true,
                Some(_) if t.top_glob => risky,
                Some(p) => self.outside(p) || (risky && w.var_at.is_none() && w.glob_at.is_none() && *p == self.work),
            };
            if bad {
                self.hit(NeverItem::DeleteOutside, format!("{what} {}", short(&w.text)));
            }
        }
    }

    fn cmd(&mut self, cwd: &Option<PathBuf>, c: &Cmd, nest: u8, pipe_fetch: bool) {
        let prog = c.prog.as_str();
        let pos = c.positionals();
        let sub = pos.first().map(|w| w.text.as_str()).unwrap_or("");
        if prog == "release.sh" || c.args.iter().any(|w| w.text.ends_with("scripts/release.sh")) {
            self.hit(NeverItem::Release, "scripts/release.sh");
        }
        if RUNNERS.contains(&prog) && (pipe_fetch || c.args.iter().any(|w| w.fetch)) {
            self.hit(NeverItem::NetworkInstall, format!("{prog} runs downloaded code"));
        }
        match prog {
            "rm" | "rmdir" | "unlink" | "shred" => {
                let risky = c.has("rRf", &["recursive", "force"]);
                if c.stdin_args && risky {
                    self.hit(NeverItem::DeleteOutside, format!("xargs {prog}: targets unknown"));
                }
                self.delete(cwd, &pos, risky, prog);
            }
            "find" => self.find(cwd, c, nest),
            "git" => self.git(cwd, c),
            "gh" => self.gh(c),
            "sudo" | "doas" | "su" | "launchctl" | "osascript" | "chown" | "ssh" | "scp" | "systemctl" | "shutdown" | "reboot" | "diskutil" | "networksetup" | "pmset" | "dscl" => {
                self.hit(NeverItem::MachineWide, prog.to_owned());
            }
            "crontab" if !c.has("l", &["list"]) => self.hit(NeverItem::MachineWide, "crontab"),
            "defaults" if matches!(sub, "write" | "delete" | "import" | "rename") => self.hit(NeverItem::MachineWide, format!("defaults {sub}")),
            "chmod" => {
                if c.has("R", &["recursive"]) {
                    self.hit(NeverItem::MachineWide, "chmod -R");
                }
                let skip = usize::from(!c.args.iter().any(|w| w.text.starts_with("--reference")));
                self.write_words(cwd, pos.iter().skip(skip).map(|w| (*w).clone()).collect());
            }
            "rsync" => {
                let remote = |p: &str| p.contains("://") || p.find(':').is_some_and(|k| !p[..k].contains('/'));
                if pos.iter().any(|w| remote(&w.text)) {
                    self.hit(NeverItem::MachineWide, "rsync to a host");
                } else if let Some(last) = pos.last() {
                    self.write_words(cwd, vec![(*last).clone()]);
                }
            }
            "railway" | "vercel" | "fly" | "flyctl" | "wrangler" | "kubectl" | "helm" | "netlify" | "heroku" => self.hit(NeverItem::Deploy, prog.to_owned()),
            "terraform" | "tofu" if matches!(sub, "apply" | "destroy") => self.hit(NeverItem::Deploy, format!("{prog} {sub}")),
            "docker" | "podman" if sub == "push" => self.hit(NeverItem::Deploy, format!("{prog} push")),
            "stripe" => self.hit(NeverItem::Payment, "stripe"),
            "upstash" => self.hit(NeverItem::Payment, "upstash"),
            "box" if sub == "create" => self.hit(NeverItem::Payment, "box create"),
            "security" | "op" | "pass" | "printenv" => self.hit(NeverItem::Secrets, prog.to_owned()),
            "env" if pos.iter().all(|w| is_assign(&w.text)) => self.hit(NeverItem::Secrets, "env prints the environment"),
            "set" if c.args.is_empty() => self.hit(NeverItem::Secrets, "set prints the environment"),
            "export" | "declare" | "typeset" if (prog == "export" && c.args.is_empty()) || c.has("p", &[]) => self.hit(NeverItem::Secrets, format!("{prog} -p")),
            "cp" | "install" | "mv" | "ln" | "tee" | "touch" | "mkdir" | "truncate" | "mkfifo" | "sed" | "dd" | "curl" | "wget" => {
                let words = self.write_args(c);
                self.write_words(cwd, words);
            }
            "npm" | "pnpm" | "yarn" | "bun" | "npx" | "bunx" | "cargo" | "pip" | "pip3" | "pip2" | "pipx" | "python" | "python3" | "python2" | "uv" | "uvx" | "poetry" | "brew" | "apt" | "apt-get" | "aptitude"
            | "dnf" | "yum" | "apk" | "zypper" | "pacman" | "gem" | "go" | "conda" | "mamba" | "micromamba" | "rustup" | "volta" | "asdf" | "mise" | "composer" | "bundle" | "dotnet" => {
                if let Some(why) = self.install(cwd, c) {
                    self.hit(NeverItem::NetworkInstall, why);
                }
                if matches!(prog, "npm" | "pnpm" | "yarn" | "cargo") && sub == "publish" {
                    self.hit(NeverItem::Release, format!("{prog} publish"));
                }
            }
            _ => {}
        }
        if SHELLS.contains(&prog) {
            for (k, w) in c.args.iter().enumerate() {
                if w.text.starts_with('-') && !w.text.starts_with("--") && w.text.contains('c') {
                    if let Some(script) = c.args.get(k + 1) {
                        self.line(&script.text, cwd.clone(), nest + 1);
                    }
                    break;
                }
            }
        }
        if prog == "eval" {
            let script = c.args.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            self.line(&script, cwd.clone(), nest + 1);
        }
        if prog == "watch" {
            for w in c.args.iter().filter(|w| w.text.contains(' ')) {
                self.line(&w.text, cwd.clone(), nest + 1);
            }
        }
    }

    fn find(&mut self, cwd: &Option<PathBuf>, c: &Cmd, nest: u8) {
        let mut deletes = c.args.iter().any(|w| w.text == "-delete");
        for (k, w) in c.args.iter().enumerate() {
            if !matches!(w.text.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
                continue;
            }
            let inner: Vec<Word> = c.args[k + 1..].iter().take_while(|w| !matches!(w.text.as_str(), ";" | "+")).filter(|w| w.text != "{}").cloned().collect();
            if inner.is_empty() {
                continue;
            }
            let sub = Cmd::new(&inner, false);
            deletes |= matches!(sub.prog.as_str(), "rm" | "rmdir" | "unlink" | "shred" | "mv");
            self.cmd(cwd, &sub, nest, false);
        }
        if deletes {
            let mut starts: Vec<&Word> = c.args.iter().take_while(|w| !(w.text.starts_with('-') || w.text == "(" || w.text == "!")).collect();
            let dot = Word { text: ".".into(), ..Word::default() };
            if starts.is_empty() {
                starts.push(&dot);
            }
            self.delete(cwd, &starts, false, "find -delete");
        }
    }

    // -- git and gh ---------------------------------------------------------------------------

    fn git(&mut self, cwd: &Option<PathBuf>, c: &Cmd) {
        let mut dirs = vec![];
        let mut i = 0;
        let mut sub_at = None;
        while i < c.args.len() {
            let t = c.args[i].text.as_str();
            match t {
                "-C" => {
                    dirs.extend(c.args.get(i + 1));
                    i += 2;
                }
                "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--super-prefix" | "--config-env" => i += 2,
                _ if t.starts_with("-C") && !t.starts_with("--") => {
                    dirs.push(&c.args[i]);
                    i += 1;
                }
                _ if t.starts_with('-') => i += 1,
                _ => {
                    sub_at = Some(i);
                    break;
                }
            }
        }
        let Some(at) = sub_at else { return };
        let sub = c.args[at].text.as_str();
        let rest = Cmd { prog: "git".into(), args: c.args[at + 1..].to_vec(), stdin_args: false };
        match sub {
            "push" => self.git_push(&rest),
            "clean" => {
                let mut dir = cwd.clone();
                for d in dirs {
                    let t = self.tgt(&dir, d);
                    dir = if t.top_var || t.top_glob { None } else { t.path };
                }
                if !dir.as_deref().is_some_and(|d| !self.outside(d)) {
                    self.hit(NeverItem::DeleteOutside, "git clean outside the worktree");
                }
                let pos = rest.positionals();
                self.delete(&dir, &pos, false, "git clean");
            }
            "config" => {
                let read = rest.has("l", &["get", "get-all", "get-regexp", "list", "get-urlmatch", "show-origin", "show-scope", "name-only"]);
                let set = rest.has("e", &["add", "replace-all", "unset", "unset-all", "edit", "rename-section", "remove-section"]) || rest.positionals().len() >= 2;
                if set && !read {
                    self.hit(NeverItem::SelfGrant, "git config set");
                }
            }
            "tag" => {
                let listing = rest.has("ldvn", &["list", "delete", "verify", "contains", "points-at", "merged", "no-merged"]);
                if !listing && !rest.positionals().is_empty() {
                    self.tagged = true;
                }
            }
            "clone" | "worktree" => {
                let pos = rest.positionals();
                let dest = if sub == "clone" { pos.get(1) } else if pos.first().is_some_and(|w| w.text == "add") { pos.get(1) } else { None };
                if let Some(d) = dest {
                    self.write_words(cwd, vec![(*d).clone()]);
                }
            }
            _ => {}
        }
    }

    fn git_push(&mut self, rest: &Cmd) {
        self.pushed = true;
        let mut force: Option<String> = None;
        let (mut tags, mut all) = (false, false);
        let mut pos: Vec<&str> = vec![];
        let mut after_dd = false;
        let mut i = 0;
        while i < rest.args.len() {
            let t = rest.args[i].text.as_str();
            i += 1;
            if after_dd || !t.starts_with('-') || t == "-" {
                pos.push(t);
            } else if t == "--" {
                after_dd = true;
            } else if let Some(long) = t.strip_prefix("--") {
                let name = long.split('=').next().unwrap_or("");
                match name {
                    "force" | "force-with-lease" | "force-if-includes" | "mirror" | "delete" => {
                        force.get_or_insert(format!("git push --{name}"));
                    }
                    "tags" | "follow-tags" => tags = true,
                    "all" | "branches" => all = true,
                    "push-option" | "repo" | "receive-pack" | "exec" if !long.contains('=') => i += 1,
                    _ => {}
                }
            } else {
                let cluster = &t[1..];
                if cluster.contains('f') {
                    force.get_or_insert("git push -f".into());
                }
                if cluster.contains('d') {
                    force.get_or_insert("git push -d".into());
                }
                if cluster == "o" {
                    i += 1;
                }
            }
        }
        if let Some(why) = force {
            self.hit(NeverItem::ForcePush, why);
        }
        if all {
            self.hit(NeverItem::PushToMain, "git push --all");
        }
        if tags {
            self.hit(NeverItem::Release, "git push --tags");
        }
        let default = self.ctx.default_branch;
        let is_main = |b: &str| {
            let b = b.strip_prefix("refs/heads/").unwrap_or(b);
            b == default || matches!(b, "main" | "master" | "railway-hub" | "origin/HEAD")
        };
        let current = self.ctx.current_branch;
        let from_current = |scan: &mut Self| match current {
            Some(b) if !is_main(b) => {}
            Some(b) => scan.hit(NeverItem::PushToMain, format!("git push from {b}")),
            None => scan.hit(NeverItem::PushToMain, "git push from an unknown branch"),
        };
        let specs: &[&str] = pos.get(1..).unwrap_or(&[]);
        if specs.is_empty() {
            from_current(self);
        }
        for spec in specs {
            let mut s = *spec;
            if let Some(r) = s.strip_prefix('+') {
                self.hit(NeverItem::ForcePush, format!("git push +{}", short(r)));
                s = r;
            }
            let (src, dst) = match s.split_once(':') {
                Some(("", dst)) => {
                    self.hit(NeverItem::ForcePush, format!("git push :{}", short(dst)));
                    ("", dst)
                }
                Some((src, dst)) => (src, dst),
                None => (s, s),
            };
            let dst = dst.trim_start_matches('+');
            if dst == "HEAD" || (dst.is_empty() && src == "HEAD") {
                from_current(self);
            } else if is_main(dst) {
                self.hit(NeverItem::PushToMain, format!("git push to {}", short(dst)));
            }
            let tag_like = {
                let d = dst.strip_prefix('v').unwrap_or(dst);
                d.starts_with(|c: char| c.is_ascii_digit()) && d.contains('.')
            };
            if dst.starts_with("refs/tags/") || tag_like {
                self.hit(NeverItem::Release, format!("git push tag {}", short(dst)));
            }
        }
    }

    fn gh(&mut self, c: &Cmd) {
        let mut pos: Vec<&str> = vec![];
        let mut i = 0;
        while i < c.args.len() {
            let t = c.args[i].text.as_str();
            i += 1;
            if matches!(t, "-R" | "--repo") {
                i += 1;
            } else if !t.starts_with('-') {
                pos.push(t);
            }
        }
        let (group, action) = (pos.first().copied().unwrap_or(""), pos.get(1).copied().unwrap_or(""));
        match (group, action) {
            ("pr", "merge") => self.hit(NeverItem::PushToMain, "gh pr merge"),
            ("release", _) => self.hit(NeverItem::Release, format!("gh release {action}")),
            ("workflow", "run" | "enable" | "disable") => self.hit(NeverItem::Deploy, format!("gh workflow {action}")),
            ("auth" | "secret", _) => self.hit(NeverItem::Secrets, format!("gh {group}")),
            ("api", _) => {
                let method = c.value('X', "method").map(|w| w.text.to_ascii_uppercase());
                let fields = c.has("fF", &["field", "raw-field", "input"]);
                let graphql_read = pos.get(1) == Some(&"graphql") && !c.args.iter().any(|w| w.text.contains("mutation"));
                let writes = match method.as_deref() {
                    Some(m) => m != "GET",
                    None => fields && !graphql_read,
                };
                if writes {
                    self.hit(NeverItem::Deploy, format!("gh api {}", method.as_deref().unwrap_or("POST")));
                }
            }
            _ => {}
        }
    }

    // -- writes and installs ------------------------------------------------------------------

    /// The words a file-writing command writes to.
    fn write_args(&self, c: &Cmd) -> Vec<Word> {
        let pos = c.positionals();
        let word = |t: &str| Word { text: t.to_owned(), ..Word::default() };
        match c.prog.as_str() {
            "cp" | "install" | "mv" | "ln" => {
                let mut out = vec![];
                let target = c.value('t', "target-directory");
                if c.prog == "mv" {
                    out.extend(pos.iter().map(|w| (*w).clone()));
                } else if target.is_none() {
                    out.extend(pos.last().map(|w| (*w).clone()));
                }
                out.extend(target);
                out
            }
            "tee" | "touch" | "mkdir" | "truncate" | "mkfifo" => pos.iter().map(|w| (*w).clone()).collect(),
            "sed" => {
                if !c.has("i", &["in-place"]) {
                    return vec![];
                }
                let script_given = c.has("ef", &["expression", "file"]);
                pos.iter().skip(usize::from(!script_given)).map(|w| (*w).clone()).collect()
            }
            "dd" => c.args.iter().filter_map(|w| w.text.strip_prefix("of=").map(&word)).collect(),
            "curl" => c.value('o', "output").into_iter().collect(),
            "wget" => c.value('O', "output-document").into_iter().chain(c.value('P', "directory-prefix")).collect(),
            _ => vec![],
        }
    }

    fn write_words(&mut self, cwd: &Option<PathBuf>, words: Vec<Word>) {
        for w in &words {
            self.write_word(cwd, w);
        }
    }

    /// Why the command adds or fetches something the lockfile doesn't pin, if it does.
    fn install(&self, cwd: &Option<PathBuf>, c: &Cmd) -> Option<String> {
        let mut pos: Vec<&str> = c.positionals().iter().map(|w| w.text.as_str()).collect();
        if c.prog == "cargo" && pos.first().is_some_and(|p| p.starts_with('+')) {
            pos.remove(0);
        }
        let sub = pos.first().copied().unwrap_or("");
        let more = pos.len() > 1;
        let prog = c.prog.as_str();
        let yes = match prog {
            "npm" => match sub {
                "install" | "i" | "add" | "in" | "ins" => more,
                "update" | "up" | "upgrade" | "exec" | "x" => true,
                "create" | "init" => more,
                _ => false,
            },
            "pnpm" => match sub {
                "add" | "update" | "up" | "upgrade" | "dlx" | "create" => true,
                "install" | "i" => more,
                _ => false,
            },
            "yarn" => matches!(sub, "add" | "global" | "upgrade" | "up" | "upgrade-interactive" | "dlx" | "create"),
            "bun" => match sub {
                "add" | "a" | "update" | "x" | "create" => true,
                "install" | "i" => more,
                _ => false,
            },
            "npx" | "bunx" => self.npx_remote(cwd, c),
            "cargo" => matches!(sub, "add" | "install" | "update" | "binstall" | "upgrade"),
            "pip" | "pip3" | "pip2" => matches!(sub, "install" | "download"),
            "pipx" => matches!(sub, "install" | "run" | "upgrade" | "inject"),
            "python" | "python3" | "python2" => {
                let m = c.args.iter().position(|w| w.text == "-m");
                m.and_then(|k| c.args.get(k + 1)).is_some_and(|w| matches!(w.text.as_str(), "pip" | "pip3")) && c.args.iter().any(|w| w.text == "install")
            }
            "uv" => matches!(sub, "add") || (matches!(sub, "pip" | "tool") && pos.get(1).is_some_and(|s| matches!(*s, "install" | "run"))),
            "uvx" => true,
            "poetry" => matches!(sub, "add" | "update"),
            "brew" => matches!(sub, "install" | "reinstall" | "upgrade" | "tap" | "bundle"),
            "apt" | "apt-get" | "aptitude" | "dnf" | "yum" | "zypper" => matches!(sub, "install" | "upgrade" | "dist-upgrade"),
            "apk" => matches!(sub, "add" | "upgrade"),
            "pacman" => c.has("S", &["sync"]),
            "gem" => matches!(sub, "install" | "update"),
            "go" => matches!(sub, "install" | "get"),
            "conda" | "mamba" | "micromamba" => matches!(sub, "install" | "create" | "update"),
            "rustup" => matches!(sub, "install" | "update" | "toolchain" | "component" | "target" | "self"),
            "volta" | "asdf" | "mise" => matches!(sub, "install"),
            "composer" => matches!(sub, "require" | "update"),
            "bundle" => matches!(sub, "add" | "update"),
            "dotnet" => matches!(sub, "add") || (sub == "tool" && pos.get(1) == Some(&"install")),
            _ => false,
        };
        yes.then(|| format!("{prog} {sub}").trim().to_owned())
    }

    /// `npx <pkg>` fetches unless the package is already in node_modules/.bin.
    fn npx_remote(&self, cwd: &Option<PathBuf>, c: &Cmd) -> bool {
        if c.has("", &["no-install", "no"]) {
            return false;
        }
        if c.value('p', "package").is_some() {
            return true;
        }
        let Some(pkg) = c.positionals().first().map(|w| w.text.clone()) else { return false };
        if pkg.get(1..).is_some_and(|r| r.contains('@')) {
            return true;
        }
        let local = |dir: &Path| dir.join("node_modules/.bin").join(&pkg).exists();
        !(local(&self.work) || cwd.as_deref().is_some_and(local))
    }
}

fn json_strings<'v>(v: &'v Value, depth: usize, out: &mut Vec<&'v str>) {
    if depth > 32 {
        return;
    }
    match v {
        Value::String(s) => out.push(s),
        Value::Array(a) => a.iter().for_each(|v| json_strings(v, depth + 1, out)),
        Value::Object(m) => {
            for (k, v) in m {
                out.push(k);
                json_strings(v, depth + 1, out);
            }
        }
        _ => {}
    }
}
