use std::sync::{Arc, Mutex};

use tauri::Url;

pub struct FramePermit(pub Arc<Mutex<Option<Url>>>);

pub fn app_document(url: &Url) -> bool {
    matches!(url.scheme(), "tauri" | "asset" | "ipc" | "blob" | "data" | "about")
        || matches!(url.host_str(), Some("tauri.localhost" | "ipc.localhost"))
}

fn loopback_http(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1" | "[::1]"))
}

fn same_frame(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
        && a.path() == b.path()
        && a.query() == b.query()
}

pub fn main_window_allows(url: &Url, permit: &mut Option<Url>) -> bool {
    if app_document(url) {
        return true;
    }
    if let Some(allowed) = permit.clone() {
        if loopback_http(&allowed) && loopback_http(url) && same_frame(&allowed, url) {
            *permit = None;
            return true;
        }
    }
    false
}

pub fn open_in_panel(window: tauri::WebviewWindow, url: &Url) {
    let text = url.as_str().to_string();
    std::thread::spawn(move || {
        // Evaluating inside the navigation callback does not run. Leave that callback first.
        std::thread::sleep(std::time::Duration::from_millis(40));
        let quoted = serde_json::to_string(&text).unwrap_or_else(|_| "\"\"".into());
        let script = format!(
            "(function(){{var u={quoted};if(typeof openLink==='function'&&/^https?:/i.test(u))openLink(u);else if(window.__TAURI__&&window.__TAURI__.core)window.__TAURI__.core.invoke('open_url',{{url:u}});}})()"
        );
        let _ = window.eval(script);
    });
}

pub fn install_main_window(app: &tauri::App, frames: Arc<Mutex<Option<Url>>>) -> tauri::Result<()> {
    let config = app.config().app.windows.iter().find(|window| window.label == "main").cloned().expect("main window");
    let nav_frames = frames.clone();
    let slot = Arc::new(Mutex::new(None));
    let nav_slot = slot.clone();
    let popup_slot = slot.clone();
    let built = tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?
        .on_navigation(move |url| {
            let mut permit = nav_frames.lock().unwrap();
            if main_window_allows(url, &mut permit) {
                return true;
            }
            drop(permit);
            if let Some(window) = nav_slot.lock().unwrap().clone() {
                open_in_panel(window, url);
            }
            false
        })
        .on_new_window(move |url, _| {
            if !app_document(&url) {
                if let Some(window) = popup_slot.lock().unwrap().clone() {
                    open_in_panel(window, &url);
                }
            }
            tauri::webview::NewWindowResponse::Deny
        })
        .build()?;
    *slot.lock().unwrap() = Some(built);
    Ok(())
}

#[tauri::command]
pub fn permit_frame_navigation(permit: tauri::State<'_, FramePermit>, url: String) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|e| e.to_string())?;
    if !loopback_http(&parsed) {
        return Err("not a loopback page".into());
    }
    *permit.0.lock().unwrap() = Some(parsed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn the_app_origin_stays_in_the_main_window() {
        let mut permit = None;
        assert!(main_window_allows(&url("tauri://localhost/"), &mut permit));
        assert!(main_window_allows(&url("tauri://localhost/index.html"), &mut permit));
        assert!(main_window_allows(&url("http://tauri.localhost/"), &mut permit));
        assert!(main_window_allows(&url("https://tauri.localhost/assets/index.html"), &mut permit));
        assert!(main_window_allows(&url("ipc://localhost/"), &mut permit));
        assert!(main_window_allows(&url("blob:tauri://localhost/uuid"), &mut permit));
        assert!(main_window_allows(&url("data:text/plain,hi"), &mut permit));
    }

    #[test]
    fn an_external_page_cannot_replace_the_main_window() {
        let mut permit = None;
        assert!(!main_window_allows(&url("https://example.com/notes"), &mut permit));
        assert!(!main_window_allows(&url("http://127.0.0.1:8874/nav?s=bare-body"), &mut permit));
        assert!(!main_window_allows(&url("http://localhost:3000/"), &mut permit));
    }

    #[test]
    fn a_loopback_frame_is_allowed_once() {
        let mut permit = Some(url("http://127.0.0.1:9/files/a"));
        assert!(!main_window_allows(&url("https://example.com/notes"), &mut permit));
        assert!(permit.is_some());
        assert!(main_window_allows(&url("http://127.0.0.1:9/files/a"), &mut permit));
        assert!(permit.is_none());
        assert!(!main_window_allows(&url("http://127.0.0.1:9/files/a"), &mut permit));
    }
}
