//! The browser panel's pages: native webviews laid over the panel's viewport, since an iframe
//! can't give a cross-origin page's history, title, devtools, zoom or a screenshot.

use serde::{Deserialize, Serialize};
use std::sync::mpsc;
use std::time::Duration;
use tauri::webview::{NewWindowResponse, PageLoadEvent, Webview, WebviewBuilder};
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
    Stop,
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
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum Event {
    State { id: String, state: State },
    Popup { id: String, url: String },
    Key { key: String, code: String, shift: bool, alt: bool },
}

/// Surface ids are `browser:<random>`, which is already a valid webview label.
fn label(id: &str) -> String {
    format!("preview-{}", id.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_', "-"))
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
    let _ = webview.with_webview(move |pw| {
        let mut state = native::state(&pw).unwrap_or_default();
        if state.url.is_empty() {
            state.url = url;
        }
        state.devtools = devtools;
        emit(&app, Event::State { id, state });
    });
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
    let (on_title, on_load, on_popup) = (app.clone(), app.clone(), app.clone());
    let (title_id, load_id, popup_id) = (id.clone(), id.clone(), id.clone());
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
        });
    let webview = window
        .add_child(builder, LogicalPosition::new(at.x, at.y), LogicalSize::new(at.width.max(1.0), at.height.max(1.0)))
        .map_err(|e| e.to_string())?;
    native::watch_keys(&app);
    webview.with_webview(|pw| native::remember(&pw)).map_err(|e| e.to_string())?;
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
        #[cfg(not(target_os = "macos"))]
        Action::Stop => webview.eval("window.stop()"),
        native_action => webview.with_webview(move |pw| native::act(&pw, native_action)),
    };
    result.map_err(|e| e.to_string())?;
    let (app, later) = (app.clone(), id.clone());
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        report(&app, &later);
    });
    Ok(())
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

/// Opens `url` in a window of its own.
#[tauri::command]
pub async fn browser_window(app: AppHandle, url: String) -> Result<(), String> {
    let url: tauri::Url = url.parse().map_err(|e| format!("not an address: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("not a web address: {url}"));
    }
    let n = app.webview_windows().keys().filter(|l| l.starts_with("preview-window-")).count();
    tauri::WebviewWindowBuilder::new(&app, format!("preview-window-{n}"), WebviewUrl::External(url.clone()))
        .title(url.host_str().unwrap_or("Preview"))
        .inner_size(1024.0, 768.0)
        .data_store_identifier(*b"pinned-preview\0\0")
        .devtools(true)
        .build()
        .map(drop)
        .map_err(|e| e.to_string())
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
    let opened = std::process::Command::new("open").arg("-R").arg(&path).spawn();
    #[cfg(not(target_os = "macos"))]
    let opened = std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn();
    opened.map(drop).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
mod native {
    use super::{Action, Appearance, Event, State};
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{msg_send, sel, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSStatusWindowLevel, NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSBitmapImageFileType, NSBitmapImageRep, NSEvent, NSEventMask, NSEventModifierFlags, NSImage, NSView};
    use objc2_foundation::{NSDate, NSDictionary, NSError, NSObjectProtocol, NSSet, NSString};
    use objc2_web_kit::{WKWebView, WKWebsiteDataTypeCookies, WKWebsiteDataTypeDiskCache, WKWebsiteDataTypeFetchCache, WKWebsiteDataTypeMemoryCache};
    use std::ptr::NonNull;
    use std::sync::{mpsc, OnceLock};
    use tauri::webview::PlatformWebview;
    use tauri::AppHandle;

    fn view(pw: &PlatformWebview) -> &WKWebView {
        unsafe { &*(pw.inner() as *const WKWebView) }
    }

    pub fn state(pw: &PlatformWebview) -> Option<State> {
        let web = view(pw);
        unsafe {
            Some(State {
                url: web.URL().and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default(),
                title: web.title().map(|s| s.to_string()).unwrap_or_default(),
                back: web.canGoBack(),
                forward: web.canGoForward(),
                loading: web.isLoading(),
                devtools: false,
                audible: web.respondsToSelector(sel!(_isPlayingAudio)) && msg_send![web, _isPlayingAudio],
            })
        }
    }

    pub fn act(pw: &PlatformWebview, action: Action) {
        let web = view(pw);
        unsafe {
            match action {
                Action::Back => drop(web.goBack()),
                Action::Forward => drop(web.goForward()),
                Action::HardReload => drop(web.reloadFromOrigin()),
                Action::Stop => web.stopLoading(),
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
                return PAGES.lock().unwrap().contains(&ptr);
            }
            at = unsafe { v.superview() };
        }
        false
    }

    /// The page webviews, by address, so a key chord in one can be told from one in the panel.
    static PAGES: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

    pub fn remember(pw: &PlatformWebview) {
        PAGES.lock().unwrap().push(pw.inner() as usize);
    }

    pub fn forget(pw: &PlatformWebview) {
        PAGES.lock().unwrap().retain(|&p| p != pw.inner() as usize);
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use super::{Action, State};
    use tauri::webview::PlatformWebview;
    use tauri::AppHandle;

    pub fn state(_: &PlatformWebview) -> Option<State> {
        None
    }
    pub fn act(_: &PlatformWebview, _: Action) {}
    pub fn remember(_: &PlatformWebview) {}
    pub fn forget(_: &PlatformWebview) {}
    pub fn snapshot(_: &PlatformWebview, tx: std::sync::mpsc::Sender<Result<Vec<u8>, String>>) {
        let _ = tx.send(Err("screenshots need macOS".into()));
    }
    pub fn watch_keys(_: &AppHandle) {}
    pub fn detach_inspector(_: &PlatformWebview) {}
    pub fn raise_inspectors() {}
}
