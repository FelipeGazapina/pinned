//! The browser panel's pages: native webviews laid over the panel's viewport, since an iframe
//! can't give a cross-origin page's history, title, devtools, zoom or a screenshot.

use serde::{Deserialize, Serialize};
use std::sync::mpsc;
use std::time::Duration;
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, Webview, WebviewBuilder};
use tauri::{AppHandle, Emitter, EventTarget, LogicalPosition, LogicalSize, Manager, Rect, Runtime, WebviewUrl, Window};

#[derive(Deserialize, Clone, Copy)]
pub struct Bounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "kebab-case")]
pub enum Action {
    Back,
    Forward,
    Reload,
    HardReload,
    Go { url: String },
    Zoom { factor: f64 },
    Devtools,
    ClearCookies,
    ClearCache,
    Appearance { mode: Appearance },
    Mute { on: bool },
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
pub enum Appearance {
    System,
    Light,
    Dark,
}

/// What the panel shows of a page: its address, title, and whether it can step through history.
#[derive(Serialize, Clone, Default)]
pub struct State {
    url: String,
    title: String,
    back: bool,
    forward: bool,
    loading: bool,
    devtools: bool,
    audible: bool,
    /// Whether the page also has a window of its own, from the More menu.
    window: bool,
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum Event {
    State { id: String, state: State },
    Popup { id: String, url: String },
    Key { key: String, code: String, shift: bool, alt: bool },
    /// The page at `url` didn't load: `description` is Chromium's name for why, as T3 shows it, or the
    /// system's own words when there is none.
    Failed { id: String, url: String, code: i64, description: String },
    /// `url` answered a file the page can't show, saved to `path` in Downloads.
    Download { id: String, url: String, path: String, success: bool },
}

/// Chromium's name for a URL loading error, which T3's unreachable page shows and explains.
fn error_name(code: i64) -> Option<&'static str> {
    Some(match code {
        -1003 => "ERR_NAME_NOT_RESOLVED",
        -1004 => "ERR_CONNECTION_REFUSED",
        -1005 => "ERR_CONNECTION_RESET",
        -1001 => "ERR_TIMED_OUT",
        -1009 => "ERR_INTERNET_DISCONNECTED",
        -1007 => "ERR_TOO_MANY_REDIRECTS",
        -1202 | -1203 => "ERR_CERT_AUTHORITY_INVALID",
        -1201 | -1204 => "ERR_CERT_DATE_INVALID",
        -1200 => "ERR_SSL_PROTOCOL_ERROR",
        _ => return None,
    })
}

/// Surface ids are `browser:<random>`, which is already a valid webview label.
fn label(id: &str) -> String {
    format!("preview-{}", id.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_', "-"))
}

fn window_label(id: &str) -> String {
    format!("preview-window-{}", label(id).trim_start_matches("preview-"))
}

fn page<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<Webview<R>, String> {
    app.get_webview(&label(id)).ok_or_else(|| format!("no page {id}"))
}

fn place<R: Runtime>(webview: &Webview<R>, at: Bounds) -> tauri::Result<()> {
    webview.set_bounds(Rect { position: LogicalPosition::new(at.x, at.y).into(), size: LogicalSize::new(at.width.max(1.0), at.height.max(1.0)).into() })
}

fn emit<R: Runtime>(app: &AppHandle<R>, event: Event) {
    let _ = app.emit_to(EventTarget::webview("main"), "browser", event);
}

/// Reads the page's state off the main thread's webview and tells the panel.
fn report<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let (app, id) = (app.clone(), id.to_string());
    let Ok(webview) = page(&app, &id) else { return };
    let url = webview.url().map(|u| u.to_string()).unwrap_or_default();
    let devtools = webview.is_devtools_open();
    let window = app.get_webview_window(&window_label(&id)).is_some();
    #[cfg(target_os = "macos")]
    let _ = webview.with_webview(move |pw| {
        let mut state = native::state(&pw);
        if state.url.is_empty() {
            state.url = url;
        }
        state.devtools = devtools;
        state.window = window;
        emit(&app, Event::State { id, state });
    });
    // Elsewhere the page answers for itself, the Navigation API telling whether it can step back.
    #[cfg(not(target_os = "macos"))]
    let _ = webview.eval_with_callback(
        "({ title: document.title, back: !!window.navigation?.canGoBack, forward: !!window.navigation?.canGoForward, loading: document.readyState !== 'complete' })",
        move |out| {
            let seen: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
            let flag = |key: &str| seen[key].as_bool().unwrap_or(false);
            let title = seen["title"].as_str().unwrap_or_default().to_string();
            let state = State { url: url.clone(), title, back: flag("back"), forward: flag("forward"), loading: flag("loading"), devtools, audible: false, window };
            emit(&app, Event::State { id: id.clone(), state });
        },
    );
}

/// Shows the page `id` at `at`, opening it on `url` the first time.
#[tauri::command]
pub async fn browser_show(app: AppHandle, window: Window, id: String, url: String, at: Bounds) -> Result<(), String> {
    if let Ok(webview) = page(&app, &id) {
        place(&webview, at).map_err(|e| e.to_string())?;
        return webview.show().map_err(|e| e.to_string());
    }
    let url: tauri::Url = url.parse().map_err(|e| format!("not an address: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("not a web address: {url}"));
    }
    let (on_title, on_load, on_popup, on_download) = (app.clone(), app.clone(), app.clone(), app.clone());
    let (title_id, load_id, popup_id, download_id) = (id.clone(), id.clone(), id.clone(), id.clone());
    let saving = std::sync::Mutex::new(std::collections::HashMap::<String, String>::new());
    let builder = WebviewBuilder::new(label(&id), WebviewUrl::External(url))
        .devtools(true)
        .data_store_identifier(*b"pinned-preview\0\0")
        .on_document_title_changed(move |_, _| report(&on_title, &title_id))
        .on_page_load(move |_, payload| {
            if matches!(payload.event(), PageLoadEvent::Started | PageLoadEvent::Finished) {
                report(&on_load, &load_id);
            }
        })
        .on_new_window(move |url, _| {
            emit(&on_popup, Event::Popup { id: popup_id.clone(), url: url.to_string() });
            NewWindowResponse::Deny
        })
        .on_download(move |_, event| {
            match event {
                DownloadEvent::Requested { url, destination } => {
                    saving.lock().unwrap().insert(url.to_string(), destination.display().to_string());
                }
                DownloadEvent::Finished { url, path, success } => {
                    let saved = saving.lock().unwrap().remove(url.as_str());
                    let path = path.map(|p| p.display().to_string()).or(saved).unwrap_or_default();
                    emit(&on_download, Event::Download { id: download_id.clone(), url: url.to_string(), path, success });
                }
                _ => {}
            }
            true
        });
    native::watch_failures(&app);
    let webview = window
        .add_child(builder, LogicalPosition::new(at.x, at.y), LogicalSize::new(at.width.max(1.0), at.height.max(1.0)))
        .map_err(|e| e.to_string())?;
    native::watch_keys(&app);
    let page_id = id.clone();
    webview.with_webview(move |pw| native::remember(&pw, &page_id)).map_err(|e| e.to_string())?;
    report(&app, &id);
    Ok(())
}

#[tauri::command]
pub fn browser_place(app: AppHandle, id: String, at: Bounds) -> Result<(), String> {
    place(&page(&app, &id)?, at).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn browser_hide(app: AppHandle, id: String) -> Result<(), String> {
    match page(&app, &id) {
        Ok(webview) => webview.hide().map_err(|e| e.to_string()),
        Err(_) => Ok(()),
    }
}

/// Closes the pages whose ids are not in `keep`; the panel sends every id it still has.
#[tauri::command]
pub fn browser_close(app: AppHandle, keep: Vec<String>) -> Result<(), String> {
    let keep: Vec<String> = keep.iter().map(|id| label(id)).collect();
    for (name, webview) in app.webviews() {
        if name.starts_with("preview-") && !keep.contains(&name) {
            webview.with_webview(|pw| native::forget(&pw)).map_err(|e| e.to_string())?;
            webview.close().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn browser_state(app: AppHandle, id: String) -> Result<(), String> {
    page(&app, &id)?;
    report(&app, &id);
    Ok(())
}

#[tauri::command]
pub fn browser_do(app: AppHandle, id: String, action: Action) -> Result<(), String> {
    let webview = page(&app, &id)?;
    let result = match action {
        Action::Go { url } => {
            let url: tauri::Url = url.parse().map_err(|e| format!("not an address: {e}"))?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(format!("not a web address: {url}"));
            }
            webview.navigate(url)
        }
        Action::Reload => webview.reload(),
        Action::Zoom { factor } => webview.set_zoom(factor.clamp(0.25, 5.0)),
        // Docked, the inspector would take the page's own small frame, so it gets a window, and one
        // above the app's, which floats over every other window.
        Action::Devtools => {
            if webview.is_devtools_open() {
                webview.close_devtools();
            } else {
                webview.open_devtools();
                let (app, page) = (app.clone(), webview.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(300));
                    let _ = page.with_webview(|pw| native::detach_inspector(&pw));
                    std::thread::sleep(Duration::from_millis(300));
                    let _ = app.run_on_main_thread(native::raise_inspectors);
                });
            }
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        Action::Back => webview.eval("history.back()"),
        #[cfg(not(target_os = "macos"))]
        Action::Forward => webview.eval("history.forward()"),
        #[cfg(not(target_os = "macos"))]
        Action::HardReload => webview.eval("location.reload()"),
        native_action => webview.with_webview(move |pw| native::act(&pw, native_action)),
    };
    result.map_err(|e| e.to_string())?;
    report_soon(&app, &id);
    Ok(())
}

/// Reports the page once what was just asked of it has had time to happen.
fn report_soon<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let (app, id) = (app.clone(), id.to_string());
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        report(&app, &id);
    });
}

/// Runs `js` in the page and answers its JSON result, for the panel's element picker and media mute.
#[tauri::command]
pub async fn browser_eval(app: AppHandle, id: String, js: String) -> Result<serde_json::Value, String> {
    let (tx, rx) = mpsc::channel();
    page(&app, &id)?.eval_with_callback(js, move |out| drop(tx.send(out))).map_err(|e| e.to_string())?;
    let out = tauri::async_runtime::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(5)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "the page didn't answer".to_string())?;
    Ok(serde_json::from_str(&out).unwrap_or(serde_json::Value::Null))
}

/// The page as a PNG, as it shows now.
#[tauri::command]
pub async fn browser_snapshot(app: AppHandle, id: String) -> Result<tauri::ipc::Response, String> {
    let (tx, rx) = mpsc::channel();
    page(&app, &id)?.with_webview(move |pw| native::snapshot(&pw, tx)).map_err(|e| e.to_string())?;
    let png = tauri::async_runtime::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(10)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "the page didn't draw".to_string())??;
    Ok(tauri::ipc::Response::new(png))
}

/// Opens the page `id` at `url` in a window of its own, or closes that window when it is open.
#[tauri::command]
pub async fn browser_window(app: AppHandle, id: String, url: String) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(&window_label(&id)) {
        window.close().map_err(|e| e.to_string())?;
        report_soon(&app, &id);
        return Ok(());
    }
    let url: tauri::Url = url.parse().map_err(|e| format!("not an address: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("not a web address: {url}"));
    }
    let (closed, closed_id) = (app.clone(), id.clone());
    tauri::WebviewWindowBuilder::new(&app, window_label(&id), WebviewUrl::External(url.clone()))
        .title(url.host_str().unwrap_or("Preview"))
        .inner_size(1024.0, 768.0)
        .data_store_identifier(*b"pinned-preview\0\0")
        .devtools(true)
        .build()
        .map_err(|e| e.to_string())?
        .on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                report_soon(&closed, &closed_id);
            }
        });
    report_soon(&app, &id);
    Ok(())
}

/// Writes `bytes` to the Downloads folder under `name`, numbering it when the name is taken.
#[tauri::command]
pub fn save_capture(app: AppHandle, name: String, bytes: Vec<u8>) -> Result<String, String> {
    let dir = app.path().download_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = std::path::Path::new(&name).file_name().ok_or("no file name")?.to_string_lossy().to_string();
    let (stem, ext) = name.rsplit_once('.').unwrap_or((&name, ""));
    let mut path = dir.join(&name);
    for n in 2.. {
        if !path.exists() {
            break;
        }
        path = dir.join(format!("{stem} {n}.{ext}"));
    }
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
pub fn reveal_file(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let opened = pinned_core::process::command("open").arg("-R").arg(&path).spawn();
    #[cfg(windows)]
    let opened = pinned_core::process::command("explorer").arg(format!("/select,{path}")).spawn();
    #[cfg(not(any(target_os = "macos", windows)))]
    let opened = pinned_core::process::command("xdg-open").arg(std::path::Path::new(&path).parent().unwrap_or(std::path::Path::new("/"))).spawn();
    opened.map(drop).map_err(|e| e.to_string())
}

/// Debug builds: presses ⌘`key` in the page the way the window server delivers a key, so the
/// native probe can check the chords the page hands to the panel.
#[tauri::command]
pub fn browser_debug_key(app: AppHandle, id: String, key: String) -> Result<(), String> {
    if !cfg!(debug_assertions) {
        return Err("only debug builds type into pages".into());
    }
    page(&app, &id)?.with_webview(move |pw| native::debug_key(&pw, &key)).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
mod native {
    use super::{Action, Appearance, Event, State};
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{msg_send, sel, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSEventType, NSStatusWindowLevel, NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSBitmapImageFileType, NSBitmapImageRep, NSEvent, NSEventMask, NSEventModifierFlags, NSImage, NSView};
    use objc2_foundation::{NSDate, NSDictionary, NSError, NSObjectProtocol, NSSet, NSString, NSURL};
    use objc2_web_kit::{WKWebView, WKWebsiteDataTypeCookies, WKWebsiteDataTypeDiskCache, WKWebsiteDataTypeFetchCache, WKWebsiteDataTypeMemoryCache};
    use std::ptr::NonNull;
    use std::sync::{mpsc, OnceLock};
    use tauri::webview::PlatformWebview;
    use tauri::AppHandle;

    fn view(pw: &PlatformWebview) -> &WKWebView {
        unsafe { &*(pw.inner() as *const WKWebView) }
    }

    pub fn state(pw: &PlatformWebview) -> State {
        let web = view(pw);
        unsafe {
            State {
                url: web.URL().and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default(),
                title: web.title().map(|s| s.to_string()).unwrap_or_default(),
                back: web.canGoBack(),
                forward: web.canGoForward(),
                loading: web.isLoading(),
                devtools: false,
                audible: web.respondsToSelector(sel!(_isPlayingAudio)) && msg_send![web, _isPlayingAudio],
                window: false,
            }
        }
    }

    pub fn act(pw: &PlatformWebview, action: Action) {
        let web = view(pw);
        unsafe {
            match action {
                Action::Back => drop(web.goBack()),
                Action::Forward => drop(web.goForward()),
                Action::HardReload => drop(web.reloadFromOrigin()),
                Action::ClearCookies => clear(web, &[WKWebsiteDataTypeCookies]),
                Action::ClearCache => clear(web, &[WKWebsiteDataTypeDiskCache, WKWebsiteDataTypeMemoryCache, WKWebsiteDataTypeFetchCache]),
                Action::Appearance { mode } => {
                    let named = match mode {
                        Appearance::System => None,
                        Appearance::Light => NSAppearance::appearanceNamed(NSAppearanceNameAqua),
                        Appearance::Dark => NSAppearance::appearanceNamed(NSAppearanceNameDarkAqua),
                    };
                    web.setAppearance(named.as_deref());
                }
                // WebKit's own page mute, which also silences Web Audio; Safari's tab mute uses it.
                Action::Mute { on } => {
                    if web.respondsToSelector(sel!(_setPageMuted:)) {
                        let _: () = msg_send![web, _setPageMuted: if on { 1usize } else { 0usize }];
                    }
                }
                Action::Go { .. } | Action::Reload | Action::Zoom { .. } | Action::Devtools => {}
            }
        }
    }

    pub fn detach_inspector(pw: &PlatformWebview) {
        unsafe {
            let inspector: Retained<AnyObject> = msg_send![view(pw), _inspector];
            let () = msg_send![&inspector, detach];
        }
    }

    pub fn raise_inspectors() {
        let Some(mtm) = MainThreadMarker::new() else { return };
        for window in NSApplication::sharedApplication(mtm).windows() {
            if window.title().to_string().contains("Inspector") {
                window.setLevel(NSStatusWindowLevel + 1);
                window.makeKeyAndOrderFront(None);
            }
        }
    }

    unsafe fn clear(web: &WKWebView, kinds: &[&NSString]) {
        let types = NSSet::from_slice(kinds);
        let store = web.configuration().websiteDataStore();
        store.removeDataOfTypes_modifiedSince_completionHandler(&types, &NSDate::distantPast(), &RcBlock::new(|| {}));
    }

    pub fn snapshot(pw: &PlatformWebview, tx: mpsc::Sender<Result<Vec<u8>, String>>) {
        let done = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            let png = unsafe { image.as_ref() }
                .and_then(|image| image.TIFFRepresentation())
                .and_then(|tiff| NSBitmapImageRep::imageRepWithData(&tiff))
                .and_then(|rep| unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) })
                .map(|data| data.to_vec());
            let _ = tx.send(png.ok_or_else(|| unsafe { error.as_ref() }.map_or("the page didn't draw".into(), |e| e.localizedDescription().to_string())));
        });
        unsafe { view(pw).takeSnapshotWithConfiguration_completionHandler(None, &done) };
    }

    /// A key chord typed into a page goes to the page's webview, never to the panel's, so the
    /// app's own shortcuts would stop working there. This hands the panel every ⌘ chord the page
    /// would otherwise get, except the editing ones a page needs.
    pub fn watch_keys(app: &AppHandle) {
        static MONITOR: OnceLock<usize> = OnceLock::new();
        let app = app.clone();
        MONITOR.get_or_init(move || {
            let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
                let event_ref = unsafe { event.as_ref() };
                forward(&app, event_ref).then(std::ptr::null_mut).unwrap_or(event.as_ptr())
            });
            let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block) };
            std::mem::forget(monitor);
            1
        });
    }

    fn forward(app: &AppHandle, event: &NSEvent) -> bool {
        let flags = event.modifierFlags();
        if !flags.contains(NSEventModifierFlags::Command) || flags.contains(NSEventModifierFlags::Control) || !in_page(event) {
            return false;
        }
        let key = event.charactersIgnoringModifiers().map(|s| s.to_string()).unwrap_or_default().to_lowercase();
        if matches!(key.as_str(), "c" | "v" | "x" | "a" | "z") {
            return false;
        }
        let shift = flags.contains(NSEventModifierFlags::Shift);
        let code = format!("Key{}", key.to_uppercase());
        super::emit(app, Event::Key { key, code, shift, alt: flags.contains(NSEventModifierFlags::Option) });
        true
    }

    /// Whether the window's first responder sits inside one of the panel's page webviews.
    fn in_page(event: &NSEvent) -> bool {
        let Some(mtm) = MainThreadMarker::new() else { return false };
        let Some(window) = event.window(mtm) else { return false };
        let Some(responder) = window.firstResponder() else { return false };
        let mut at: Option<Retained<NSView>> = responder.downcast::<NSView>().ok();
        while let Some(v) = at {
            if v.isKindOfClass(objc2::class!(WKWebView)) {
                let ptr = Retained::as_ptr(&v) as *const AnyObject as usize;
                return PAGES.lock().unwrap().iter().any(|(p, _)| *p == ptr);
            }
            at = unsafe { v.superview() };
        }
        false
    }

    pub fn debug_key(pw: &PlatformWebview, key: &str) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let web = view(pw);
        let Some(window) = web.window() else { return };
        window.makeFirstResponder(Some(web));
        let code = match key {
            "r" => 15,
            "l" => 37,
            "=" => 24,
            "b" => 11,
            _ => 0,
        };
        let chars = NSString::from_str(key);
        let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            objc2_foundation::NSPoint::new(0.0, 0.0),
            NSEventModifierFlags::Command,
            0.0,
            window.windowNumber(),
            None,
            &chars,
            &chars,
            false,
            code,
        );
        if let Some(event) = event {
            NSApplication::sharedApplication(mtm).postEvent_atStart(&event, false);
        }
    }

    /// The page webviews, by address with their surface id, so a key chord in one can be told from
    /// one in the panel, and a failed load tells which page it was.
    static PAGES: std::sync::Mutex<Vec<(usize, String)>> = std::sync::Mutex::new(Vec::new());
    static APP: OnceLock<AppHandle> = OnceLock::new();

    pub fn remember(pw: &PlatformWebview, id: &str) {
        PAGES.lock().unwrap().push((pw.inner() as usize, id.to_string()));
    }

    pub fn forget(pw: &PlatformWebview) {
        PAGES.lock().unwrap().retain(|(p, _)| *p != pw.inner() as usize);
    }

    /// wry's navigation delegate doesn't hear about failed loads, so its class learns the two
    /// WebKit methods for them before the first page's webview takes it on as delegate; WebKit
    /// reads what a delegate answers once, then.
    pub fn watch_failures(app: &AppHandle) {
        let _ = APP.set(app.clone());
        static ADDED: OnceLock<()> = OnceLock::new();
        ADDED.get_or_init(|| {
            let Some(class) = objc2::runtime::AnyClass::get(c"WryNavigationDelegate") else { return };
            type Failed = unsafe extern "C-unwind" fn(*mut AnyObject, objc2::runtime::Sel, *mut WKWebView, *mut AnyObject, *mut NSError);
            let imp: Failed = failed;
            for name in [sel!(webView:didFailProvisionalNavigation:withError:), sel!(webView:didFailNavigation:withError:)] {
                unsafe {
                    objc2::ffi::class_addMethod(class as *const _ as *mut _, name, std::mem::transmute::<Failed, objc2::runtime::Imp>(imp), c"v@:@@@".as_ptr());
                }
            }
        });
    }

    unsafe extern "C-unwind" fn failed(_: *mut AnyObject, _: objc2::runtime::Sel, web: *mut WKWebView, _: *mut AnyObject, error: *mut NSError) {
        let (Some(web), Some(error), Some(app)) = (unsafe { web.as_ref() }, unsafe { error.as_ref() }, APP.get()) else { return };
        let Some(id) = PAGES.lock().unwrap().iter().find(|(p, _)| *p == web as *const WKWebView as usize).map(|(_, id)| id.clone()) else { return };
        let (domain, code) = (error.domain().to_string(), error.code() as i64);
        // A stop, a new address typed over a loading one, and a response turned into a download all
        // end a load without it failing.
        if (domain == "NSURLErrorDomain" && code == -999) || (domain == "WebKitErrorDomain" && code == 102) {
            return;
        }
        let info = error.userInfo();
        let url = info.objectForKey(&NSString::from_str("NSErrorFailingURLKey"))
            .and_then(|u| u.downcast::<NSURL>().ok())
            .and_then(|u| u.absoluteString())
            .map(|u| u.to_string())
            .unwrap_or_default();
        let description = if domain == "NSURLErrorDomain" { super::error_name(code).map(String::from) } else { None }.unwrap_or_else(|| error.localizedDescription().to_string());
        super::emit(app, Event::Failed { id: id.clone(), url, code, description });
        super::report(app, &id);
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use super::Action;
    use tauri::webview::PlatformWebview;
    use tauri::AppHandle;

    pub fn act(_: &PlatformWebview, _: Action) {}
    pub fn remember(_: &PlatformWebview, _: &str) {}
    pub fn watch_failures(_: &AppHandle) {}
    pub fn forget(_: &PlatformWebview) {}
    pub fn snapshot(_: &PlatformWebview, tx: std::sync::mpsc::Sender<Result<Vec<u8>, String>>) {
        let _ = tx.send(Err("screenshots need macOS".into()));
    }
    pub fn watch_keys(_: &AppHandle) {}
    pub fn debug_key(_: &PlatformWebview, _: &str) {}
    pub fn detach_inspector(_: &PlatformWebview) {}
    pub fn raise_inspectors() {}
}
