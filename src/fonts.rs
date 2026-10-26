//! The installed font families, for Settings' font list. WKWebView has no `queryLocalFonts`, so the
//! app asks AppKit through JavaScript for Automation, as T3's Electron build asks Chromium.

use std::process::Command;

const LIST_FAMILIES: &str =
    "ObjC.import('AppKit'); $.NSFontManager.sharedFontManager.availableFontFamilies.js.map((f) => f.js).join('\\n')";

/// Sorted, without duplicates or macOS's dot-prefixed UI faces.
pub fn families() -> Result<Vec<String>, String> {
    let out = Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", LIST_FAMILIES])
        .output()
        .map_err(|e| format!("could not list fonts: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(parse(&String::from_utf8_lossy(&out.stdout)))
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

    #[cfg(target_os = "macos")]
    #[test]
    fn appkit_lists_the_installed_families() {
        let found = families().unwrap();
        assert!(found.iter().any(|f| f == "Menlo"), "{found:?}");
    }
}
