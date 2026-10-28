Object.assign(ICON_PATHS, {
  house: `<path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8"/><path d="M3 10a2 2 0 0 1 .709-1.528l7-5.999a2 2 0 0 1 2.582 0l7 5.999A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>`,
  square: `<rect width="18" height="18" x="3" y="3" rx="2"/>`,
  type: `<polyline points="4 7 4 4 20 4 20 7"/><line x1="9" x2="15" y1="20" y2="20"/><line x1="12" x2="12" y1="4" y2="20"/>`,
  power: `<path d="M12 2v10"/><path d="M18.4 6.6a9 9 0 1 1-12.77.04"/>`,
});

/** The Device tab, as T3's DevicePanel draws it over Expo's device hub on the thread's machine: a
 * setup wizard, the machine's iOS simulators and Android emulators to open, then the device's live
 * screen with T3's controls rail and tools. The machine answers the hub's REST routes; the screen
 * streams straight from the hub when the hub is on this device, and from screenshots when it isn't. */
const deviceHosts = new Map();
const deviceHost = (machine) => deviceHosts.get(machine) ?? deviceHosts.set(machine, { list: null, listing: null, error: null, hub: null }).get(machine);
const deviceCall = (tab, op) => callOn(tab.machine, { cmd: "device", ...op });
const deviceSetupDone = (machine) => load(`gui-device-setup:${machine}`) === "done";
const platformLabel = (platform) => (platform === "ios" ? "iOS Simulators" : "Android Emulators");
const olderPinned = (tab, e) => (/unknown variant/.test(String(e)) ? `${machineName(tab.machine)} runs an older pinned, which can't open devices. Update it to use this tab.` : String(e));

async function listDevices(tab) {
  const host = deviceHost(tab.machine);
  host.listing ??= (async () => {
    try {
      host.hub ??= await callOn(tab.machine, { cmd: "device_hub" });
      host.list = await deviceCall(tab, { op: "list" });
      host.error = null;
    } catch (e) {
      host.error = olderPinned(tab, e);
    }
    host.listing = null;
    if (tab === active && shownSurface(tab)?.kind === "device") renderPanel();
    if (!deviceSetup.hidden) renderDeviceSetup();
  })();
  return host.listing;
}

/** T3's setup: turn the hub on, see which platforms are ready, and decide whether agents drive devices. */
const deviceSetup = document.createElement("div");
deviceSetup.id = "device-setup";
deviceSetup.hidden = true;
document.body.append(deviceSetup);
let setupFor = null;
const SETUP_STEPS = ["Device hub", "Simulators", "Agent access"];
const PLATFORM_COPY = {
  ios: { ready: "Xcode and iOS Simulator are available.", no_device: "Xcode is installed, but no iOS Simulator is available. Install a runtime in Xcode Settings → Components.", missing: "iOS support was not detected." },
  android: { ready: "The Android SDK and Emulator are available.", no_device: "The Android SDK is installed, but no virtual device exists. Create one in Android Studio → Device Manager.", missing: "Android support was not detected." },
};
function openDeviceSetup(tab) {
  setupFor = { tab, step: 0, enabled: load(`gui-device-hub:${tab.machine}`) === "on", agents: false };
  deviceSetup.hidden = false;
  renderDeviceSetup();
  callOn(tab.machine, { cmd: "device_agents" }).then((on) => {
    if (setupFor?.tab !== tab) return;
    setupFor.agents = on;
    renderDeviceSetup();
  }, () => {});
}
function renderDeviceSetup() {
  if (!setupFor) return;
  const { tab, step, enabled, agents } = setupFor;
  const host = deviceHost(tab.machine);
  const sw = (on, label, attr) => `<button type="button" class="switch${on ? " on" : ""}" role="switch" aria-checked="${on}" aria-label="${esc(label)}" ${attr}></button>`;
  const status = !enabled ? "" : host.error ? `<p class="setup-error" role="alert">${esc(host.error)}</p>`
    : host.list ? `<p class="setup-status">${icon("check", "ok")}Device hub is ready.</p>` : `<p class="setup-status"><span class="spinner small"></span>Starting device hub…</p>`;
  const platformRow = (platform) => {
    const support = host.list?.[platform] ?? "missing";
    return `<div class="setup-platform" data-platform="${platform}">${icon(support === "ready" ? "check" : "circle-alert", support === "ready" ? "ok" : "warn")}<div><p class="name">${platform === "ios" ? "iOS" : "Android"}</p><p>${esc(PLATFORM_COPY[platform][support])}</p></div></div>`;
  };
  const body = [
    `<h3>Enable the device hub</h3><p>Enable this environment to open simulators and emulators, whether they run here or on a remote device host.</p>
      <div class="setup-switch"><span>Device hub</span>${sw(enabled, "Enable device hub", "data-setup-hub")}</div>${status}`,
    `<h3>Check simulator support</h3>${platformRow("ios")}${platformRow("android")}<p class="muted">You can use either platform. Fixing a missing platform does not block the other one.</p>
      <button type="button" class="outline" data-setup-check${host.listing ? " disabled" : ""}>${host.listing ? "Checking…" : "Check again"}</button>`,
    `<h3>Allow agent control</h3><p>Allow new agent sessions in this environment to start and control local and remote devices, with required tools set up automatically.</p>
      <div class="setup-switch"><span>Agent control</span>${sw(agents, "Allow agents to control devices", "data-setup-agents")}</div>
      ${agents ? `<p class="setup-status">${icon("check", "ok")}Agent tools are ready.</p>` : ""}<p class="muted">Leave this off to keep manual device controls without giving agents access.</p>`,
  ][step];
  const canGo = step !== 0 || (enabled && !!host.list);
  deviceSetup.innerHTML = `<div class="setup-card" role="dialog" aria-modal="true" aria-labelledby="device-setup-title">
    <header><h2 id="device-setup-title">Set up devices</h2><p>Review what runs on this environment before using simulators and emulators.</p>
      <ol class="setup-steps">${SETUP_STEPS.map((name, i) => `<li class="${i === step ? "on" : i < step ? "done" : ""}"><span>${i + 1}</span>${esc(name)}</li>`).join("")}</ol></header>
    <div class="setup-body">${body}</div>
    <footer><button type="button" class="ghost" data-setup-back>${step === 0 ? "Cancel" : "Back"}</button><button type="button" class="primary" data-setup-next${canGo ? "" : " disabled"}>${step === 2 ? "Done" : "Continue"}</button></footer></div>`;
}
function closeDeviceSetup(done) {
  const tab = setupFor?.tab;
  deviceSetup.hidden = true;
  deviceSetup.innerHTML = "";
  setupFor = null;
  if (!tab) return;
  if (done) save(`gui-device-setup:${tab.machine}`, "done");
  // Leaving setup undone closes the tab, as T3's dismissed wizard does.
  else {
    const s = panelOf(tab).surfaces.find((x) => x.kind === "device" && !x.target);
    if (s && tab === active) return closeSurface(s.id);
  }
  if (tab === active) renderPanel();
}
deviceSetup.addEventListener("click", (e) => {
  if (!setupFor) return;
  const { tab } = setupFor;
  if (e.target === deviceSetup) return closeDeviceSetup(false);
  if (e.target.closest("[data-setup-hub]")) {
    setupFor.enabled = !setupFor.enabled;
    save(`gui-device-hub:${tab.machine}`, setupFor.enabled ? "on" : "off");
    if (setupFor.enabled) listDevices(tab);
  } else if (e.target.closest("[data-setup-agents]")) {
    // New sessions on the machine are told how to drive its devices; the machine keeps the choice.
    callOn(tab.machine, { cmd: "device_agents", allow: !setupFor.agents }).then((on) => {
      if (!setupFor) return;
      setupFor.agents = on;
      renderDeviceSetup();
    }, (err) => flash(olderPinned(tab, err)));
  } else if (e.target.closest("[data-setup-check]")) listDevices(tab);
  else if (e.target.closest("[data-setup-back]")) {
    if (setupFor.step === 0) return closeDeviceSetup(false);
    setupFor.step--;
  } else if (e.target.closest("[data-setup-next]:not([disabled])")) {
    if (setupFor.step === 2) return closeDeviceSetup(true);
    setupFor.step++;
  }
  renderDeviceSetup();
});
deviceSetup.addEventListener("keydown", (e) => e.key === "Escape" && closeDeviceSetup(false));

function deviceLoadingHtml(name, description, message, stage = 1) {
  return `<div class="device-loading"><div class="card"><span class="tile">${icon("smartphone")}</span><p class="name">${esc(name)}</p>${description ? `<p class="muted">${esc(description)}</p>` : ""}
    <p class="message"><span class="spinner small"></span>${esc(message)}</p>
    <div class="steps" role="img" aria-label="${stage === 1 ? "Step 1 of 2: open device" : "Step 2 of 2: connect video"}"><span class="on"></span><span${stage === 2 ? ` class="on"` : ""}></span></div></div></div>`;
}
const deviceNotice = (text, tone = "error") => `<div class="device-notice ${tone}" role="alert">${icon("circle-alert")}<div class="text">${esc(text)}</div><button type="button" class="icon" data-device-dismiss aria-label="${tone === "warning" ? "Dismiss device warning" : "Dismiss device error"}">${icon("x")}</button></div>`;

function deviceListHtml(tab, s) {
  const host = deviceHost(tab.machine);
  const list = host.list;
  const devices = list?.devices ?? [];
  const groups = ["ios", "android"].map((platform) => ({ platform, devices: devices.filter((d) => d.platform === platform).sort((a, b) => b.booted - a.booted || a.name.localeCompare(b.name)) })).filter((g) => g.devices.length);
  const where = machineName(tab.machine);
  const rows = groups.map((g) => `<section><h3>${icon("smartphone")}${platformLabel(g.platform)}</h3><div class="device-rows">${g.devices.map((d) => `<div class="device-row">
      <span class="tile">${icon("smartphone")}</span><span class="what"><span class="name">${esc(d.name)}</span><span class="muted">${esc(`${where} · ${d.version} · ${d.booted ? "Running" : "Stopped"}`)}</span></span>
      <button type="button" class="outline" data-device-open="${esc(d.platform)}\n${esc(d.id)}" aria-label="${d.booted ? "Open" : "Start"} ${esc(d.name)}"${dv(s).pending ? " disabled" : ""}>${dv(s).pending?.id === d.id ? `<span class="spinner small"></span>` : icon(d.booted ? "arrow-up-right" : "play")}${d.booted ? "Open" : "Start"}</button></div>`).join("")}</div></section>`).join("");
  const empty = !groups.length ? `<span class="big">${icon("smartphone")}</span><p>${esc(host.error ?? "No simulators or emulators were found on this environment.")}</p>` : "";
  const noAvd = list && list.android !== "missing" && !devices.some((d) => d.platform === "android") ? `<p class="note">No Android virtual devices found. Create one in Android Studio's Device Manager, then refresh.</p>` : "";
  return `<div class="device-list${groups.length ? "" : " empty"}"><div class="inner">${empty}${rows}${noAvd}
    <button type="button" class="outline refresh" data-device-refresh${host.listing ? " disabled" : ""}>Refresh devices</button></div></div>`;
}

function renderDevicePanel(s) {
  const tab = active;
  delete s.url;
  const host = deviceHost(tab.machine);
  if (!deviceSetupDone(tab.machine)) {
    if (!setupFor) openDeviceSetup(tab);
    if (load(`gui-device-hub:${tab.machine}`) === "on" && !host.list && !host.listing) listDevices(tab);
    return showPanel(`<div class="device-panel"></div>`);
  }
  const notices = `${host.list?.detail && !s.target && !host.dismissed ? deviceNotice(`Device discovery warning · ${machineName(tab.machine)}\n${host.list.detail}`, "warning") : ""}${dv(s).error ? deviceNotice(dv(s).error) : ""}`;
  if (s.target) return renderDeviceWorkspace(tab, s, notices);
  if (!host.list && !host.error) {
    if (!host.listing) listDevices(tab);
    return showPanel(`<div class="device-panel">${deviceLoadingHtml("Devices", "", "Finding devices…")}</div>`);
  }
  if (dv(s).pending) return showPanel(`<div class="device-panel">${notices}${deviceLoadingHtml(dv(s).pending.name, `${machineName(tab.machine)} · ${dv(s).pending.version}`, dv(s).pending.booted ? "Opening device…" : "Starting device…")}</div>`);
  showPanel(`<div class="device-panel">${notices}${deviceListHtml(tab, s)}</div>`);
}

async function openDevice(tab, s, device) {
  dv(s).pending = device;
  dv(s).error = null;
  renderPanel();
  try {
    const id = await deviceCall(tab, { op: "open", platform: device.platform, id: device.id, name: device.name, booted: device.booted });
    s.target = { platform: device.platform, id, name: device.name, version: device.version };
    dv(s).pending = null;
    savePanel(tab);
    listDevices(tab);
  } catch (e) {
    Object.assign(s, { pending: null, error: olderPinned(tab, e) });
  }
  if (tab === active) renderPanel();
}

/** What a Device tab shows for now and doesn't keep: the device opening, its error, its settings. */
const deviceViews = new Map();
const dv = (s) => deviceViews.get(s.id) ?? deviceViews.set(s.id, {}).get(s.id);

/** The device's screen: one per Device tab, kept across renders so its stream stays open. */
const deviceScreens = new Map();
const hubSocketUrl = (tab, path) => `${deviceHost(tab.machine).hub.replace(/^http/, "ws").replace(/\/$/, "")}${path}`;
function deviceScreen(tab, s) {
  const known = deviceScreens.get(s.id);
  if (known && known.target === `${s.target.platform}:${s.target.id}`) return known;
  if (known) closeDeviceScreen(s.id);
  const el = document.createElement("div");
  el.className = "device-phone";
  el.innerHTML = `<div class="device-screen" tabindex="0" role="application" aria-label="${esc(s.target.name)} screen"><canvas></canvas><img alt="" hidden><div class="ax-layer"></div></div>`;
  const entry = { el, screen: el.firstElementChild, tab, s, target: `${s.target.platform}:${s.target.id}`, size: null, mode: null, socket: null, hid: null, decoder: null, timer: null, tree: null, treeTimer: null };
  deviceScreens.set(s.id, entry);
  wireDeviceInput(entry);
  startDeviceStream(entry);
  return entry;
}
function closeDeviceScreen(id) {
  const entry = deviceScreens.get(id);
  if (!entry) return;
  deviceScreens.delete(id);
  clearInterval(entry.timer);
  clearInterval(entry.treeTimer);
  entry.socket?.close();
  entry.hid?.close();
  try {
    entry.decoder?.close();
  } catch {}
  entry.el.querySelector("img").src = "";
  entry.el.remove();
}
function sizeDeviceScreen(entry, width, height) {
  if (!width || !height || (entry.size?.[0] === width && entry.size?.[1] === height)) return;
  entry.size = [width, height];
  const canvas = entry.el.querySelector("canvas");
  Object.assign(canvas, { width, height });
  entry.el.style.setProperty("--ratio", `${width} / ${height}`);
}

/** Android streams H.264 over serve-emu's socket, which WebCodecs decodes; iOS streams MJPEG from
 * serve-sim. Both need the hub on this device; elsewhere, and without WebCodecs, screenshots stand in. */
function startDeviceStream(entry) {
  const { tab, s } = entry;
  const local = servesHere(tab) && !!deviceHost(tab.machine).hub;
  if (local && s.target.platform === "android" && typeof VideoDecoder !== "undefined") return streamAndroid(entry);
  if (local && s.target.platform === "ios") return streamIos(entry);
  pollDeviceScreen(entry);
}
function streamAndroid(entry) {
  const { tab, s } = entry;
  entry.mode = "video";
  const socket = new WebSocket(hubSocketUrl(tab, `/vendor/serve-emu/ws?device=${encodeURIComponent(s.target.id)}&frame-meta=1`));
  socket.binaryType = "arraybuffer";
  entry.socket = socket;
  const canvas = entry.el.querySelector("canvas");
  const draw = canvas.getContext("2d");
  let configured = false;
  socket.onmessage = (e) => {
    if (typeof e.data === "string") {
      const said = JSON.parse(e.data);
      if (said.type === "video-session") sizeDeviceScreen(entry, said.size?.width, said.size?.height);
      return;
    }
    const bytes = new Uint8Array(e.data);
    const view = new DataView(e.data);
    // serve-emu's v1 header: "SEMU", version, flags (bit 0 a keyframe), two spare bytes, pts in µs.
    if (bytes.length < 16 || view.getUint32(0) !== 0x53454d55) return;
    const key = (bytes[5] & 1) === 1;
    const data = bytes.subarray(16);
    if (!configured) {
      if (!key) return;
      const sps = annexBUnits(data).find((u) => (u[0] & 31) === 7);
      if (!sps) return;
      entry.decoder = new VideoDecoder({
        output: (frame) => {
          sizeDeviceScreen(entry, frame.displayWidth, frame.displayHeight);
          draw.drawImage(frame, 0, 0, canvas.width, canvas.height);
          frame.close();
          entry.screen.dataset.live = "video";
        },
        error: () => fallBackToScreenshots(entry),
      });
      entry.decoder.configure({ codec: `avc1.${[...sps.subarray(1, 4)].map((b) => b.toString(16).padStart(2, "0")).join("")}`, optimizeForLatency: true });
      configured = true;
    }
    entry.decoder.decode(new EncodedVideoChunk({ type: key ? "key" : "delta", timestamp: Number(view.getBigUint64(8)), data }));
  };
  socket.onerror = () => fallBackToScreenshots(entry);
  socket.onclose = () => deviceScreens.get(s.id) === entry && entry.mode === "video" && fallBackToScreenshots(entry);
}
function annexBUnits(bytes) {
  const units = [];
  let start = -1;
  for (let i = 0; i + 3 <= bytes.length; i++) {
    if (bytes[i] === 0 && bytes[i + 1] === 0 && (bytes[i + 2] === 1 || (bytes[i + 2] === 0 && bytes[i + 3] === 1))) {
      if (start >= 0) units.push(bytes.subarray(start, i));
      i += bytes[i + 2] === 1 ? 2 : 3;
      start = i + 1;
    }
  }
  if (start >= 0) units.push(bytes.subarray(start));
  return units;
}
function streamIos(entry) {
  const { tab, s } = entry;
  entry.mode = "mjpeg";
  const img = entry.el.querySelector("img");
  img.hidden = false;
  entry.el.querySelector("canvas").hidden = true;
  // A multipart image may never fire load, so its first frame is watched for.
  entry.timer = setInterval(() => {
    if (!img.naturalWidth) return;
    sizeDeviceScreen(entry, img.naturalWidth, img.naturalHeight);
    entry.screen.dataset.live = "mjpeg";
  }, 250);
  img.onerror = () => {
    clearInterval(entry.timer);
    fallBackToScreenshots(entry);
  };
  img.src = `${deviceHost(tab.machine).hub}vendor/serve-sim/helper/${encodeURIComponent(s.target.id)}/stream.mjpeg`;
  entry.hid = new WebSocket(hubSocketUrl(tab, `/vendor/serve-sim/helper/ws?device=${encodeURIComponent(s.target.id)}`));
  entry.hid.binaryType = "arraybuffer";
}
function fallBackToScreenshots(entry) {
  if (entry.mode === "screenshots") return;
  clearInterval(entry.timer);
  entry.socket?.close();
  entry.socket = null;
  try {
    entry.decoder?.close();
  } catch {}
  entry.decoder = null;
  pollDeviceScreen(entry);
}
function pollDeviceScreen(entry) {
  const { tab, s } = entry;
  entry.mode = "screenshots";
  const img = entry.el.querySelector("img");
  img.hidden = false;
  entry.el.querySelector("canvas").hidden = true;
  img.onload = () => sizeDeviceScreen(entry, img.naturalWidth, img.naturalHeight);
  img.onerror = null;
  let busy = false;
  const shoot = async () => {
    if (busy || !entry.el.isConnected) return;
    busy = true;
    try {
      img.src = await deviceCall(tab, { op: "screenshot", platform: s.target.platform, id: s.target.id });
      entry.screen.dataset.live = "screenshots";
    } catch {}
    busy = false;
  };
  shoot();
  entry.timer = setInterval(shoot, 700);
}

/** Touch, keys and text: on the stream's own socket when it is open, else through the machine. */
function sendDevice(entry, input) {
  const { tab, s } = entry;
  if (s.target.platform === "android") {
    if (entry.socket?.readyState === WebSocket.OPEN) return entry.socket.send(JSON.stringify({ ...input, ack: false }));
    if (input.type === "touch") return;
    return deviceCall(tab, { op: "input", platform: "android", id: s.target.id, input }).catch((e) => flash(String(e)));
  }
  if (entry.hid?.readyState !== WebSocket.OPEN) return;
  const tags = { touch: 0x03, button: 0x04, key: 0x06, orientation: 0x07 };
  const { kind, ...body } = input;
  const json = new TextEncoder().encode(JSON.stringify(body));
  const frame = new Uint8Array(json.length + 1);
  frame[0] = tags[kind];
  frame.set(json, 1);
  entry.hid.send(frame);
}
const deviceButton = (entry, name) => sendDevice(entry, entry.s.target.platform === "android" ? { type: name } : { kind: "button", button: name === "recents" ? "app_switcher" : name });
const ANDROID_KEYS = { Enter: 66, Backspace: 67, Tab: 61, Escape: 111, ArrowUp: 19, ArrowDown: 20, ArrowLeft: 21, ArrowRight: 22, Delete: 112 };
function wireDeviceInput(entry) {
  const screen = entry.el.querySelector(".device-screen");
  const at = (e) => {
    const r = screen.getBoundingClientRect();
    return { x: Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)), y: Math.min(1, Math.max(0, (e.clientY - r.top) / r.height)) };
  };
  let down = null;
  screen.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    screen.focus({ preventScroll: true });
    screen.setPointerCapture(e.pointerId);
    down = { ...at(e), moved: false };
    if (entry.s.target.platform === "ios") sendDevice(entry, { kind: "touch", type: "begin", x: down.x, y: down.y });
    else sendDevice(entry, { type: "touch", action: "down", x: down.x, y: down.y });
  });
  screen.addEventListener("pointermove", (e) => {
    if (!down) return;
    const p = at(e);
    if (Math.hypot(p.x - down.x, p.y - down.y) > 0.01) down.moved = true;
    if (entry.s.target.platform === "ios") sendDevice(entry, { kind: "touch", type: "move", ...p });
    else sendDevice(entry, { type: "touch", action: "move", ...p });
  });
  screen.addEventListener("pointerup", (e) => {
    if (!down) return;
    const p = at(e);
    if (entry.s.target.platform === "ios") sendDevice(entry, { kind: "touch", type: "end", ...p });
    else if (entry.socket?.readyState === WebSocket.OPEN) sendDevice(entry, { type: "touch", action: "up", ...p });
    // Without the socket a press goes as one tap or swipe.
    else sendDevice(entry, down.moved ? { type: "swipe", x1: down.x, y1: down.y, x2: p.x, y2: p.y, durationMs: 300 } : { type: "tap", x: p.x, y: p.y });
    down = null;
  });
  screen.addEventListener("keydown", (e) => {
    if (e.metaKey || e.ctrlKey) return;
    if (entry.s.target.platform === "android") {
      if (ANDROID_KEYS[e.key]) sendDevice(entry, { type: "key", keycode: ANDROID_KEYS[e.key] });
      else if (e.key.length === 1) sendDevice(entry, { type: "text", text: e.key });
      else return;
    } else return;
    e.preventDefault();
  });
  screen.addEventListener("paste", (e) => {
    const text = e.clipboardData.getData("text/plain");
    if (!text || entry.s.target.platform !== "android") return;
    e.preventDefault();
    const bytes = new TextEncoder();
    // serve-emu takes at most 300 bytes of text a message.
    let part = "";
    for (const ch of text) {
      if (bytes.encode(part + ch).length > 300) sendDevice(entry, { type: "text", text: part }), (part = "");
      part += ch;
    }
    if (part) sendDevice(entry, { type: "text", text: part });
  });
}

/** The accessibility frames over the screen, read every two seconds while the overlay is on. */
function normalizedTree(platform, payload) {
  if (platform === "ios") {
    const root = payload?.[0]?.frame ?? {};
    const [w, h] = [Math.max(1, root.width ?? 1), Math.max(1, root.height ?? 1)];
    const out = [];
    const visit = (node) => {
      if (!node?.frame || out.length >= 500) return;
      const f = node.frame;
      if (!(Math.abs(f.width - w) < 0.5 && Math.abs(f.height - h) < 0.5) && f.width > 0 && f.height > 0) out.push({ label: node.AXLabel ?? "", role: node.type ?? "", x: f.x / w, y: f.y / h, width: f.width / w, height: f.height / h });
      (node.children ?? []).forEach(visit);
    };
    (payload ?? []).forEach(visit);
    return out;
  }
  const nodes = (payload?.nodes ?? []).filter((n) => n.bounds);
  const root = nodes[0]?.bounds ?? {};
  const [w, h] = [Math.max(1, root.right ?? 1), Math.max(1, root.bottom ?? 1)];
  return nodes.slice(1).flatMap((n) => {
    const b = n.bounds;
    const [width, height] = [(b.right - b.left) / w, (b.bottom - b.top) / h];
    const label = n.text || n.contentDescription || "";
    if ((width >= 0.95 && height >= 0.9) || (!label && n.clickable !== true)) return [];
    return [{ label, role: (n.className ?? "").split(".").pop(), x: b.left / w, y: b.top / h, width, height }];
  });
}
function syncOverlay(entry) {
  const on = !!entry.s.overlay;
  const layer = entry.el.querySelector(".ax-layer");
  if (!on) {
    clearInterval(entry.treeTimer);
    entry.treeTimer = null;
    layer.innerHTML = "";
    return;
  }
  if (entry.treeTimer) return;
  const read = async () => {
    try {
      const tree = normalizedTree(entry.s.target.platform, await deviceCall(entry.tab, { op: "tree", platform: entry.s.target.platform, id: entry.s.target.id }));
      layer.innerHTML = tree.map((n) => `<span class="ax" style="left:${n.x * 100}%;top:${n.y * 100}%;width:${n.width * 100}%;height:${n.height * 100}%" title="${esc(`${n.role}${n.label ? ` · ${n.label}` : ""}`)}">${n.label ? `<span class="chip">${esc(n.label)}</span>` : ""}</span>`).join("");
    } catch {}
  };
  read();
  entry.treeTimer = setInterval(read, 2000);
}

const railButton = (act, ic, label, extra = "") => `<button type="button" class="icon" data-rail="${act}" aria-label="${esc(label)}" title="${esc(label)}"${extra}>${icon(ic)}</button>`;
function deviceRailHtml(s) {
  const android = s.target.platform === "android";
  const dark = dv(s).detail?.appearance === "dark";
  return `<aside class="device-rail" aria-label="Device controls"><div class="pill">
    ${railButton("home", "house", "Home")}
    ${android ? `${railButton("back", "chevron-left", "Back")}${railButton("recents", "square", "Recents")}` : ""}
    ${railButton("rotate", "rotate-ccw", "Rotate device")}
    <hr>
    ${railButton("appearance", dark ? "sun" : "moon", `Switch device to ${dark ? "light" : "dark"} mode`)}
    ${railButton("text-size", "type", "Device text size")}
    ${railButton("tools", "sliders-horizontal", "Device tools", ` aria-pressed="${!!s.tools}"${s.tools ? ` data-on` : ""}`)}
    ${railButton("screenshot", "camera", dv(s).capturing ? "Capturing screenshot" : "Save screenshot", dv(s).capturing ? " disabled" : "")}
    ${railButton("more", "ellipsis", "More device actions")}
  </div></aside>`;
}

const TEXT_SIZES = [["small", "Small"], ["default", "Default"], ["large", "Large"], ["extra-large", "Extra large"]];
const LOCATION_PRESETS = [["San Francisco", 37.7749, -122.4194], ["New York", 40.7128, -74.006], ["London", 51.5074, -0.1278], ["Stockholm", 59.3293, 18.0686], ["Tokyo", 35.6762, 139.6503]];
const DEVICE_PERMISSIONS = ["camera", "microphone", "photos", "contacts", "calendar", "location", "notifications", "motion"];
function deviceToolsHtml(s) {
  const d = dv(s).detail;
  const ios = s.target.platform === "ios";
  const toggle = (setting, label, on) => `<label class="tool-switch"><span>${esc(label)}</span><button type="button" class="switch${on ? " on" : ""}" role="switch" aria-checked="${!!on}" aria-label="${esc(label)}" data-tool-toggle="${setting}"></button></label>`;
  const seg = (name, options, value) => `<span class="seg" role="radiogroup" aria-label="${esc(name)}">${options.map(([v, label]) => `<button type="button" role="radio" aria-checked="${v === value}" data-tool-${name.toLowerCase().replace(/ /g, "-")}="${v}"${v === value ? " data-on" : ""}>${esc(label)}</button>`).join("")}</span>`;
  const sections = !d ? `<p class="muted loading">Reading device settings…</p>` : `
    <section><h4>App</h4><p class="row"><span class="muted">Foreground</span><span class="mono">${esc(d.foreground ?? "None")}</span></p>
      ${d.foreground ? `<p class="row buttons"><button type="button" class="outline" data-tool-app="terminate">Terminate</button><button type="button" class="outline" data-tool-app="relaunch">Relaunch</button></p>` : ""}
      <form class="row" data-tool-form="open-url"><input name="url" placeholder="https://… or myapp://" spellcheck="false" autocomplete="off"><button type="submit" class="outline">Open</button></form>
      <form class="row" data-tool-form="launch"><input name="app" placeholder="${ios ? "Bundle ID to launch" : "Package name to launch"}" spellcheck="false" autocomplete="off"><button type="submit" class="outline">Launch</button></form></section>
    <section><h4>${ios ? "Simulator" : "Emulator"}</h4>
      <p class="row"><span>Appearance</span>${seg("Appearance", [["light", "Light"], ["dark", "Dark"]], d.appearance)}</p>
      <p class="row"><span>Text size</span><select data-tool-select="text-size" aria-label="Text size">${TEXT_SIZES.map(([v, l]) => `<option value="${v}"${v === d.textSize ? " selected" : ""}>${l}</option>`).join("")}</select></p>
      ${ios ? toggle("increaseContrast", "Increase Contrast", d.increaseContrast) : `<p class="row"><span>Orientation</span><select data-tool-select="orientation" aria-label="Orientation"><option value="" selected disabled>Rotate to…</option><option value="portrait">Portrait</option><option value="landscape_left">Landscape</option><option value="portrait_upside_down">Upside down</option><option value="landscape_right">Landscape (right)</option></select></p>${toggle("networkEnabled", "Network", d.networkEnabled)}`}
      ${toggle("reduceMotion", "Reduce Motion", d.reduceMotion)}</section>
    <section><h4>Accessibility</h4>${toggle("overlay", "Overlay element frames", s.overlay)}</section>
    <section><h4>Location</h4><form data-tool-form="location"><p class="row"><input name="latitude" inputmode="decimal" placeholder="Latitude" aria-label="Latitude"><input name="longitude" inputmode="decimal" placeholder="Longitude" aria-label="Longitude"></p>
      <p class="row"><select data-tool-select="preset" aria-label="Preset"><option value="" selected disabled>Preset…</option>${LOCATION_PRESETS.map(([n]) => `<option>${n}</option>`).join("")}</select><button type="submit" class="outline">Set</button>${ios ? `<button type="button" class="outline" data-tool-app="clear-location">Clear</button>` : ""}</p></form></section>
    <section><h4>Permissions</h4><form data-tool-form="permission"><p class="row"><input name="app" placeholder="App ID" aria-label="App ID" spellcheck="false" autocomplete="off" value="${esc(d.foreground ?? "")}"><select name="permission" aria-label="Permission">${DEVICE_PERMISSIONS.map((p) => `<option>${p}</option>`).join("")}</select></p>
      <p class="row buttons"><button type="submit" class="outline" data-decision="grant">Grant</button><button type="submit" class="outline" data-decision="revoke">Revoke</button></p></form></section>
    ${ios ? `<section><h4>Push notification</h4>${d.foreground ? `<form class="row" data-tool-form="push"><input name="text" placeholder="Alert text" aria-label="Alert text"><button type="submit" class="outline">Send</button></form>` : `<p class="muted">Open an app first.</p>`}</section>` : ""}`;
  return `<aside class="device-tools" aria-label="Device tools"><header><h3>Tools</h3><button type="button" class="icon" data-rail="tools" aria-label="Close tools">${icon("x")}</button></header><div class="tools-body">${sections}</div></aside>`;
}

function renderDeviceWorkspace(tab, s, notices) {
  if (!deviceHost(tab.machine).hub) {
    if (!deviceHost(tab.machine).listing) listDevices(tab);
    return showPanel(`<div class="device-panel">${deviceLoadingHtml(s.target.name, `${machineName(tab.machine)} · ${s.target.version}`, "Opening device…", 2)}</div>`);
  }
  if (dv(s).detail === undefined && !dv(s).reading) {
    dv(s).reading = true;
    readDeviceDetail(tab, s);
  }
  const entry = deviceScreen(tab, s);
  const floating = panelOf(tab).pip === s.id;
  const html = `<div class="device-panel"><div class="device-ws${s.tools ? " with-tools" : ""}" data-device="${esc(s.id)}">${notices}
    <div class="device-stage">${floating ? `<div class="floating"><p class="muted">This device is floating over the chat.</p><button type="button" class="outline" data-device-unfloat>${icon("picture-in-picture-2")}Return it to the panel</button></div>` : ""}</div>${deviceRailHtml(s)}${s.tools ? deviceToolsHtml(s) : ""}</div></div>`;
  // A detail read re-renders the tools; what the user typed or picked in their forms survives it.
  const fields = [...panelBody.querySelectorAll(".device-tools [data-tool-form] [name]")];
  const same = (field) => panelBody.querySelector(`.device-tools [data-tool-form="${field.closest("[data-tool-form]").dataset.toolForm}"] [name="${field.name}"]`);
  const edited = fields.filter((field) => field.value !== (field.defaultValue ?? [...field.options].find((o) => o.defaultSelected)?.value ?? "")).map((field) => [field, field.value]);
  const focusedField = fields.find((field) => field === document.activeElement);
  const focused = document.activeElement?.closest?.("#panel-body .device-tools");
  const rebuilt = showPanel(html);
  if (rebuilt || (!floating && entry.el.parentElement !== panelBody.querySelector(".device-stage"))) {
    if (!floating) panelBody.querySelector(".device-stage").append(entry.el);
  }
  if (rebuilt) {
    for (const [field, value] of edited) {
      const again = same(field);
      if (again) again.value = value;
    }
    const again = focusedField && same(focusedField);
    if (again) {
      again.focus();
      if (again.type === "text") again.setSelectionRange(again.value.length, again.value.length);
    } else if (focused) panelBody.querySelector(".device-tools .tools-body")?.focus?.();
  }
  syncOverlay(entry);
}

/** The floating device over the chat, as T3's mini player holds it. */
function renderDevicePip(tab, s, title) {
  const entry = s.target && deviceScreen(tab, s);
  if (!entry) return;
  if (pipFrame.dataset.id !== s.id) {
    pipFrame.dataset.id = s.id;
    pipFrame.innerHTML = `<div class="page device-pip"></div><div class="pill"><span class="title">${esc(title)}</span><button type="button" class="icon" data-pip="panel" title="Open device in right panel">${icon("panel-right")}</button><button type="button" class="icon" data-pip="close" title="Close floating device">${icon("x")}</button></div>`;
  }
  pipFrame.querySelector(".title").textContent = title;
  const page = pipFrame.querySelector(".device-pip");
  if (entry.el.parentElement !== page) page.append(entry.el);
}

async function deviceAct(tab, s, action, done) {
  try {
    await deviceCall(tab, { op: "act", platform: s.target.platform, id: s.target.id, action });
    if (done) flash(done);
  } catch (e) {
    flash(String(e));
  }
  readDeviceDetail(tab, s);
}
async function readDeviceDetail(tab, s) {
  try {
    dv(s).detail = await deviceCall(tab, { op: "detail", platform: s.target.platform, id: s.target.id });
  } catch {
    dv(s).detail = {};
  }
  if (tab === active && shownSurface(tab) === s) renderPanel();
}
async function saveDeviceScreenshot(tab, s) {
  dv(s).capturing = true;
  renderPanel();
  try {
    const url = await deviceCall(tab, { op: "screenshot", platform: s.target.platform, id: s.target.id });
    const blob = await (await fetch(url)).blob();
    const name = `${s.target.name}-${new Date().toISOString().replace(/[:.]/g, "-")}.png`;
    if (TAURI) await saveCapture(name, blob, "Screenshot");
    else {
      const a = Object.assign(document.createElement("a"), { href: URL.createObjectURL(blob), download: name });
      a.click();
      setTimeout(() => URL.revokeObjectURL(a.href), 30000);
      flash("Screenshot saved");
    }
  } catch (e) {
    flash(`Couldn't capture the screen: ${e}`);
  }
  dv(s).capturing = false;
  if (tab === active) renderPanel();
}
async function powerOffDevice(tab, s) {
  try {
    await deviceCall(tab, { op: "shutdown", platform: s.target.platform, id: s.target.id });
    closeDeviceScreen(s.id);
    closeSurface(s.id);
    listDevices(tab);
  } catch (e) {
    dv(s).error = String(e);
    renderPanel();
  }
}

function railClick(tab, s, button) {
  const act = button.dataset.rail;
  const entry = deviceScreens.get(s.id);
  const android = s.target.platform === "android";
  if (act === "home" || act === "back" || act === "recents") return entry && deviceButton(entry, act);
  if (act === "rotate") {
    if (android) return toggleMenu(button, [{ label: "Portrait", run: () => deviceAct(tab, s, { type: "setOrientation", value: "portrait" }) }, { label: "Landscape", run: () => deviceAct(tab, s, { type: "setOrientation", value: "landscape_left" }) }]);
    const order = ["portrait", "landscape_left", "portrait_upside_down", "landscape_right"];
    entry.orientation = order[(order.indexOf(entry.orientation ?? "portrait") + 1) % 4];
    return sendDevice(entry, { kind: "orientation", orientation: entry.orientation });
  }
  if (act === "appearance") {
    const value = dv(s).detail?.appearance === "dark" ? "light" : "dark";
    dv(s).detail = { ...dv(s).detail, appearance: value };
    renderPanel();
    return deviceAct(tab, s, { type: "setAppearance", value });
  }
  if (act === "text-size") return toggleMenu(button, TEXT_SIZES.map(([value, label]) => ({ label, checked: (dv(s).detail?.textSize ?? "default") === value, run: () => deviceAct(tab, s, { type: "setTextSize", value }) })));
  if (act === "tools") {
    s.tools = !s.tools;
    savePanel(tab);
    if (s.tools) readDeviceDetail(tab, s);
    return renderPanel();
  }
  if (act === "screenshot") return saveDeviceScreenshot(tab, s);
  if (act === "more") {
    return toggleMenu(button, [
      { label: "Float device over chat", icon: "picture-in-picture-2", run: () => {
        panelOf(tab).pip = s.id;
        panelOf(tab).open = false;
        savePanel(tab);
        renderPanel();
      } },
      { label: "Close device panel", icon: "x", run: () => closeSurface(s.id) },
      "-",
      { label: "Power off device", icon: "power", danger: true, run: () => powerOffDevice(tab, s) },
    ]);
  }
}

panelBody.addEventListener("click", (e) => {
  const tab = active;
  const s = tab && shownSurface(tab);
  if (s?.kind !== "device") return;
  const host = deviceHost(tab.machine);
  const open = e.target.closest("[data-device-open]");
  if (open) {
    const [platform, id] = open.dataset.deviceOpen.split("\n");
    const device = host.list?.devices.find((d) => d.platform === platform && d.id === id);
    if (device) openDevice(tab, s, device);
    return;
  }
  if (e.target.closest("[data-device-refresh]")) return listDevices(tab).then(() => renderPanel());
  if (e.target.closest("[data-device-dismiss]")) {
    if (dv(s).error) dv(s).error = null;
    else host.dismissed = true;
    return renderPanel();
  }
  if (e.target.closest("[data-device-unfloat]")) {
    panelOf(tab).pip = null;
    savePanel(tab);
    return renderPanel();
  }
  const rail = e.target.closest("[data-rail]");
  if (rail && s.target) return railClick(tab, s, rail);
  const tool = e.target.closest("[data-tool-toggle]");
  if (tool && s.target) {
    const setting = tool.dataset.toolToggle;
    if (setting === "overlay") {
      s.overlay = !s.overlay;
      savePanel(tab);
      return renderPanel();
    }
    const value = !dv(s).detail?.[setting];
    dv(s).detail = { ...dv(s).detail, [setting]: value };
    renderPanel();
    return deviceAct(tab, s, { type: "setToggle", setting, value });
  }
  const look = e.target.closest("[data-tool-appearance]");
  if (look && s.target) return deviceAct(tab, s, { type: "setAppearance", value: look.dataset.toolAppearance });
  const app = e.target.closest("[data-tool-app]");
  if (app && s.target) {
    const id = dv(s).detail?.foreground;
    if (app.dataset.toolApp === "terminate") return deviceAct(tab, s, { type: "terminateApp", appId: id });
    if (app.dataset.toolApp === "relaunch") return deviceAct(tab, s, { type: "terminateApp", appId: id }).then(() => deviceAct(tab, s, { type: "launchApp", appId: id }));
    if (app.dataset.toolApp === "clear-location") return deviceAct(tab, s, { type: "clearLocation" });
  }
});
panelBody.addEventListener("change", (e) => {
  const tab = active;
  const s = tab && shownSurface(tab);
  const pick = e.target.closest("[data-tool-select]");
  if (s?.kind !== "device" || !s.target || !pick) return;
  const which = pick.dataset.toolSelect;
  if (which === "text-size") return deviceAct(tab, s, { type: "setTextSize", value: pick.value });
  if (which === "orientation") return deviceAct(tab, s, { type: "setOrientation", value: pick.value });
  if (which === "preset") {
    const [, latitude, longitude] = LOCATION_PRESETS.find(([n]) => n === pick.value);
    const form = pick.closest("form");
    form.latitude.value = latitude;
    form.longitude.value = longitude;
  }
});
let deviceDecision = "grant";
panelBody.addEventListener("pointerdown", (e) => {
  const b = e.target.closest("[data-decision]");
  if (b) deviceDecision = b.dataset.decision;
});
panelBody.addEventListener("submit", (e) => {
  const tab = active;
  const s = tab && shownSurface(tab);
  const form = e.target.closest("[data-tool-form]");
  if (s?.kind !== "device" || !s.target || !form) return;
  e.preventDefault();
  const decision = e.submitter?.dataset.decision ?? deviceDecision;
  const kind = form.dataset.toolForm;
  if (kind === "open-url" && form.url.value.trim()) deviceAct(tab, s, { type: "openUrl", url: form.url.value.trim() });
  else if (kind === "launch" && form.app.value.trim()) deviceAct(tab, s, { type: "launchApp", appId: form.app.value.trim() });
  else if (kind === "location") {
    const [latitude, longitude] = [Number(form.latitude.value), Number(form.longitude.value)];
    if (form.latitude.value && form.longitude.value && Number.isFinite(latitude) && Number.isFinite(longitude)) deviceAct(tab, s, { type: "setLocation", latitude, longitude }, "Location set");
  } else if (kind === "permission" && form.app.value.trim()) deviceAct(tab, s, { type: "setPermission", permission: form.permission.value, decision, appId: form.app.value.trim() }, `${decision === "grant" ? "Granted" : "Revoked"} ${form.permission.value}`);
  else if (kind === "push" && form.text.value.trim()) deviceAct(tab, s, { type: "sendPush", appId: dv(s).detail.foreground, text: form.text.value.trim() }, "Notification sent");
});

/** A Device tab that closes ends its screen's stream. */
setInterval(() => {
  const keep = new Set([...panelStates.values()].flatMap((p) => p.surfaces.map((s) => s.id)));
  for (const id of deviceScreens.keys()) if (!keep.has(id)) closeDeviceScreen(id);
}, 1000);
