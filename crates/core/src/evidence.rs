//! `pinned evidence`: posts a proof video on a pull request without committing it. GitHub has no
//! API that puts a video inline in a comment, so the video becomes an asset of one prerelease and
//! the comment links it.

use serde::Deserialize;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

pub const SUBCOMMAND: &str = "evidence";
pub const TAG: &str = "pinned-evidence";
const USAGE: &str = "usage: pinned evidence <video-file> --caption \"<what the video shows>\" [--pr <number>]";

#[derive(Debug, PartialEq)]
struct Args {
    video: PathBuf,
    caption: String,
    pr: Option<u64>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let (mut video, mut caption, mut pr) = (None, None, None);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--caption" => caption = args.next().filter(|c| !c.trim().is_empty()).cloned(),
            "--pr" => pr = Some(args.next().and_then(|n| n.trim_start_matches('#').parse().ok()).ok_or(USAGE)?),
            _ if video.is_none() && !arg.starts_with("--") => video = Some(PathBuf::from(arg)),
            _ => return Err(USAGE.into()),
        }
    }
    Ok(Args { video: video.ok_or(USAGE)?, caption: caption.ok_or(USAGE)?, pr })
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Repo {
    name_with_owner: String,
    default_branch_ref: Branch,
}

#[derive(Deserialize, Debug, PartialEq)]
struct Branch {
    name: String,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Pr {
    number: u64,
    head_ref_name: String,
}

#[derive(Deserialize)]
struct Comment {
    html_url: String,
    body: String,
}

/// Marks the comment a video file already produced, so a rerun finds it instead of posting again.
fn marker(file: &str) -> String {
    format!("<!-- pinned-evidence: {file} -->")
}

/// The comment already posted for `file`, from `gh api --jq '.[] | {html_url, body}'` lines.
fn posted(lines: &str, file: &str) -> Option<String> {
    let marker = marker(file);
    lines.lines().filter_map(|l| serde_json::from_str::<Comment>(l).ok()).find(|c| c.body.contains(&marker)).map(|c| c.html_url)
}

/// Branch names hold slashes and other characters an asset name can't.
fn asset_name(branch: &str, unix: u64, ext: &str) -> String {
    let branch: String = branch.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' { c } else { '-' }).collect();
    format!("{}-{unix}.{ext}", branch.trim_matches('-'))
}

fn asset_url(repo: &str, asset: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{TAG}/{asset}")
}

fn comment_body(caption: &str, url: &str, asset: &str, file: &str) -> String {
    format!(
        "## Proof\n\n{}\n\n[{asset}]({url})\n\nThe video is an asset of this repository's `{TAG}` prerelease, so it opens or downloads instead of playing here. Nothing was committed.\n\n{}\n",
        caption.trim(),
        marker(file)
    )
}

/// The programs the subcommand runs, found on `path` the way a shell finds them.
struct Tools {
    path: OsString,
}

impl Tools {
    fn find(&self, program: &str) -> Option<PathBuf> {
        std::env::split_paths(&self.path).map(|dir| dir.join(program)).find(|p| p.is_file())
    }

    /// Runs `program` and returns its stdout, or its stderr as the error.
    fn run<S: AsRef<OsStr>>(&self, program: &str, args: &[S]) -> Result<String, String> {
        let out = Command::new(program).args(args).env("PATH", &self.path).output().map_err(|e| format!("could not run {program}: {e}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        if out.status.success() {
            Ok(stdout)
        } else {
            Err(if stderr.is_empty() { stdout } else { stderr })
        }
    }

    fn gh_json<T: for<'de> Deserialize<'de>>(&self, args: &[&str]) -> Result<T, String> {
        let out = self.run("gh", args)?;
        serde_json::from_str(&out).map_err(|e| format!("gh {} answered {out:?}: {e}", args.join(" ")))
    }
}

pub fn run(args: &[String]) -> Result<String, String> {
    let tools = Tools { path: std::env::var_os("PATH").unwrap_or_default() };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    post(parse_args(args)?, &tools, now)
}

fn post(args: Args, tools: &Tools, now: u64) -> Result<String, String> {
    let file = args.video.file_name().and_then(|f| f.to_str()).filter(|_| args.video.is_file());
    let file = file.ok_or(format!("{} is not a video file", args.video.display()))?.to_owned();
    tools.find("gh").ok_or("gh is not installed: install the GitHub CLI from https://cli.github.com")?;
    tools.run("gh", &["auth", "status"]).map_err(|e| format!("gh is not signed in: run `gh auth login`.\n{e}"))?;
    let repo: Repo = tools.gh_json(&["repo", "view", "--json", "nameWithOwner,defaultBranchRef"])?;
    let number = args.pr.map(|n| n.to_string());
    let mut view = vec!["pr", "view"];
    view.extend(number.as_deref());
    view.extend(["--json", "number,headRefName"]);
    let pr: Pr = tools.gh_json(&view).map_err(|e| match args.pr {
        Some(n) => format!("no pull request #{n} in {}: {e}", repo.name_with_owner),
        None => format!("no pull request for this branch: open one with `gh pr create` first, or pass --pr <number>.\n{e}"),
    })?;

    let comments = format!("repos/{}/issues/{}/comments", repo.name_with_owner, pr.number);
    let lines = tools.run("gh", &["api", "--paginate", &comments, "--jq", ".[] | {html_url, body}"])?;
    if let Some(url) = posted(&lines, &file) {
        return Ok(url);
    }

    static STAGINGS: AtomicU32 = AtomicU32::new(0);
    let n = STAGINGS.fetch_add(1, Ordering::Relaxed);
    let staging = std::env::temp_dir().join(format!("pinned-evidence-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let result = upload_and_comment(&args, tools, &repo, &pr, &file, &staging, now);
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn upload_and_comment(args: &Args, tools: &Tools, repo: &Repo, pr: &Pr, file: &str, staging: &Path, now: u64) -> Result<String, String> {
    let ext = args.video.extension().and_then(|e| e.to_str()).unwrap_or("webm").to_ascii_lowercase();
    let mp4 = staging.join(asset_name(&pr.head_ref_name, now, "mp4"));
    // An mp4 opens in the browser's player on more systems than a webm.
    let video = if ext == "webm" && tools.find("ffmpeg").is_some() {
        let input = args.video.as_os_str();
        let ffmpeg = [OsStr::new("-v"), "error".as_ref(), "-y".as_ref(), "-i".as_ref(), input, "-c:v".as_ref(), "libx264".as_ref()];
        let rest = ["-pix_fmt".as_ref(), "yuv420p".as_ref(), "-movflags".as_ref(), "+faststart".as_ref(), mp4.as_os_str()];
        tools.run("ffmpeg", &[&ffmpeg[..], &rest[..]].concat()).map_err(|e| format!("ffmpeg could not convert {file}: {e}"))?;
        mp4
    } else {
        let copy = staging.join(asset_name(&pr.head_ref_name, now, &ext));
        std::fs::copy(&args.video, &copy).map_err(|e| e.to_string())?;
        copy
    };
    let asset = video.file_name().unwrap().to_string_lossy().into_owned();

    if let Err(e) = tools.run("gh", &["release", "view", TAG, "--json", "tagName"]) {
        if !e.contains("not found") {
            return Err(format!("could not read release {TAG}: {e}"));
        }
        let notes = "Proof videos that pinned posted on pull requests. Each asset is linked from a pull request comment.";
        let create = ["release", "create", TAG, "--prerelease", "--title", TAG, "--notes", notes, "--target", &repo.default_branch_ref.name];
        tools.run("gh", &create).map_err(|e| format!("could not create release {TAG}: {e}"))?;
    }
    let upload = [OsStr::new("release"), "upload".as_ref(), TAG.as_ref(), video.as_os_str(), "--clobber".as_ref()];
    tools.run("gh", &upload).map_err(|e| format!("uploading {asset} to release {TAG} failed: {e}"))?;

    let body = staging.join("comment.md");
    std::fs::write(&body, comment_body(&args.caption, &asset_url(&repo.name_with_owner, &asset), &asset, file)).map_err(|e| e.to_string())?;
    let number = pr.number.to_string();
    let comment = [OsStr::new("pr"), "comment".as_ref(), number.as_ref(), "--body-file".as_ref(), body.as_os_str()];
    tools.run("gh", &comment).map_err(|e| format!("could not comment on pull request #{}: {e}", pr.number))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn args_name_the_video_its_caption_and_maybe_the_pr() {
        assert_eq!(
            parse_args(&strings(&["v.webm", "--caption", "Saves the form.", "--pr", "#42"])),
            Ok(Args { video: "v.webm".into(), caption: "Saves the form.".into(), pr: Some(42) })
        );
        assert_eq!(parse_args(&strings(&["--caption", "c", "v.webm"])), Ok(Args { video: "v.webm".into(), caption: "c".into(), pr: None }));
        for bad in [&["v.webm"][..], &["--caption", "c"], &["v.webm", "--caption", " "], &["v.webm", "--caption", "c", "--pr", "x"], &["a", "b", "--caption", "c"]] {
            assert_eq!(parse_args(&strings(bad)), Err(USAGE.into()), "{bad:?}");
        }
    }

    #[test]
    fn gh_answers_parse_into_the_repo_and_the_pr() {
        let repo: Repo = serde_json::from_str(r#"{"defaultBranchRef":{"name":"main"},"nameWithOwner":"acme/app"}"#).unwrap();
        assert_eq!(repo, Repo { name_with_owner: "acme/app".into(), default_branch_ref: Branch { name: "main".into() } });
        let pr: Pr = serde_json::from_str(r#"{"headRefName":"eng-7/fix-export","number":42}"#).unwrap();
        assert_eq!(pr, Pr { number: 42, head_ref_name: "eng-7/fix-export".into() });
    }

    #[test]
    fn assets_are_named_after_the_branch_and_the_time() {
        assert_eq!(asset_name("eng-7/fix export", 1792400050, "mp4"), "eng-7-fix-export-1792400050.mp4");
        assert_eq!(asset_name("/weird/", 1, "webm"), "weird-1.webm");
    }

    #[test]
    fn the_comment_links_the_asset_under_a_proof_heading() {
        let url = asset_url("acme/app", "eng-7-1.mp4");
        assert_eq!(url, "https://github.com/acme/app/releases/download/pinned-evidence/eng-7-1.mp4");
        assert_eq!(
            comment_body(" The export downloads.\nNo 500 in the log. ", &url, "eng-7-1.mp4", "page@ab.webm"),
            "## Proof\n\nThe export downloads.\nNo 500 in the log.\n\n[eng-7-1.mp4](https://github.com/acme/app/releases/download/pinned-evidence/eng-7-1.mp4)\n\n\
             The video is an asset of this repository's `pinned-evidence` prerelease, so it opens or downloads instead of playing here. Nothing was committed.\n\n\
             <!-- pinned-evidence: page@ab.webm -->\n"
        );
    }

    #[test]
    fn a_file_already_posted_on_the_pr_is_found_by_its_marker() {
        let lines = r###"{"body":"LGTM","html_url":"https://github.com/acme/app/pull/42#issuecomment-1"}
{"body":"## Proof\n\nx\n\n<!-- pinned-evidence: page@ab.webm -->\n","html_url":"https://github.com/acme/app/pull/42#issuecomment-2"}"###;
        assert_eq!(posted(lines, "page@ab.webm"), Some("https://github.com/acme/app/pull/42#issuecomment-2".into()));
        assert_eq!(posted(lines, "other.webm"), None);
        assert_eq!(posted("", "page@ab.webm"), None);
    }

    /// A fake `gh` (and optionally `ffmpeg`) first on PATH, which logs each call and keeps the
    /// release and the PR's comments in files, as GitHub would between runs.
    struct Fake {
        dir: PathBuf,
    }

    const FAKE_GH: &str = r#"#!/bin/sh
D="$(dirname "$0")"
echo "gh $*" >> "$D/calls"
case "$1 $2" in
  "auth status") [ -f "$D/signed-out" ] && { echo "You are not logged into any GitHub hosts." >&2; exit 1; }; exit 0 ;;
  "repo view") echo '{"defaultBranchRef":{"name":"main"},"nameWithOwner":"acme/app"}' ;;
  "pr view") [ -f "$D/no-pr" ] && { echo 'no pull requests found for branch "eng-7/fix"' >&2; exit 1; }
    echo '{"headRefName":"eng-7/fix","number":42}' ;;
  "api --paginate") cat "$D/comments" 2>/dev/null || true ;;
  "release view") [ -f "$D/release" ] || { echo "release not found" >&2; exit 1; } ;;
  "release create") touch "$D/release" ;;
  "release upload") [ -f "$D/upload-fails" ] && { echo "HTTP 422: Validation Failed" >&2; exit 1; }; basename "$4" >> "$D/assets" ;;
  "pr comment") /usr/bin/python3 -c 'import json,sys; print(json.dumps({"body": open(sys.argv[1]).read(), "html_url": "https://github.com/acme/app/pull/42#issuecomment-9"}))' "$5" >> "$D/comments"
    echo "https://github.com/acme/app/pull/42#issuecomment-9" ;;
  *) echo "unexpected gh $*" >&2; exit 2 ;;
esac
"#;

    const FAKE_FFMPEG: &str = "#!/bin/sh\necho \"ffmpeg $*\" >> \"$(dirname \"$0\")/calls\"\nfor last; do :; done\ntouch \"$last\"\n";

    impl Fake {
        fn new(name: &str, ffmpeg: bool) -> Fake {
            use std::os::unix::fs::PermissionsExt;
            let dir = std::env::temp_dir().join(format!("pinned-fake-gh-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let mut tools = vec![("gh", FAKE_GH)];
            if ffmpeg {
                tools.push(("ffmpeg", FAKE_FFMPEG));
            }
            for (tool, script) in tools {
                std::fs::write(dir.join(tool), script).unwrap();
                std::fs::set_permissions(dir.join(tool), std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            std::fs::write(dir.join("page@ab.webm"), b"webm").unwrap();
            Fake { dir }
        }

        fn post(&self, extra: &[&str]) -> Result<String, String> {
            let video = self.dir.join("page@ab.webm").display().to_string();
            let mut args = strings(&[&video, "--caption", "The export downloads."]);
            args.extend(strings(extra));
            let tools = Tools { path: std::env::join_paths([self.dir.as_path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap() };
            post(parse_args(&args)?, &tools, 1792400050)
        }

        fn calls(&self) -> Vec<String> {
            let calls = std::fs::read_to_string(self.dir.join("calls")).unwrap_or_default();
            calls.lines().map(|l| l.replace(&self.dir.display().to_string(), "<fake>")).map(|l| redact_staging(&l)).collect()
        }

        fn touch(&self, file: &str) {
            std::fs::write(self.dir.join(file), "").unwrap();
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn redact_staging(line: &str) -> String {
        line.split(' ').map(|w| if w.contains("pinned-evidence-") { "<staging>/".to_owned() + w.rsplit('/').next().unwrap() } else { w.to_owned() }).collect::<Vec<_>>().join(" ")
    }

    const LOOKUP: [&str; 4] = [
        "gh auth status",
        "gh repo view --json nameWithOwner,defaultBranchRef",
        "gh pr view --json number,headRefName",
        "gh api --paginate repos/acme/app/issues/42/comments --jq .[] | {html_url, body}",
    ];

    #[test]
    fn the_first_run_creates_the_release_uploads_once_and_comments_once_and_a_rerun_posts_nothing() {
        let fake = Fake::new("first", true);
        assert_eq!(fake.post(&[]), Ok("https://github.com/acme/app/pull/42#issuecomment-9".into()));
        let mut expected = LOOKUP.map(String::from).to_vec();
        expected.extend([
            "ffmpeg -v error -y -i <fake>/page@ab.webm -c:v libx264 -pix_fmt yuv420p -movflags +faststart <staging>/eng-7-fix-1792400050.mp4".into(),
            "gh release view pinned-evidence --json tagName".into(),
            "gh release create pinned-evidence --prerelease --title pinned-evidence --notes Proof videos that pinned posted on pull requests. Each asset is linked from a pull request comment. --target main".into(),
            "gh release upload pinned-evidence <staging>/eng-7-fix-1792400050.mp4 --clobber".into(),
            "gh pr comment 42 --body-file <staging>/comment.md".into(),
        ]);
        assert_eq!(fake.calls(), expected);
        assert_eq!(std::fs::read_to_string(fake.dir.join("assets")).unwrap(), "eng-7-fix-1792400050.mp4\n");
        let comments = std::fs::read_to_string(fake.dir.join("comments")).unwrap();
        assert!(comments.contains("## Proof\\n\\nThe export downloads.\\n\\n[eng-7-fix-1792400050.mp4](https://github.com/acme/app/releases/download/pinned-evidence/eng-7-fix-1792400050.mp4)"));

        std::fs::remove_file(fake.dir.join("calls")).unwrap();
        assert_eq!(fake.post(&[]), Ok("https://github.com/acme/app/pull/42#issuecomment-9".into()));
        assert_eq!(fake.calls(), LOOKUP);
        assert_eq!(std::fs::read_to_string(fake.dir.join("comments")).unwrap().lines().count(), 1);
    }

    #[test]
    fn without_ffmpeg_the_webm_is_uploaded_to_the_existing_release_of_the_named_pr() {
        let fake = Fake::new("webm", false);
        fake.touch("release");
        assert!(fake.post(&["--pr", "42"]).is_ok());
        let calls = fake.calls();
        assert_eq!(calls[2], "gh pr view 42 --json number,headRefName");
        assert_eq!(
            calls[4..],
            [
                "gh release view pinned-evidence --json tagName",
                "gh release upload pinned-evidence <staging>/eng-7-fix-1792400050.webm --clobber",
                "gh pr comment 42 --body-file <staging>/comment.md",
            ]
        );
    }

    #[test]
    fn failures_say_what_to_do() {
        let fake = Fake::new("failures", false);
        fake.touch("upload-fails");
        assert_eq!(fake.post(&[]), Err("uploading eng-7-fix-1792400050.webm to release pinned-evidence failed: HTTP 422: Validation Failed".into()));
        assert!(!fake.calls().iter().any(|c| c.starts_with("gh pr comment")));
        fake.touch("no-pr");
        assert_eq!(
            fake.post(&[]),
            Err("no pull request for this branch: open one with `gh pr create` first, or pass --pr <number>.\nno pull requests found for branch \"eng-7/fix\"".into())
        );
        fake.touch("signed-out");
        assert_eq!(fake.post(&[]), Err("gh is not signed in: run `gh auth login`.\nYou are not logged into any GitHub hosts.".into()));
        let tools = Tools { path: OsString::from("/nonexistent") };
        let args = Args { video: fake.dir.join("page@ab.webm"), caption: "c".into(), pr: None };
        assert_eq!(post(args, &tools, 1), Err("gh is not installed: install the GitHub CLI from https://cli.github.com".into()));
        let args = Args { video: fake.dir.join("missing.webm"), caption: "c".into(), pr: None };
        assert_eq!(post(args, &tools, 1), Err(format!("{} is not a video file", fake.dir.join("missing.webm").display())));
    }
}
