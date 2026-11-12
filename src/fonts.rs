//! The installed font families, for Settings' font list. WKWebView has no `queryLocalFonts`, so the
//! app asks AppKit through JavaScript for Automation, as T3's Electron build asks Chromium. On
//! Linux, WebKitGTK has none either, and fontconfig lists them.

#[cfg(not(target_os = "linux"))]
const LIST_FAMILIES: &str =
    "ObjC.import('AppKit'); $.NSFontManager.sharedFontManager.availableFontFamilies.js.map((f) => f.js).join('\\n')";

/// Sorted, without duplicates or macOS's dot-prefixed UI faces.
#[cfg(not(target_os = "linux"))]
pub fn families() -> Result<Vec<String>, String> {
    let out = pinned_core::process::command("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", LIST_FAMILIES])
        .output()
        .map_err(|e| format!("could not list fonts: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(parse(&String::from_utf8_lossy(&out.stdout)))
}

/// fontconfig's families, sorted and without duplicates. A font with names in several languages
/// lists them on one line, comma-separated; the first is the one CSS matches best.
#[cfg(target_os = "linux")]
pub fn families() -> Result<Vec<String>, String> {
    let out = pinned_core::process::command("fc-list")
        .args([":", "family"])
        .output()
        .map_err(|e| format!("could not list fonts with fc-list: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(parse(&first_names(&String::from_utf8_lossy(&out.stdout))))
}

#[cfg(any(target_os = "linux", test))]
fn first_names(listing: &str) -> String {
    listing.lines().map(|line| line.split(',').next().unwrap_or_default().replace("\\-", "-")).collect::<Vec<_>>().join("\n")
}

fn parse(listing: &str) -> Vec<String> {
    let mut names: Vec<String> = listing.lines().map(str::trim).filter(|n| !n.is_empty() && !n.starts_with('.')).map(String::from).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

#[tauri::command]
pub async fn font_families() -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(families).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_listing_drops_ui_faces_and_repeats_and_sorts() {
        assert_eq!(parse("Menlo\n.SF NS\narial\nGeorgia\n\nMenlo\n"), ["arial", "Georgia", "Menlo"]);
    }

    #[test]
    fn fontconfig_lines_give_their_first_name() {
        let listing = "DejaVu Sans,DejaVu Sans Light\nNoto Sans CJK JP,Noto Sans CJK JP Regular\nDejaVu Sans\nUbuntu Mono\nFira\\-Code\n";
        assert_eq!(parse(&first_names(listing)), ["DejaVu Sans", "Fira-Code", "Noto Sans CJK JP", "Ubuntu Mono"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fontconfig_lists_the_installed_families() {
        let found = families().unwrap();
        assert!(!found.is_empty() && found.iter().all(|f| !f.contains(',')), "{found:?}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn appkit_lists_the_installed_families() {
        let found = families().unwrap();
        assert!(found.iter().any(|f| f == "Menlo"), "{found:?}");
    }
}
