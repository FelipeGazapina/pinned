//! A project's icon, found where T3 Code's ProjectFaviconResolver looks, as a data URL.

use base64::Engine;
use std::path::Path;

const LIMIT: u64 = 512 * 1024;
const CANDIDATES: [&str; 23] = [
    "favicon.svg",
    "favicon.ico",
    "favicon.png",
    "public/favicon.svg",
    "public/favicon.ico",
    "public/favicon.png",
    "app/favicon.ico",
    "app/favicon.png",
    "app/icon.svg",
    "app/icon.png",
    "app/icon.ico",
    "src/favicon.ico",
    "src/favicon.svg",
    "src/app/favicon.ico",
    "src/app/icon.svg",
    "src/app/icon.png",
    "assets/icon.svg",
    "assets/icon.png",
    "assets/logo.svg",
    "assets/logo.png",
    ".idea/icon.svg",
    "Resources/AppIcon.icns",
    "AppIcon.icns",
];
const SOURCES: [&str; 7] = ["index.html", "public/index.html", "app/routes/__root.tsx", "src/routes/__root.tsx", "app/root.tsx", "src/root.tsx", "src/index.html"];

pub fn of(project: &Path) -> Option<String> {
    let found = CANDIDATES.iter().find_map(|name| data_url(project, name));
    found.or_else(|| SOURCES.iter().find_map(|source| icon_href(&std::fs::read_to_string(project.join(source)).ok()?).and_then(|href| {
        let href = href.trim_start_matches('/');
        data_url(project, &format!("public/{href}")).or_else(|| data_url(project, href))
    })))
}

/// The file at `relative`, inside `project`, as a data URL its page can draw.
fn data_url(project: &Path, relative: &str) -> Option<String> {
    if relative.split('/').any(|part| part == ".." || part.is_empty()) {
        return None;
    }
    let file = project.join(relative);
    let meta = std::fs::metadata(&file).ok()?;
    if !meta.is_file() || meta.len() > LIMIT {
        return None;
    }
    let mut bytes = std::fs::read(&file).ok()?;
    let ext = relative.rsplit('.').next()?.to_ascii_lowercase();
    let mime = match ext.as_str() {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "icns" => {
            bytes = icns_png(&bytes)?;
            "image/png"
        }
        _ => return None,
    };
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// The smallest PNG an `.icns` holds of 32px or more, since a page can't draw `.icns`.
fn icns_png(icns: &[u8]) -> Option<Vec<u8>> {
    const PNG: &[u8] = b"\x89PNG";
    if !icns.starts_with(b"icns") {
        return None;
    }
    let mut pngs = Vec::new();
    let mut at = 8;
    while at + 8 <= icns.len() {
        let len = u32::from_be_bytes(icns[at + 4..at + 8].try_into().ok()?) as usize;
        if len < 8 || at + len > icns.len() {
            break;
        }
        let data = &icns[at + 8..at + len];
        if data.starts_with(PNG) {
            pngs.push((matches!(&icns[at..at + 4], b"icp4" | b"ic11"), data));
        }
        at += len;
    }
    // 16px ones last, then the smallest.
    pngs.sort_by_key(|(small, data)| (*small, data.len()));
    let best = pngs.first().map(|(_, data)| *data);
    best.map(<[u8]>::to_vec)
}

/// The `href` of a page's `<link rel="icon">`, or of an object with `rel: "icon"` as a router root declares it.
fn icon_href(source: &str) -> Option<String> {
    let lower = source.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("<link") {
        let start = from + i;
        let tag = &source[start..start + lower[start..].find('>')?];
        let tag_lower = tag.to_ascii_lowercase();
        if tag_lower.contains("rel=\"icon\"") || tag_lower.contains("rel='icon'") || tag_lower.contains("shortcut icon") {
            if let Some(href) = attribute(tag, "href=") {
                return Some(href);
            }
        }
        from = start + 5;
    }
    source.split('}').find(|run| {
        let run = run.to_ascii_lowercase();
        run.contains("rel: \"icon\"") || run.contains("rel: 'icon'") || run.contains("rel:\"icon\"")
    }).and_then(|run| attribute(run, "href:"))
}

fn attribute(text: &str, name: &str) -> Option<String> {
    let rest = text[text.find(name)? + name.len()..].trim_start();
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value: String = rest[1..].chars().take_while(|c| *c != quote && *c != '?').collect();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_known_file_wins_then_a_pages_link() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(of(dir.path()), None);
        std::fs::write(dir.path().join("index.html"), r#"<head><link rel="stylesheet" href="a.css"><link href="/logo.svg?v=2" rel="icon"></head>"#).unwrap();
        std::fs::create_dir(dir.path().join("public")).unwrap();
        std::fs::write(dir.path().join("public/logo.svg"), "<svg/>").unwrap();
        assert_eq!(of(dir.path()).as_deref(), Some("data:image/svg+xml;base64,PHN2Zy8+"));
        std::fs::write(dir.path().join("favicon.png"), b"\x89PNG").unwrap();
        assert_eq!(of(dir.path()).as_deref(), Some("data:image/png;base64,iVBORw=="));
    }

    #[test]
    fn a_router_root_declares_its_icon_in_an_object() {
        assert_eq!(icon_href(r#"links: [{ rel: "stylesheet", href: "/app.css" }, { rel: "icon", href: "/favicon.ico" }]"#).as_deref(), Some("/favicon.ico"));
        assert_eq!(icon_href("<link rel=\"apple-touch-icon\" href=\"/a.png\">"), None);
    }

    #[test]
    fn an_icns_gives_its_png() {
        let entry = |kind: &[u8], data: &[u8]| [kind, &((data.len() + 8) as u32).to_be_bytes(), data].concat();
        let body = [entry(b"icp4", b"\x89PNG small"), entry(b"ic12", b"\x89PNG retina"), entry(b"ic07", b"\x89PNG big one")].concat();
        let icns = [b"icns".as_slice(), &((body.len() + 8) as u32).to_be_bytes(), &body].concat();
        assert_eq!(icns_png(&icns).as_deref(), Some(b"\x89PNG retina".as_slice()));
        assert_eq!(icns_png(b"nope"), None);
    }
}
