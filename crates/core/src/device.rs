//! The Device tab's work on a machine, as T3 Code's device host does it: the simulators and
//! emulators Expo's device hub finds, opening and powering them off, screenshots, the accessibility
//! tree, input for a page that can't reach the hub's socket, and the settings T3 changes with
//! `xcrun simctl` and `adb` rather than through the hub.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// What a session started while agents may drive devices is told, as T3 hands its agents the same
/// agent-device CLI.
pub const AGENT_PROMPT: &str = "Device control is on for this machine: you can see and drive its iOS simulators and Android emulators with the agent-device CLI, `npx -y agent-device@0.21.25`. Run `npx -y agent-device@0.21.25 --help` before your first use to learn its commands.";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Ios,
    Android,
}

impl Platform {
    fn vendor(self) -> &'static str {
        match self {
            Platform::Ios => "/vendor/serve-sim",
            Platform::Android => "/vendor/serve-emu",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub version: String,
    pub platform: Platform,
    pub booted: bool,
}

/// What a platform needs before its devices can open, as T3's setup checks it.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    Ready,
    /// The tools are installed, but there is no simulator or virtual device.
    NoDevice,
    Missing,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Devices {
    pub devices: Vec<Device>,
    /// What the hub couldn't read, for T3's discovery warning.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub ios: Support,
    pub android: Support,
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DeviceOp {
    /// `Devices`.
    List,
    /// The opened device's id, an Android emulator's serial once it runs. Boots it when it is off,
    /// and attaches the hub's stream to an iOS simulator.
    Open { platform: Platform, id: String, name: String, booted: bool },
    /// `null`. Powers the device off.
    Shutdown { platform: Platform, id: String },
    /// A `data:image/png` URL of the screen.
    Screenshot { platform: Platform, id: String },
    /// `null`. An Android gesture, key or text in the hub's socket shape, for a page that can't open
    /// the socket itself.
    Input { platform: Platform, id: String, input: Value },
    /// The hub's accessibility tree, as the platform's helper answers it.
    Tree { platform: Platform, id: String },
    /// `Detail`: the settings T3's tools panel shows, and the app in front.
    Detail { platform: Platform, id: String },
    /// `null`. Changes a setting or acts on an app.
    Act { platform: Platform, id: String, action: Action },
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    SetAppearance { value: Appearance },
    SetTextSize { value: TextSize },
    SetToggle { setting: Toggle, value: bool },
    SetOrientation { value: Orientation },
    #[serde(rename_all = "camelCase")]
    SetLocation { latitude: f64, longitude: f64 },
    ClearLocation,
    #[serde(rename_all = "camelCase")]
    SetPermission { permission: String, decision: Decision, app_id: String },
    OpenUrl { url: String },
    #[serde(rename_all = "camelCase")]
    LaunchApp { app_id: String },
    #[serde(rename_all = "camelCase")]
    TerminateApp { app_id: String },
    #[serde(rename_all = "camelCase")]
    SendPush { app_id: String, text: String },
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Light,
    Dark,
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
pub enum TextSize {
    Small,
    Default,
    Large,
    ExtraLarge,
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub enum Toggle {
    ReduceMotion,
    IncreaseContrast,
    NetworkEnabled,
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Portrait,
    LandscapeLeft,
    PortraitUpsideDown,
    LandscapeRight,
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Grant,
    Revoke,
}

#[derive(Serialize, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appearance: Option<Appearance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_size: Option<TextSize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reduce_motion: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub increase_contrast: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
}

/// The machine's tools and the hub at `port`, which `run` reaches with the machine's PATH.
pub struct Host<'a> {
    pub port: u16,
    pub path: Option<&'a str>,
}

impl Host<'_> {
    pub fn run(&self, op: DeviceOp) -> Result<Value, String> {
        let done = Ok(Value::Null);
        match op {
            DeviceOp::List => Ok(json!(self.list()?)),
            DeviceOp::Open { platform, id, name, booted } => self.open(platform, &id, &name, booted).map(Value::from),
            DeviceOp::Shutdown { platform, id } => self.shutdown(platform, &id).and(done),
            DeviceOp::Screenshot { platform, id } => {
                let png = self.hub("POST", &format!("{}/api/screenshot?device={}", platform.vendor(), encode(&id)), None, Duration::from_secs(20))?;
                use base64::Engine;
                Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)).into())
            }
            DeviceOp::Input { platform: Platform::Android, id, input } => self.input(&id, input).and(done),
            DeviceOp::Input { platform: Platform::Ios, .. } => Err("an iOS simulator takes input only from its stream".into()),
            DeviceOp::Tree { platform, id } => {
                let path = match platform {
                    Platform::Ios => format!("/vendor/serve-sim/helper/{}/ax", encode(&id)),
                    Platform::Android => format!("/vendor/serve-emu/api/accessibility?device={}", encode(&id)),
                };
                serde_json::from_slice(&self.hub("GET", &path, None, Duration::from_secs(15))?).map_err(|e| format!("the hub sent a bad tree: {e}"))
            }
            DeviceOp::Detail { platform: Platform::Android, id } => Ok(json!(self.android_detail(&id))),
            DeviceOp::Detail { platform: Platform::Ios, id } => Ok(json!(self.ios_detail(&id))),
            DeviceOp::Act { platform: Platform::Android, id, action } => self.android_act(&id, action).and(done),
            DeviceOp::Act { platform: Platform::Ios, id, action } => self.ios_act(&id, action).and(done),
        }
    }

    fn list(&self) -> Result<Devices, String> {
        let body: Value = serde_json::from_slice(&self.hub("GET", "/api/devices", None, Duration::from_secs(15))?).map_err(|e| format!("the hub sent a bad device list: {e}"))?;
        let mut devices: Vec<Device> = ["simulators", "emulators"]
            .iter()
            .flat_map(|k| body[k].as_array().cloned().unwrap_or_default())
            .filter_map(|d| serde_json::from_value(d).ok())
            .collect();
        let mut detail: Vec<String> = body["errors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| !stale_simulator(e))
            .filter_map(|e| e["message"].as_str().map(str::to_owned))
            .collect();
        // The hub lists an AVD only once it runs, so the ones that are off come from the emulator.
        let avds = self.tool("emulator", &["-list-avds"]);
        match &avds {
            Some(Ok(out)) => {
                for name in out.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("INFO")) {
                    if !devices.iter().any(|d| d.platform == Platform::Android && d.name == name) {
                        devices.push(Device { id: name.into(), name: name.into(), version: "Android".into(), platform: Platform::Android, booted: false });
                    }
                }
            }
            Some(Err(code)) => detail.push(format!("Could not list Android virtual devices (emulator -list-avds, exit code {code}). Discovered devices remain available.")),
            None => {}
        }
        let has = |p: Platform| devices.iter().any(|d| d.platform == p);
        let ios = if has(Platform::Ios) {
            Support::Ready
        } else if cfg!(target_os = "macos") && self.tool("xcrun", &["simctl", "help"]).is_some_and(|r| r.is_ok()) {
            Support::NoDevice
        } else {
            Support::Missing
        };
        let android = if has(Platform::Android) {
            Support::Ready
        } else if avds.is_some() || self.tool("adb", &["version"]).is_some() {
            Support::NoDevice
        } else {
            Support::Missing
        };
        Ok(Devices { devices, detail: (!detail.is_empty()).then(|| detail.join("\n")), ios, android })
    }

    fn open(&self, platform: Platform, id: &str, name: &str, booted: bool) -> Result<String, String> {
        let id = if booted {
            id.to_owned()
        } else {
            let answer = self.hub_json("POST", "/api/devices/boot", Some(json!({ "platform": platform, "id": id, "name": name })), Duration::from_secs(180))?;
            if answer["ok"] != true {
                let said: Vec<&str> = std::iter::once(&answer["error"])
                    .chain(answer["errors"].as_array().into_iter().flatten().flat_map(|e| [&e["message"], &e["error"]]))
                    .filter_map(|v| v.as_str())
                    .filter(|t| !t.trim().is_empty())
                    .collect();
                return Err(if said.is_empty() { format!("{name} didn't start") } else { said.join("\n\n") });
            }
            answer["serial"].as_str().or(answer["id"].as_str()).unwrap_or(id).to_owned()
        };
        // Booting alone doesn't attach serve-sim's capture; its grid start does both.
        if platform == Platform::Ios {
            let answer = self.hub_json("POST", "/vendor/serve-sim/grid/api/start", Some(json!({ "udid": id })), Duration::from_secs(180))?;
            if answer["ok"] != true {
                return Err(answer["error"].as_str().unwrap_or("the simulator's stream didn't start").to_owned());
            }
        }
        Ok(id)
    }

    fn shutdown(&self, platform: Platform, id: &str) -> Result<(), String> {
        let (path, body) = match platform {
            Platform::Ios => ("/vendor/serve-sim/grid/api/shutdown", json!({ "udid": id })),
            Platform::Android => ("/api/devices/shutdown", json!({ "platform": platform, "id": id })),
        };
        let answer = self.hub_json("POST", path, Some(body), Duration::from_secs(30))?;
        if answer["ok"] == true {
            Ok(())
        } else {
            Err(answer["error"].as_str().unwrap_or("the device didn't power off").to_owned())
        }
    }

    /// serve-emu's REST routes take the same gestures its socket does.
    fn input(&self, id: &str, input: Value) -> Result<(), String> {
        let kind = input["type"].as_str().unwrap_or_default().to_owned();
        let (route, body) = match kind.as_str() {
            "tap" | "swipe" | "text" | "key" => (kind.clone(), input),
            "back" | "home" | "recents" | "power" => ("key".to_owned(), json!({ "key": kind })),
            _ => return Err(format!("{kind:?} is no input the hub takes")),
        };
        self.hub("POST", &format!("/vendor/serve-emu/api/{route}?device={}", encode(id)), Some(body), Duration::from_secs(10)).map(drop)
    }

    fn adb(&self, serial: &str, args: &[&str]) -> Result<String, String> {
        let mut all = vec!["-s", serial];
        all.extend(args);
        match self.tool("adb", &all) {
            Some(Ok(out)) => Ok(out),
            Some(Err(code)) => Err(format!("adb {} failed (exit code {code})", args.join(" "))),
            None => Err("adb isn't on this machine's PATH".into()),
        }
    }

    fn shell(&self, serial: &str, args: &[&str]) -> Result<String, String> {
        let mut all = vec!["shell"];
        all.extend(args);
        self.adb(serial, &all)
    }

    fn android_act(&self, serial: &str, action: Action) -> Result<(), String> {
        match action {
            Action::SetAppearance { value } => self.shell(serial, &["cmd", "uimode", "night", if value == Appearance::Dark { "yes" } else { "no" }]).map(drop),
            Action::SetTextSize { value } => {
                let scale = match value {
                    TextSize::Small => "0.85",
                    TextSize::Default => "1.0",
                    TextSize::Large => "1.15",
                    TextSize::ExtraLarge => "1.3",
                };
                self.shell(serial, &["settings", "put", "system", "font_scale", scale]).map(drop)
            }
            Action::SetToggle { setting: Toggle::NetworkEnabled, value } => {
                let state = if value { "enable" } else { "disable" };
                self.shell(serial, &["svc", "wifi", state])?;
                self.shell(serial, &["svc", "data", state]).map(drop)
            }
            Action::SetToggle { setting: Toggle::ReduceMotion, value } => {
                for key in ["animator_duration_scale", "transition_animation_scale", "window_animation_scale"] {
                    self.shell(serial, &["settings", "put", "global", key, if value { "0" } else { "1" }])?;
                }
                Ok(())
            }
            Action::SetOrientation { value } => {
                let size = self.shell(serial, &["wm", "size"]).unwrap_or_default();
                let landscape_native = size.lines().filter_map(|l| l.split("size: ").nth(1)).last().and_then(|s| s.trim().split_once('x')).is_some_and(|(w, h)| w.parse::<u32>().ok() > h.parse::<u32>().ok());
                // A landscape-native tablet sits a quarter turn further along.
                let value = if !landscape_native {
                    value
                } else {
                    match value {
                        Orientation::Portrait => Orientation::LandscapeRight,
                        Orientation::LandscapeLeft => Orientation::Portrait,
                        Orientation::PortraitUpsideDown => Orientation::LandscapeLeft,
                        Orientation::LandscapeRight => Orientation::PortraitUpsideDown,
                    }
                };
                let (gravity, rotation) = match value {
                    Orientation::Portrait => ("0:9.81:0", "0"),
                    Orientation::LandscapeLeft => ("9.81:0:0", "1"),
                    Orientation::PortraitUpsideDown => ("0:-9.81:0", "2"),
                    Orientation::LandscapeRight => ("-9.81:0:0", "3"),
                };
                // Locking the rotation only turns the windows on recent images; tilting the emulator's
                // accelerometer turns the display the hub captures.
                if serial.starts_with("emulator-") {
                    self.shell(serial, &["settings", "put", "system", "accelerometer_rotation", "1"])?;
                    self.shell(serial, &["cmd", "window", "user-rotation", "free"])?;
                    return self.adb(serial, &["emu", "sensor", "set", "acceleration", gravity]).map(drop);
                }
                self.shell(serial, &["cmd", "window", "user-rotation", "lock", rotation]).map(drop)
            }
            Action::SetLocation { latitude, longitude } => self.adb(serial, &["emu", "geo", "fix", &longitude.to_string(), &latitude.to_string()]).map(drop),
            // The emulator has no way to clear a fix.
            Action::ClearLocation => Ok(()),
            Action::SetPermission { permission, decision, app_id } => {
                let names: &[&str] = match permission.as_str() {
                    "camera" => &["android.permission.CAMERA"],
                    "microphone" => &["android.permission.RECORD_AUDIO"],
                    "photos" => &["android.permission.READ_MEDIA_IMAGES", "android.permission.READ_EXTERNAL_STORAGE"],
                    "contacts" => &["android.permission.READ_CONTACTS", "android.permission.WRITE_CONTACTS"],
                    "calendar" => &["android.permission.READ_CALENDAR", "android.permission.WRITE_CALENDAR"],
                    "location" => &["android.permission.ACCESS_FINE_LOCATION", "android.permission.ACCESS_COARSE_LOCATION"],
                    "notifications" => &["android.permission.POST_NOTIFICATIONS"],
                    "motion" => &["android.permission.ACTIVITY_RECOGNITION"],
                    _ => return Err(format!("Android has no {permission} permission to change")),
                };
                let verb = if decision == Decision::Grant { "grant" } else { "revoke" };
                for name in names {
                    let _ = self.shell(serial, &["pm", verb, &app_id, name]);
                }
                Ok(())
            }
            Action::OpenUrl { url } => self.shell(serial, &["am", "start", "-a", "android.intent.action.VIEW", "-d", &url]).map(drop),
            Action::LaunchApp { app_id } => self.shell(serial, &["monkey", "-p", &app_id, "-c", "android.intent.category.LAUNCHER", "1"]).map(drop),
            Action::TerminateApp { app_id } => self.shell(serial, &["am", "force-stop", &app_id]).map(drop),
            Action::SetToggle { setting: Toggle::IncreaseContrast, .. } | Action::SendPush { .. } => Err("Android emulators can't do that".into()),
        }
    }

    fn android_detail(&self, serial: &str) -> Detail {
        let read = |args: &[&str]| self.shell(serial, args).ok().map(|s| s.trim().to_owned()).filter(|s| s != "null");
        let night = read(&["cmd", "uimode", "night"]);
        let scale = read(&["settings", "get", "system", "font_scale"]).and_then(|s| s.parse::<f64>().ok());
        let animator = read(&["settings", "get", "global", "animator_duration_scale"]).and_then(|s| s.parse::<f64>().ok());
        let wifi = read(&["settings", "get", "global", "wifi_on"]);
        // `dumpsys window windows` stopped printing the focus on API 36; the whole dump still does.
        let focus = read(&["dumpsys", "window"]).unwrap_or_default();
        let foreground = focus.lines().find_map(|l| {
            let at = l.find("mCurrentFocus=").or_else(|| l.find("mFocusedApp="))?;
            let rest = &l[at..];
            let word = rest.split_whitespace().find(|w| w.contains('/'))?;
            Some(word.split('/').next()?.to_owned())
        });
        Detail {
            appearance: night.as_deref().and_then(|n| if n.contains("yes") { Some(Appearance::Dark) } else if n.contains("no") { Some(Appearance::Light) } else { None }),
            text_size: scale.map(|s| if s <= 0.9 { TextSize::Small } else if s >= 1.25 { TextSize::ExtraLarge } else if s >= 1.1 { TextSize::Large } else { TextSize::Default }),
            reduce_motion: animator.map(|a| a == 0.0),
            increase_contrast: None,
            network_enabled: wifi.as_deref().and_then(|w| match w {
                "1" => Some(true),
                "0" => Some(false),
                _ => None,
            }),
            foreground,
        }
    }

    fn simctl(&self, udid: &str, args: &[&str]) -> Result<String, String> {
        let mut all = vec!["simctl", args[0], udid];
        all.extend(&args[1..]);
        match self.tool("xcrun", &all) {
            Some(Ok(out)) => Ok(out),
            Some(Err(code)) => Err(format!("simctl {} failed (exit code {code})", args.join(" "))),
            None => Err("xcrun isn't on this machine's PATH".into()),
        }
    }

    fn ios_act(&self, udid: &str, action: Action) -> Result<(), String> {
        let size = |s: TextSize| match s {
            TextSize::Small => "small",
            TextSize::Default => "large",
            TextSize::Large => "extra-extra-large",
            TextSize::ExtraLarge => "accessibility-large",
        };
        match action {
            Action::SetAppearance { value } => self.simctl(udid, &["ui", "appearance", if value == Appearance::Dark { "dark" } else { "light" }]).map(drop),
            Action::SetTextSize { value } => self.simctl(udid, &["ui", "content_size", size(value)]).map(drop),
            Action::SetToggle { setting: Toggle::IncreaseContrast, value } => self.simctl(udid, &["ui", "increase_contrast", if value { "enabled" } else { "disabled" }]).map(drop),
            Action::SetLocation { latitude, longitude } => self.simctl(udid, &["location", "set", &format!("{latitude},{longitude}")]).map(drop),
            Action::ClearLocation => self.simctl(udid, &["location", "clear"]).map(drop),
            Action::SetPermission { permission, decision, app_id } => {
                let service = match permission.as_str() {
                    "camera" | "microphone" | "photos" | "contacts" | "calendar" | "reminders" | "motion" | "media-library" | "faceid" | "location" => permission.as_str(),
                    _ => return Err(format!("simctl can't change the {permission} permission")),
                };
                self.simctl(udid, &["privacy", if decision == Decision::Grant { "grant" } else { "revoke" }, service, &app_id]).map(drop)
            }
            Action::OpenUrl { url } => self.simctl(udid, &["openurl", &url]).map(drop),
            Action::LaunchApp { app_id } => self.simctl(udid, &["launch", &app_id]).map(drop),
            Action::TerminateApp { app_id } => self.simctl(udid, &["terminate", &app_id]).map(drop),
            Action::SendPush { app_id, text } => {
                let payload = json!({ "aps": { "alert": text } }).to_string();
                let mut command = crate::process::command("xcrun");
                command.args(["simctl", "push", udid, &app_id, "-"]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
                if let Some(path) = self.path {
                    command.env("PATH", path);
                }
                let mut child = command.spawn().map_err(|_| "xcrun isn't on this machine's PATH".to_owned())?;
                let _ = child.stdin.take().map(|mut i| i.write_all(payload.as_bytes()));
                match child.wait() {
                    Ok(s) if s.success() => Ok(()),
                    _ => Err("simctl couldn't send the notification".into()),
                }
            }
            Action::SetToggle { setting: Toggle::ReduceMotion, value } => self.ax_settings(udid, "reduce-motion", value),
            Action::SetToggle { setting: Toggle::NetworkEnabled, .. } | Action::SetOrientation { .. } => Err("iOS simulators can't do that from here".into()),
        }
    }

    /// serve-sim's accessibility helper, which the hub's own install carries.
    fn ax_settings(&self, udid: &str, option: &str, on: bool) -> Result<(), String> {
        let helper = ax_helper().ok_or("the device hub's accessibility helper isn't installed")?;
        let helper = helper.to_string_lossy();
        match self.tool("xcrun", &["simctl", "spawn", udid, &helper, "set", option, if on { "on" } else { "off" }]) {
            Some(Ok(_)) => Ok(()),
            _ => Err(format!("the simulator didn't take {option}")),
        }
    }

    fn ios_detail(&self, udid: &str) -> Detail {
        let ui = |option: &str| self.simctl(udid, &["ui", option]).ok().map(|s| s.trim().to_ascii_lowercase());
        let size = ui("content_size");
        Detail {
            appearance: ui("appearance").and_then(|a| match a.as_str() {
                "dark" => Some(Appearance::Dark),
                "light" => Some(Appearance::Light),
                _ => None,
            }),
            text_size: size.map(|c| match c.as_str() {
                "small" | "extra-small" | "medium" => TextSize::Small,
                "large" => TextSize::Default,
                c if c.starts_with("accessibility") => TextSize::ExtraLarge,
                c if c.contains("extra") => TextSize::Large,
                _ => TextSize::Default,
            }),
            increase_contrast: ui("increase_contrast").map(|c| c == "enabled"),
            ..Detail::default()
        }
    }

    /// `None` when `program` isn't installed, else its output or exit code.
    fn tool(&self, program: &str, args: &[&str]) -> Option<Result<String, i32>> {
        let mut command = crate::process::command(program);
        command.args(args).stdin(Stdio::null()).stderr(Stdio::null());
        if let Some(path) = self.path {
            command.env("PATH", path);
        }
        let out = command.output().ok()?;
        Some(if out.status.success() { Ok(String::from_utf8_lossy(&out.stdout).into_owned()) } else { Err(out.status.code().unwrap_or(-1)) })
    }

    fn hub_json(&self, method: &str, path: &str, body: Option<Value>, wait: Duration) -> Result<Value, String> {
        serde_json::from_slice(&self.hub(method, path, body, wait)?).map_err(|e| format!("the device hub sent a bad answer: {e}"))
    }

    /// The hub's answer to one request. Without an Origin header the hub takes it as a local tool's,
    /// which its mutation guard lets through.
    fn hub(&self, method: &str, path: &str, body: Option<Value>, wait: Duration) -> Result<Vec<u8>, String> {
        let mut stream = std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], self.port).into(), Duration::from_secs(2)).map_err(|_| "the device hub isn't running".to_owned())?;
        stream.set_read_timeout(Some(wait)).map_err(|e| e.to_string())?;
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        let kind = if body.is_empty() { "" } else { "Content-Type: application/json\r\n" };
        write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n{kind}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", self.port, body.len()).map_err(|e| e.to_string())?;
        let mut answer = vec![];
        stream.read_to_end(&mut answer).map_err(|e| format!("the device hub didn't answer: {e}"))?;
        let split = answer.windows(4).position(|w| w == b"\r\n\r\n").ok_or("the device hub sent no headers")?;
        let head = String::from_utf8_lossy(&answer[..split]).to_ascii_lowercase();
        let mut payload = answer[split + 4..].to_vec();
        if head.contains("transfer-encoding: chunked") {
            payload = unchunk(&payload);
        }
        let status: u16 = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        if !(200..300).contains(&status) {
            let said: Value = serde_json::from_slice(&payload).unwrap_or(Value::Null);
            let message = said["error"]["message"].as_str().or(said["error"].as_str()).map(str::to_owned).unwrap_or_else(|| String::from_utf8_lossy(&payload).trim().to_owned());
            return Err(if message.is_empty() { format!("the device hub answered {status}") } else { message });
        }
        Ok(payload)
    }
}

fn unchunk(body: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    let mut rest = body;
    while let Some(end) = rest.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(String::from_utf8_lossy(&rest[..end]).trim(), 16).unwrap_or(0);
        if size == 0 || rest.len() < end + 2 + size {
            break;
        }
        out.extend_from_slice(&rest[end + 2..end + 2 + size]);
        rest = &rest[(end + 4 + size).min(rest.len())..];
    }
    out
}

fn encode(part: &str) -> String {
    part.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// CoreSimulator keeps deleted devices, whose plists the hub then fails to read.
fn stale_simulator(error: &Value) -> bool {
    let message = error["message"].as_str().unwrap_or_default();
    message.starts_with("[apple-utils] Failed to read CoreSimulator plist at ") && message.ends_with("/device.plist:") && error["error"].as_str().is_some_and(|e| e.contains("ENOENT"))
}

fn ax_helper() -> Option<PathBuf> {
    let cache = Path::new(&std::env::var_os("HOME")?).join(".npm/_npx");
    std::fs::read_dir(cache).ok()?.flatten().map(|e| e.path().join("node_modules/expo-device-hub/vendor/serve-sim/dist/simax/serve-sim-ax-settings")).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A hub that answers each request with the next canned reply and keeps what it was asked.
    fn fake_hub(replies: Vec<(&'static str, String)>) -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let asked = Arc::new(Mutex::new(vec![]));
        let kept = asked.clone();
        std::thread::spawn(move || {
            for ((status, body), stream) in replies.into_iter().zip(listener.incoming()) {
                let mut stream = stream.unwrap();
                let mut request = vec![];
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&request).into_owned();
                let length: usize = head.lines().find_map(|l| l.strip_prefix("Content-Length: ")).and_then(|n| n.trim().parse().ok()).unwrap_or(0);
                let mut sent = vec![0; length];
                stream.read_exact(&mut sent).unwrap();
                kept.lock().unwrap().push(format!("{} {}", head.lines().next().unwrap(), String::from_utf8_lossy(&sent)));
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len()).unwrap();
            }
        });
        (port, asked)
    }

    #[cfg(unix)]
    fn tools(dir: &Path, scripts: &[(&str, &str)]) -> String {
        use std::os::unix::fs::PermissionsExt;
        for (name, script) in scripts {
            let file = dir.join(name);
            std::fs::write(&file, format!("#!/bin/sh\necho \"{name} $*\" >> {}/calls\n{script}\n", dir.display())).unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        format!("{}:/usr/bin:/bin", dir.display())
    }

    #[test]
    #[cfg(unix)]
    fn the_list_joins_the_hubs_devices_with_avds_that_are_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = tools(dir.path(), &[("emulator", "printf 'Pixel_9\\nTablet\\n'")]);
        let list = r#"{"simulators":[],"emulators":[{"id":"emulator-5554","name":"Pixel_9","version":"Android 16","platform":"android","booted":true,"physical":false}],
            "errors":[{"message":"[apple-utils] Failed to read CoreSimulator plist at /x/CoreSimulator/Devices/A/device.plist:","error":"Error: ENOENT: gone"},{"message":"adb is slow"}]}"#;
        let (port, _) = fake_hub(vec![("200 OK", list.into())]);
        let got = Host { port, path: Some(&path) }.list().unwrap();
        assert_eq!(
            got,
            Devices {
                devices: vec![
                    Device { id: "emulator-5554".into(), name: "Pixel_9".into(), version: "Android 16".into(), platform: Platform::Android, booted: true },
                    Device { id: "Tablet".into(), name: "Tablet".into(), version: "Android".into(), platform: Platform::Android, booted: false },
                ],
                detail: Some("adb is slow".into()),
                ios: if cfg!(target_os = "macos") && Path::new("/usr/bin/xcrun").exists() { got.ios } else { Support::Missing },
                android: Support::Ready,
            }
        );
    }

    #[test]
    fn opening_boots_an_avd_and_a_refusal_says_why() {
        let (port, asked) = fake_hub(vec![("200 OK", r#"{"ok":true,"id":"Pixel_9","serial":"emulator-5556"}"#.into()), ("200 OK", r#"{"ok":false,"error":"boot failed","errors":[{"message":"no space left on device"}]}"#.into())]);
        let host = Host { port, path: None };
        assert_eq!(host.open(Platform::Android, "Pixel_9", "Pixel_9", false), Ok("emulator-5556".into()));
        assert_eq!(host.open(Platform::Android, "Pixel_9", "Pixel_9", false), Err("boot failed\n\nno space left on device".into()));
        assert_eq!(asked.lock().unwrap()[0], r#"POST /api/devices/boot HTTP/1.1 {"platform":"android","id":"Pixel_9","name":"Pixel_9"}"#);
    }

    #[test]
    fn input_goes_to_serve_emus_routes() {
        let (port, asked) = fake_hub(vec![("200 OK", r#"{"ok":true}"#.into()), ("200 OK", r#"{"ok":true}"#.into()), ("403 Forbidden", r#"{"ok":false,"error":{"code":"forbidden","message":"Browser origin is not allowed"}}"#.into())]);
        let host = Host { port, path: None };
        host.input("emulator-5554", json!({ "type": "tap", "x": 0.5, "y": 0.25 })).unwrap();
        host.input("emulator-5554", json!({ "type": "home" })).unwrap();
        assert_eq!(host.input("emulator-5554", json!({ "type": "text", "text": "hi" })), Err("Browser origin is not allowed".into()));
        assert_eq!(host.input("emulator-5554", json!({ "type": "explode" })), Err("\"explode\" is no input the hub takes".into()));
        assert_eq!(
            *asked.lock().unwrap(),
            vec![
                r#"POST /vendor/serve-emu/api/tap?device=emulator-5554 HTTP/1.1 {"type":"tap","x":0.5,"y":0.25}"#.to_owned(),
                r#"POST /vendor/serve-emu/api/key?device=emulator-5554 HTTP/1.1 {"key":"home"}"#.to_owned(),
                r#"POST /vendor/serve-emu/api/text?device=emulator-5554 HTTP/1.1 {"type":"text","text":"hi"}"#.to_owned(),
            ]
        );
    }

    #[test]
    #[cfg(unix)]
    fn android_settings_run_t3s_adb_commands_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = tools(
            dir.path(),
            &[("adb", r#"case "$*" in *"uimode night") echo "Night mode: yes";; *font_scale) echo 1.15;; *animator_duration_scale) echo 0;; *wifi_on) echo 1;; *"dumpsys window") echo "  mCurrentFocus=Window{1a2b u0 com.example.app/com.example.app.MainActivity}";; *"wm size") echo "Physical size: 1080x2400";; esac"#)],
        );
        let host = Host { port: 1, path: Some(&path) };
        let act = |action| host.android_act("emulator-5554", action).unwrap();
        act(Action::SetAppearance { value: Appearance::Dark });
        act(Action::SetTextSize { value: TextSize::ExtraLarge });
        act(Action::SetToggle { setting: Toggle::NetworkEnabled, value: false });
        act(Action::SetOrientation { value: Orientation::LandscapeLeft });
        act(Action::LaunchApp { app_id: "com.example.app".into() });
        let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
        assert_eq!(
            calls,
            "adb -s emulator-5554 shell cmd uimode night yes\n\
             adb -s emulator-5554 shell settings put system font_scale 1.3\n\
             adb -s emulator-5554 shell svc wifi disable\n\
             adb -s emulator-5554 shell svc data disable\n\
             adb -s emulator-5554 shell wm size\n\
             adb -s emulator-5554 shell settings put system accelerometer_rotation 1\n\
             adb -s emulator-5554 shell cmd window user-rotation free\n\
             adb -s emulator-5554 emu sensor set acceleration 9.81:0:0\n\
             adb -s emulator-5554 shell monkey -p com.example.app -c android.intent.category.LAUNCHER 1\n"
        );
        assert_eq!(
            host.android_detail("emulator-5554"),
            Detail { appearance: Some(Appearance::Dark), text_size: Some(TextSize::Large), reduce_motion: Some(true), increase_contrast: None, network_enabled: Some(true), foreground: Some("com.example.app".into()) }
        );
    }

    #[test]
    fn device_ops_read_as_the_page_sends_them() {
        let op: DeviceOp = serde_json::from_value(json!({ "op": "act", "platform": "android", "id": "emulator-5554", "action": { "type": "setPermission", "permission": "camera", "decision": "grant", "appId": "com.x" } })).unwrap();
        assert_eq!(op, DeviceOp::Act { platform: Platform::Android, id: "emulator-5554".into(), action: Action::SetPermission { permission: "camera".into(), decision: Decision::Grant, app_id: "com.x".into() } });
        let op: DeviceOp = serde_json::from_value(json!({ "op": "act", "platform": "ios", "id": "U", "action": { "type": "setTextSize", "value": "extra-large" } })).unwrap();
        assert_eq!(op, DeviceOp::Act { platform: Platform::Ios, id: "U".into(), action: Action::SetTextSize { value: TextSize::ExtraLarge } });
    }
}
