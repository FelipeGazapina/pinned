Object.assign(ICON_PATHS, {
  "unlink-2": `<path d="M15 7h2a5 5 0 0 1 0 10h-2"/><path d="M9 17H7A5 5 0 0 1 7 7"/>`,
  "screen-rotation": `<rect x="7.25" y="7.25" width="9.5" height="9.5" rx="1.4" transform="rotate(-45 12 12)"/><path d="M12.5 2a10 10 0 0 1 8.4 5.4"/><path d="M20.8 3.5v4h-4"/><path d="M11.5 22a10 10 0 0 1-8.4-5.4"/><path d="M3.2 20.5v-4h4"/>`,
});

/** The pages this device opened, newest first, as T3's browser history keeps them: the address, the
 * page's title and icon once known, and when it was last visited. */
let browserRecents = [];
try {
  browserRecents = (JSON.parse(load("gui-browser-recent")) ?? []).map((e) => (typeof e === "string" ? { url: e, at: 0 } : e)).filter((e) => e?.url);
} catch {}
const saveRecents = () => save("gui-browser-recent", JSON.stringify(browserRecents));
function rememberVisit(url) {
  if (!url) return;
  const was = browserRecents.find((e) => e.url === url);
  browserRecents = [{ ...was, url, at: Date.now() }, ...browserRecents.filter((e) => e.url !== url)].slice(0, 8);
  saveRecents();
}
/** What a page told about itself after it loaded: its title, and its icon. */
function rememberPage(url, { title, icon: favicon }) {
  const entry = browserRecents.find((e) => e.url === url);
  if (!entry || ((title ?? entry.title) === entry.title && (favicon ?? entry.icon) === entry.icon)) return;
  Object.assign(entry, title ? { title } : {}, favicon ? { icon: favicon } : {});
  saveRecents();
}
function forgetVisit(url) {
  browserRecents = browserRecents.filter((e) => e.url !== url);
  saveRecents();
}

const sinceText = (ms) => {
  const s = Math.floor((Date.now() - ms) / 1000);
  if (!ms || s < 5) return "just now";
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
};

const browserMockup = `<span class="mockup" aria-hidden="true"><span class="dots"><i></i><i></i><i></i></span><span class="lines"><i></i><i></i></span></span>`;
/** A page's icon from the first of `sources` that loads, else a little browser window. */
function faviconHtml(sources) {
  const [first, ...rest] = sources.filter(Boolean);
  return first ? `<img class="favicon" src="${esc(first)}" data-next="${esc(rest.join(" "))}" alt="" draggable="false">${browserMockup}` : browserMockup;
}
document.addEventListener("error", (e) => {
  const img = e.target;
  if (!img.matches?.(".browser-empty img.favicon")) return;
  const [next, ...rest] = img.dataset.next.split(" ").filter(Boolean);
  if (next) {
    img.dataset.next = rest.join(" ");
    img.src = next;
  } else img.remove();
}, true);
const originIcon = (url) => {
  try {
    return `${new URL(url).origin}/favicon.ico`;
  } catch {
    return "";
  }
};

function discoveryRow(url, title, description, sources, extra = "") {
  return `<div class="row"><button type="button" data-url="${esc(url)}">${faviconHtml(sources)}<span class="text"><b>${esc(title)}</b><span>${esc(description)}</span></span></button>${extra}</div>`;
}

/** The empty browser tab, as T3's PreviewEmptyState draws it: the pages this device opened and the
 * thread's own local servers, or a plain "No preview yet" while it has neither. */
function browserEmptyHtml(tab) {
  const t = tab && threadOf(tab);
  if (tab && !t.ports && t.ports !== null) fetchPorts(tab);
  // A node too old to list ports leaves the addresses its transcript mentions.
  const servers = t?.ports === null ? seenUrls(tab).map((url) => ({ url, name: "Listening" }))
    : (t?.ports ?? []).filter((p) => p.own).map((p) => ({ url: `http://localhost:${p.port}`, name: p.command || "Listening" }));
  const recents = browserRecents.filter((e) => URL.canParse(e.url)).slice(0, 8);
  const remote = tab && tab.machine !== LOCAL ? `<p class="hint">This thread runs on ${esc(machineName(tab.machine))}, so its local servers are that machine's, not this device's.</p>` : "";
  if (!servers.length && !recents.length) {
    return `<div class="browser-empty blank"><span class="media">${icon("globe")}</span><p class="title">No preview yet</p>
      <p>Type a URL above, or run a dev script. Browser-ready localhost servers will show up here automatically.</p>${remote}</div>`;
  }
  const recentRows = recents.map((e) => {
    const u = new URL(e.url);
    const label = `${u.host}${u.pathname === "/" ? "" : u.pathname}${u.search}${u.hash}`;
    return discoveryRow(e.url, e.title || label, `${e.title ? `${label} · ` : ""}${sinceText(e.at)}`, [e.icon, originIcon(e.url)],
      `<button type="button" class="icon forget" data-forget="${esc(e.url)}" aria-label="Remove ${esc(label)} from history" title="Remove from history">${icon("x")}</button>`);
  });
  const serverRows = servers.map((r) => discoveryRow(r.url, r.name, new URL(r.url).host, [browserRecents.find((e) => new URL(e.url).origin === new URL(r.url).origin)?.icon, originIcon(r.url)]));
  return `<div class="browser-empty lists">
    ${recentRows.length ? `<section><p class="group">${icon("history")}Recently used</p><div class="servers">${recentRows.join("")}</div></section>` : ""}
    ${serverRows.length ? `<section><p class="group">${icon("radio-tower")}Local servers</p><div class="servers">${serverRows.join("")}</div>
      <p class="hint">Select a live local server to open it in this browser tab.</p></section>` : ""}${remote}</div>`;
}

panelBody.addEventListener("click", (e) => {
  const forget = e.target.closest(".browser [data-forget]");
  if (!forget) return;
  e.stopPropagation();
  forgetVisit(forget.dataset.forget);
  const s = shownSurface(active);
  if (s?.kind === "browser" && !s.url) renderBrowser(s, true);
}, true);

/** Chrome DevTools' standard devices, in its order, as T3's previewViewport.ts lists them. */
const DEVICE_PRESETS = [
  ["iphone-se", "iPhone SE", 375, 667], ["iphone-xr", "iPhone XR", 414, 896], ["iphone-12-pro", "iPhone 12 Pro", 390, 844],
  ["iphone-14-pro-max", "iPhone 14 Pro Max", 430, 932], ["pixel-7", "Pixel 7", 412, 915], ["galaxy-s8-plus", "Samsung Galaxy S8+", 360, 740],
  ["galaxy-s20-ultra", "Samsung Galaxy S20 Ultra", 412, 915], ["ipad-mini", "iPad Mini", 768, 1024], ["ipad-air", "iPad Air", 820, 1180],
  ["ipad-pro", "iPad Pro", 1024, 1366], ["surface-pro-7", "Surface Pro 7", 912, 1368], ["surface-duo", "Surface Duo", 540, 720],
  ["galaxy-z-fold-5", "Galaxy Z Fold 5", 344, 882], ["zenbook-fold", "Asus Zenbook Fold", 853, 1280], ["galaxy-a51", "Samsung Galaxy A51/71", 412, 914],
  ["nest-hub", "Nest Hub", 1024, 600], ["nest-hub-max", "Nest Hub Max", 1280, 800],
].map(([id, label, w, h]) => ({ id, label, w, h }));
const VIEWPORT_MIN = 240;
const VIEWPORT_MAX = 3840;
const VIEWPORT_AREA = 3840 * 2160;
const DEVICE_BAR = 38;
const RAIL = 10;

/** A sized viewport, `s.device`: `{ preset, w, h }`, where a null preset is Responsive. No device is
 * the page filling the panel, with the toolbar hidden. `s.aspect` is the locked width over height. */
const devicePreset = (s) => DEVICE_PRESETS.find((p) => p.id === s.device?.preset);
const validSize = (w, h) => Number.isInteger(w) && Number.isInteger(h) && w >= VIEWPORT_MIN && w <= VIEWPORT_MAX && h >= VIEWPORT_MIN && h <= VIEWPORT_MAX && w * h <= VIEWPORT_AREA;
const clampSide = (v) => Math.min(VIEWPORT_MAX, Math.max(VIEWPORT_MIN, v));

function sizeAtAspect(desired, aspect, axis) {
  if (axis === "w") {
    const min = Math.ceil(Math.max(VIEWPORT_MIN, VIEWPORT_MIN * aspect));
    const max = Math.floor(Math.min(VIEWPORT_MAX, VIEWPORT_MAX * aspect, Math.sqrt(VIEWPORT_AREA * aspect)));
    let w = Math.min(max, Math.max(min, Math.round(desired)));
    let h = Math.round(w / aspect);
    while (w * h > VIEWPORT_AREA && w > min) h = Math.round(--w / aspect);
    return { w, h };
  }
  const min = Math.ceil(Math.max(VIEWPORT_MIN, VIEWPORT_MIN / aspect));
  const max = Math.floor(Math.min(VIEWPORT_MAX, VIEWPORT_MAX / aspect, Math.sqrt(VIEWPORT_AREA / aspect)));
  let h = Math.min(max, Math.max(min, Math.round(desired)));
  let w = Math.round(h * aspect);
  while (w * h > VIEWPORT_AREA && h > min) w = Math.round(--h * aspect);
  return { w, h };
}

/** T3's resizeFreeformViewport: `start` grown by a pointer `delta` dragged from edge `dir`. */
function resizeViewport(start, delta, zoom, dir, aspect) {
  const dx = dir.includes("e") ? delta.x : dir.includes("w") ? -delta.x : 0;
  const dy = dir.includes("s") ? delta.y : dir.includes("n") ? -delta.y : 0;
  const w = start.w + dx / zoom;
  const h = start.h + dy / zoom;
  if (aspect) {
    const width = dx !== 0 || dir === "e" || dir === "w";
    const height = dy !== 0 || dir === "n" || dir === "s";
    const axis = width && !height ? "w" : height && !width ? "h" : Math.abs(w - start.w) / start.w >= Math.abs(h - start.h) / start.h ? "w" : "h";
    return sizeAtAspect(axis === "w" ? w : h, aspect, axis);
  }
  let out = { w: clampSide(Math.round(w)), h: clampSide(Math.round(h)) };
  if (out.w * out.h <= VIEWPORT_AREA) return out;
  if (Math.abs(dx) >= Math.abs(dy)) out.w = Math.max(VIEWPORT_MIN, Math.floor(VIEWPORT_AREA / out.h));
  else out.h = Math.max(VIEWPORT_MIN, Math.floor(VIEWPORT_AREA / out.w));
  return out;
}

/** The centered page grows on both sides, so a rail drag moves its edge with the pointer until the
 * page fills the room, then grows only that way: T3's resizeBrowserViewportFromRail. */
function resizeFromRail(start, delta, room, zoom, dir, aspect) {
  const fromEnd = (size, d, avail) => {
    const edge = size < avail ? (avail + size) / 2 : size;
    const to = edge + d;
    return to <= avail ? to * 2 - avail : to;
  };
  const fromStart = (size, d, avail) => {
    if (size > avail) return d <= size - avail ? size - d : avail - (d - (size - avail)) * 2;
    const to = (avail - size) / 2 + d;
    return to >= 0 ? avail - to * 2 : avail - to;
  };
  const [sw, sh] = [start.w * zoom, start.h * zoom];
  const w = dir.includes("e") ? fromEnd(sw, delta.x, room.w) : dir.includes("w") ? fromStart(sw, delta.x, room.w) : sw;
  const h = dir.includes("s") ? fromEnd(sh, delta.y, room.h) : dir.includes("n") ? fromStart(sh, delta.y, room.h) : sh;
  return resizeViewport(start, { x: dir.includes("w") ? -(w - sw) : w - sw, y: dir.includes("n") ? -(h - sh) : h - sh }, zoom, dir, aspect);
}

/** The room a sized page has inside the panel's viewport, between the side and bottom rails. */
const deviceRoom = (viewport, toolbar = 0) => {
  const r = viewport.getBoundingClientRect();
  return { w: Math.max(1, r.width - RAIL * 2), h: Math.max(1, r.height - toolbar - RAIL) };
};

function deviceBarHtml(s) {
  const width = panelBody.clientWidth || 600;
  const { w, h } = s.device;
  const preset = devicePreset(s);
  return `<div class="device-bar${width >= 360 ? "" : " tight"}" role="toolbar" aria-label="Browser device toolbar">
    ${width >= 560 ? `<span class="dims-label">Dimensions</span>` : ""}
    <button type="button" class="pick${width >= 440 ? "" : " narrow"}" data-device aria-label="Browser device preset"><span>${esc(preset?.label ?? "Responsive")}</span>${icon("chevrons-up-down")}</button>
    <span class="dims"><input type="number" inputmode="numeric" data-size="w" min="${VIEWPORT_MIN}" max="${VIEWPORT_MAX}" value="${w}" aria-label="Viewport width"><span class="muted">×</span><input type="number" inputmode="numeric" data-size="h" min="${VIEWPORT_MIN}" max="${VIEWPORT_MAX}" value="${h}" aria-label="Viewport height"></span>
    <button type="button" class="icon${s.aspect ? " on" : ""}" data-device-do="lock" aria-pressed="${!!s.aspect}" aria-label="${s.aspect ? "Unlock viewport aspect ratio" : "Lock viewport aspect ratio"}" title="${s.aspect ? "Unlock aspect ratio" : "Lock aspect ratio"}">${icon(s.aspect ? "link-2" : "unlink-2")}</button>
    <button type="button" class="icon" data-device-do="rotate" aria-label="Rotate viewport" title="Rotate viewport">${icon("screen-rotation")}</button>
    <span class="close"><button type="button" class="icon" data-device-do="close" aria-label="Close device toolbar" title="Close device toolbar">${icon("x")}</button></span></div>`;
}

const RESIZE_HANDLES = [["w", "left edge"], ["e", "right edge"], ["s", "bottom edge"], ["sw", "bottom-left corner"], ["se", "bottom-right corner"]];
const resizeHandlesHtml = () => RESIZE_HANDLES.map(([dir, where]) => `<button type="button" class="rail ${dir}" data-resize="${dir}" aria-label="Resize browser viewport from ${where}. Use arrow keys to resize."><span></span></button>`).join("");

/** Sizes the page's box to its device, scaled down to fit the room, and answers that scale. The web
 * build's frame lays the page out at the device's size and zoom inside the box. */
function fitPage(s, box) {
  const zoom = s.zoom ?? 1;
  const frame = box.querySelector("iframe");
  if (!s.device) {
    box.style.cssText = "";
    if (frame) frame.style.cssText = `width:${100 / zoom}%;height:${100 / zoom}%;transform:scale(${zoom})`;
    const r = box.getBoundingClientRect();
    if (r.width > 2 && r.height > 2) s.viewSize = { w: Math.round(r.width), h: Math.round(r.height) };
    return 1;
  }
  const room = deviceRoom(box.closest(".viewport"));
  const scale = Math.min(1, room.w / (s.device.w * zoom), room.h / (s.device.h * zoom));
  box.style.cssText = `width:${Math.round(s.device.w * zoom * scale)}px;height:${Math.round(s.device.h * zoom * scale)}px`;
  if (frame) frame.style.cssText = `width:${s.device.w}px;height:${s.device.h}px;transform:scale(${zoom * scale})`;
  return scale;
}
const fitShownPage = () => {
  const s = active && shownSurface(active);
  const box = panelBody.querySelector(".browser .viewport .page");
  if (isPage(s) && box) fitPage(s, box);
};
new ResizeObserver(() => fitShownPage()).observe(panelBody);

function setViewport(s, device, aspect = s.aspect ?? null) {
  s.device = device;
  s.aspect = device ? aspect : null;
  savePanel(browserTab(s) ?? active);
  renderBrowser(s);
}
function syncDeviceBar(s) {
  const bar = panelBody.querySelector(".device-bar");
  if (bar && s.device) bar.outerHTML = deviceBarHtml(s);
}

/** Show device toolbar: a Responsive viewport the size the page has now, as T3 opens it. */
function toggleDeviceToolbar(s) {
  if (s.device) return setViewport(s, null);
  const viewport = panelBody.querySelector(".browser .viewport");
  const zoom = s.zoom ?? 1;
  const room = viewport ? deviceRoom(viewport, DEVICE_BAR) : { w: 1280, h: 800 };
  setViewport(s, { preset: null, ...resizeViewport({ w: room.w / zoom, h: room.h / zoom }, { x: 0, y: 0 }, 1, "se") }, null);
}

function deviceMenu(s) {
  const preset = devicePreset(s);
  return [
    { label: "Responsive", checked: !preset, run: () => preset && setViewport(s, { preset: null, w: s.device.w, h: s.device.h }) },
    { group: "Standard" },
    ...DEVICE_PRESETS.map((p) => ({ label: p.label, hint: `${p.w} × ${p.h}`, checked: preset === p, run: () => setViewport(s, { preset: p.id, w: p.w, h: p.h }, s.aspect ? p.w / p.h : null) })),
  ];
}

panelBody.addEventListener("click", (e) => {
  const s = active && shownSurface(active);
  if (!isPage(s) || !s.device) return;
  const pick = e.target.closest(".device-bar [data-device]");
  if (pick) return toggleMenu(pick, deviceMenu(s));
  const what = e.target.closest(".device-bar [data-device-do]")?.dataset.deviceDo;
  if (what === "close") setViewport(s, null);
  else if (what === "rotate") setViewport(s, { ...s.device, w: s.device.h, h: s.device.w }, s.aspect ? 1 / s.aspect : null);
  else if (what === "lock") {
    s.aspect = s.aspect ? null : s.device.w / s.device.h;
    savePanel(active);
    syncDeviceBar(s);
  }
});
panelBody.addEventListener("input", (e) => {
  const s = active && shownSurface(active);
  if (!isPage(s) || !s.device?.w || !s.aspect || !e.target.matches(".device-bar [data-size]")) return;
  const value = Number(e.target.value);
  if (!Number.isInteger(value) || value < VIEWPORT_MIN || value > VIEWPORT_MAX) return;
  const axis = e.target.dataset.size;
  const sized = resizeViewport(s.device, axis === "w" ? { x: value - s.device.w, y: 0 } : { x: 0, y: value - s.device.h }, 1, axis === "w" ? "e" : "s", s.aspect);
  panelBody.querySelector(`.device-bar [data-size="${axis === "w" ? "h" : "w"}"]`).value = axis === "w" ? sized.h : sized.w;
});
panelBody.addEventListener("change", (e) => {
  const s = active && shownSurface(active);
  if (!isPage(s) || !s.device || !e.target.matches(".device-bar [data-size]")) return;
  const [w, h] = [...panelBody.querySelectorAll(".device-bar [data-size]")].map((i) => Number(i.value));
  if (!validSize(w, h)) return syncDeviceBar(s);
  if (w !== s.device.w || h !== s.device.h) setViewport(s, { preset: null, w, h });
});
panelBody.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && e.target.matches(".device-bar [data-size]")) e.target.dispatchEvent(new Event("change", { bubbles: true }));
});

/** Dragging a rail, or an arrow key on one, resizes the viewport the way T3's handles do. */
function resizeDevice(s, dir, room, start, delta, rail) {
  const zoom = s.zoom ?? 1;
  const size = rail ? resizeFromRail(start, delta, room, zoom, dir, s.aspect) : resizeViewport(start, delta, zoom, dir, s.aspect);
  s.device = { preset: null, ...size };
  const box = panelBody.querySelector(".browser .viewport .page");
  if (box) fitPage(s, box);
  for (const input of panelBody.querySelectorAll(".device-bar [data-size]")) input.value = s.device[input.dataset.size];
  const pick = panelBody.querySelector(".device-bar [data-device] span");
  if (pick) pick.textContent = "Responsive";
  placePages(true);
}
panelBody.addEventListener("pointerdown", (down) => {
  const handle = down.target.closest(".browser [data-resize]");
  const s = active && shownSurface(active);
  if (!handle || !isPage(s) || !s.device || down.button !== 0) return;
  down.preventDefault();
  const dir = handle.dataset.resize;
  const start = { w: s.device.w, h: s.device.h };
  const room = deviceRoom(handle.closest(".viewport"));
  handle.setPointerCapture(down.pointerId);
  handle.classList.add("active");
  const readout = Object.assign(document.createElement("div"), { className: "device-size", textContent: `${start.w} × ${start.h}` });
  handle.parentElement.append(readout);
  const move = (e) => {
    resizeDevice(s, dir, room, start, { x: e.clientX - down.clientX, y: e.clientY - down.clientY }, true);
    readout.textContent = `${s.device.w} × ${s.device.h}`;
  };
  const end = () => {
    handle.removeEventListener("pointermove", move);
    handle.removeEventListener("lostpointercapture", end);
    handle.classList.remove("active");
    readout.remove();
    savePanel(active);
  };
  handle.addEventListener("pointermove", move);
  handle.addEventListener("lostpointercapture", end);
});
panelBody.addEventListener("keydown", (e) => {
  const handle = e.target.closest?.(".browser [data-resize]");
  const s = active && shownSurface(active);
  if (!handle || !isPage(s) || !s.device) return;
  const dir = handle.dataset.resize;
  const step = (e.shiftKey ? 50 : 10) * (s.zoom ?? 1);
  const delta = e.key === "ArrowLeft" && /[ew]/.test(dir) ? { x: -step, y: 0 } : e.key === "ArrowRight" && /[ew]/.test(dir) ? { x: step, y: 0 }
    : e.key === "ArrowUp" && dir.includes("s") ? { x: 0, y: -step } : e.key === "ArrowDown" && dir.includes("s") ? { x: 0, y: step } : null;
  if (!delta) return;
  e.preventDefault();
  resizeDevice(s, dir, null, s.device, delta, false);
  savePanel(active);
});

/** The address bar's search engines, as T3 offers them; `%s` is where the query goes. */
const SEARCH_ENGINES = {
  duckduckgo: ["DuckDuckGo", "https://duckduckgo.com/?q=%s"],
  google: ["Google", "https://www.google.com/search?q=%s"],
  bing: ["Bing", "https://www.bing.com/search?q=%s"],
  brave: ["Brave Search", "https://search.brave.com/search?q=%s"],
  kagi: ["Kagi", "https://kagi.com/search?q=%s"],
  ecosia: ["Ecosia", "https://www.ecosia.org/search?q=%s"],
  custom: ["Custom", ""],
};
const searchEngine = () => (SEARCH_ENGINES[load("gui-browser-search")] ? load("gui-browser-search") : "duckduckgo");
const fillSearch = (template, query) => template.replaceAll("%s", encodeURIComponent(query));
/** A custom search URL is an http(s) address with `%s` past its host, so a query can't change where it goes. */
function validSearchTemplate(template) {
  if (!template.includes("%s")) return false;
  try {
    const [a, b] = [new URL(fillSearch(template, "a")), new URL(fillSearch(template, "b"))];
    return /^https?:$/.test(a.protocol) && `${a.username}:${a.password}@${a.host}` === `${b.username}:${b.password}@${b.host}`;
  } catch {
    return false;
  }
}
function searchTemplate() {
  const engine = searchEngine();
  if (engine !== "custom") return SEARCH_ENGINES[engine][1];
  const custom = (load("gui-browser-search-url") ?? "").trim();
  return validSearchTemplate(custom) ? custom : SEARCH_ENGINES.duckduckgo[1];
}

/** What the address bar opens for what was typed: T3's resolveAddressBarInput. An address goes as it
 * is, a loopback host over http, a bare host or host:port over https, and anything else is searched.
 * A non-web scheme opens nothing. */
function addressOf(raw) {
  const typed = raw.trim();
  const normalize = (text) => {
    const loopback = /^(?:localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1?\])(?::|\/|$)/i.test(text);
    const url = new URL(/^[a-z][a-z\d+.-]*:\/\//i.test(text) ? text : `${loopback ? "http" : "https"}://${text}`);
    if (!/^https?:$/.test(url.protocol)) throw new Error(`can't open ${url.protocol} addresses`);
    return url.href;
  };
  if (!typed) return null;
  try {
    if (/^[a-z][a-z\d+.-]*:\/\//i.test(typed)) return normalize(typed);
    if (!/\s/.test(typed)) {
      const scheme = /^([a-z][a-z\d+.-]*):/i.exec(typed)?.[1]?.toLowerCase();
      const nonWeb = ["about", "blob", "chrome", "data", "file", "ftp", "javascript", "mailto", "sms", "ssh", "tel", "view-source", "ws", "wss"];
      if (scheme && (nonWeb.includes(scheme) || !/^[^/?#:@]+:\d+(?:[/?#]|$)/.test(typed))) return /^https?$/.test(scheme) ? normalize(typed) : null;
      const authority = typed.split(/[/?#]/, 1)[0].replace(/^[^@]*@/, "");
      const host = authority.replace(/:\d+$/, "");
      if (host.length < authority.length || /^(?:localhost|\d{1,3}(?:\.\d{1,3}){3}|\[[\da-f:.]+\])$/i.test(host) || /^[^.]+(?:\.[^.]+)+\.?$/.test(host)) {
        try {
          return normalize(typed);
        } catch {}
      }
    }
  } catch {
    return null;
  }
  return fillSearch(searchTemplate(), typed);
}

/** "Open links in": where a plain click on a web link in the chat or a terminal goes. */
const linksOpenInApp = () => load("gui-browser-links") === "app";
const recordsKeys = () => load("gui-browser-rec-keys") === "true";
const recordsMouse = () => load("gui-browser-rec-mouse") === "true";

function browserSettingsHtml() {
  const row = (key, title, about, control, status = "") => `<div class="side-setting" data-setting="${key}"><div><label for="browser-${key}">${title}</label><p class="meta">${about}</p>${status}</div>${control}</div>`;
  const select = (key, value, options, label) => `<select id="browser-${key}" aria-label="${label}">${Object.entries(options).map(([v, name]) => `<option value="${v}"${v === value ? " selected" : ""}>${esc(name)}</option>`).join("")}</select>`;
  const toggle = (key, on, title, about) => row(key, title, about, `<label class="switch"><input type="checkbox" id="browser-${key}" aria-label="${title}"${on ? " checked" : ""}><span></span></label>`);
  const engine = searchEngine();
  const custom = load("gui-browser-search-url") ?? "";
  return `<section id="browser-settings"><h2 class="looks-title">Browser</h2>
    ${row("links", "Open links in", "Where links in the chat and terminal open. Hold ⌘ or Ctrl while clicking a link to open it in your default browser either way.",
      select("links", linksOpenInApp() ? "app" : "system", { system: "Your default browser", app: "pinned" }, "Open links in"))}
    ${row("search", "Search engine", "What the address bar searches when you type something that isn't a URL.",
      select("search", engine, Object.fromEntries(Object.entries(SEARCH_ENGINES).map(([k, [name]]) => [k, name])), "Search engine"))}
    ${engine === "custom" ? row("search-url", "Custom search URL", "Use %s where the search terms go.",
      `<input id="browser-search-url" type="text" placeholder="https://example.com/search?q=%s" autocapitalize="none" spellcheck="false" maxlength="2048" value="${esc(custom)}" aria-label="Custom search URL">`,
      validSearchTemplate(custom.trim()) ? "" : `<p class="meta error">Enter an http or https URL containing %s. Searches use DuckDuckGo until then.</p>`) : ""}
    ${NATIVE_PAGES ? `${toggle("rec-keys", recordsKeys(), "Show key presses in recordings", "Show pressed keys and shortcuts in new recordings. Password fields are excluded.")}
    ${toggle("rec-mouse", recordsMouse(), "Show mouse presses in recordings", "Highlight mouse presses and held buttons in new recordings.")}` : ""}</section>`;
}
function bindBrowserSettings() {
  const root = document.getElementById("browser-settings");
  if (!root) return;
  const rebuild = () => {
    // Replacing the focused field blurs it, which fires its change again on the detached root.
    if (!root.isConnected) return;
    root.outerHTML = browserSettingsHtml();
    bindBrowserSettings();
  };
  root.querySelector("#browser-links").onchange = (e) => save("gui-browser-links", e.target.value);
  root.querySelector("#browser-search").onchange = (e) => {
    save("gui-browser-search", e.target.value);
    rebuild();
  };
  const custom = root.querySelector("#browser-search-url");
  if (custom) custom.onchange = () => {
    save("gui-browser-search-url", custom.value.trim());
    rebuild();
  };
  for (const key of ["rec-keys", "rec-mouse"]) {
    const box = root.querySelector(`#browser-${key}`);
    if (box) box.onchange = () => save(`gui-browser-${key}`, String(box.checked));
  }
}

/** Web addresses and file paths in a terminal, as T3's terminal-links.ts finds them, across the rows a
 * long line wraps over. */
const TERM_URL = /https?:\/\/[^\s"'`<>]+/giu;
const TERM_PATH = /(?:~[\\/]|\.{1,2}[\\/]|\/|[A-Za-z]:[\\/]|\\\\)[^\s"'`<>]+|(?<![\p{L}\p{M}\p{N}._-])[\p{L}\p{M}\p{N}._-]+(?:[\\/][\p{L}\p{M}\p{N}._-]+)+(?::\d+){0,2}/gu;
const TERM_QUOTED_PATH = /(["'`])(?<path>(?:~[\\/]|\.{1,2}[\\/]|\/|[A-Za-z]:[\\/]|\\\\|[\p{L}\p{M}\p{N}._ -]+[\\/])[^\r\n]*?|[\p{L}\p{M}\p{N}._ -]+\.[\p{L}\p{M}\p{N}_-]+(?::\d+){0,2})\1(?<position>(?::\d+){0,2})/gu;
const TERM_ROUTE = /(?:^|[\s"'`])(?:GET|HEAD|POST|PUT|PATCH|DELETE|OPTIONS|CONNECT|TRACE|proxying)\s+["'`]?(?:\/|[A-Za-z0-9_-]+\/)[^\s?]*\?\S*/g;
function trimLink(value, kind) {
  let out = value.replace(kind === "path" ? /[.,;:!?]+$/ : /[.,;!?]+$/, "");
  for (const [open, close] of [["(", ")"], ["[", "]"], ["{", "}"]]) {
    while (out.endsWith(close) && out.split(open).length - 1 < out.split(close).length - 1) out = out.slice(0, -1);
  }
  return out;
}
function terminalLinks(line) {
  const overlaps = (a, b) => a.start < b.end && b.start < a.end;
  const found = [];
  const collect = (kind, pattern) => {
    for (const m of line.matchAll(pattern)) {
      const quoted = m.groups?.path;
      const raw = quoted === undefined ? m[0] : quoted + (m.groups?.position ?? "");
      const start = m.index + (quoted === undefined ? 0 : 1);
      const text = quoted === undefined ? trimLink(raw, kind) : raw;
      if (!text || (kind === "path" && /^https?:\/\//i.test(text))) continue;
      const link = { kind, text, start, end: start + text.length + (m.groups?.position ? 1 : 0) };
      if (!found.some((other) => overlaps(link, other))) found.push(link);
    }
  };
  collect("url", TERM_URL);
  collect("path", TERM_QUOTED_PATH);
  collect("path", TERM_PATH);
  const routes = [...line.matchAll(TERM_ROUTE)].map((m) => ({ start: m.index, end: m.index + m[0].length }));
  return found.filter((l) => l.kind === "url" || !routes.some((r) => overlaps(l, r))).sort((a, b) => a.start - b.start);
}

/** Opens a path a terminal printed, `src/app.ts:12:4` or an absolute one in the thread's folder, in a
 * File tab at that line, as the chat's file links do. */
function openTerminalPath(text) {
  const tab = active;
  if (!tab) return;
  const [, file, line] = text.match(/^(.+?)(?::(\d+))?(?::\d+)?$/) ?? [];
  const home = /^~[\\/]/.test(file) ? "" : file;
  const cwd = tab.view.cwd.replace(/\/+$/, "");
  const path = (home.startsWith(`${cwd}/`) ? home.slice(cwd.length + 1) : home).replace(/^\.\//, "");
  if (!path || path.startsWith("/") || path.startsWith("../")) return flash(`${file} is outside this thread's folder`);
  openFileSurface(path, { line: Number(line) || null });
}

function wireTerminalLinks(term) {
  if (term.pinnedLinks) return;
  term.pinnedLinks = true;
  term.registerLinkProvider({
    provideLinks(y, callback) {
      const buffer = term.buffer.active;
      let first = y;
      while (first > 1 && buffer.getLine(first - 1)?.isWrapped) first--;
      const rows = [];
      for (let at = first; buffer.getLine(at - 1); at++) {
        const wrapsOn = buffer.getLine(at)?.isWrapped === true;
        const text = buffer.getLine(at - 1).translateToString(!wrapsOn);
        rows.push({ y: at, text, from: rows.length ? rows.at(-1).to : 0, to: (rows.length ? rows.at(-1).to : 0) + text.length });
        if (!wrapsOn) break;
      }
      const cell = (offset) => {
        const row = rows.find((r) => offset < r.to) ?? rows.at(-1);
        return { x: offset - row.from + 1, y: row.y };
      };
      const links = terminalLinks(rows.map((r) => r.text).join("")).filter((l) => l.kind === "url" || guiOn())
        .map((l) => ({ ...l, range: { start: cell(l.start), end: cell(l.end - 1) } }))
        .filter((l) => l.range.start.y <= y && l.range.end.y >= y)
        .map((l) => ({
          text: l.text,
          range: l.range,
          activate: (e) => (l.kind === "url" ? (guiOn() ? openLink(l.text, e.metaKey || e.ctrlKey) : openOutside(l.text)) : guiOn() && openTerminalPath(l.text)),
        }));
      callback(links.length ? links : undefined);
    },
  });
}

/** Why a page didn't load, in the words T3's PreviewUnreachable uses for Chromium's error names. */
const PAGE_ERRORS = {
  ERR_NAME_NOT_RESOLVED: "DNS address could not be found",
  ERR_NAME_RESOLUTION_FAILED: "DNS address could not be found",
  ERR_CONNECTION_REFUSED: "Connection refused",
  ERR_CONNECTION_RESET: "Connection was reset",
  ERR_CONNECTION_CLOSED: "Connection was closed",
  ERR_CONNECTION_TIMED_OUT: "Connection timed out",
  ERR_INTERNET_DISCONNECTED: "No internet connection",
  ERR_TIMED_OUT: "Connection timed out",
  ERR_CERT_AUTHORITY_INVALID: "Certificate authority is not trusted",
  ERR_CERT_COMMON_NAME_INVALID: "Certificate hostname mismatch",
  ERR_CERT_DATE_INVALID: "Certificate is expired or not yet valid",
  ERR_TOO_MANY_REDIRECTS: "Too many redirects",
};
const PAGE_ERROR_ICON = `<svg class="error-icon" viewBox="0 0 64 64" fill="none" stroke="currentColor" stroke-width="2.5" aria-hidden="true"><path d="M16 12 L48 12 L48 52 L16 52 Z"/><path d="M22 22 L42 22 M22 30 L36 30 M22 38 L40 38" stroke-linecap="round"/><path d="M52 8 L12 56" stroke-linecap="round"/></svg>`;

/** `s.failure`: the page that couldn't be reached, `{ kind: "unreachable", url, description, code }`,
 * or a file it answered instead of a page, `{ kind: "file", url, name, path }`. Either stands in for
 * the page until the tab goes somewhere. */
function failureHtml(s) {
  const f = s.failure;
  const host = hostOf(f.url) || f.url;
  const action = (what, label, primary = false) => `<button type="button" class="${primary ? "primary" : "outline"}" data-failure="${what}">${label}</button>`;
  if (f.kind === "file") {
    return `<div class="page-failure"><div class="inner">${PAGE_ERROR_ICON}<h1>This file can’t be shown here</h1>
      <p><b>${esc(f.name)}</b> from ${esc(host)} was downloaded instead.</p>
      <div class="actions"><span class="spacer"></span>${action("open", "Open in browser")}${f.path ? action("reveal", IS_MAC ? "Show in Finder" : "Show in folder", true) : `<a class="button primary" href="${esc(f.url)}" download="${esc(f.name)}">Download</a>`}</div></div></div>`;
  }
  const friendly = PAGE_ERRORS[f.description] ?? (f.description || "Network error");
  return `<div class="page-failure"><div class="inner">${PAGE_ERROR_ICON}<h1>This site can’t be reached</h1>
    <p><b>${esc(host)}</b>: ${esc(friendly.replace(/\.$/, ""))}.</p>
    ${f.details ? `<div class="details"><p>Try:</p><ul><li>Checking your connection</li><li>Confirming the dev server is running</li><li>Checking the proxy and the firewall</li></ul></div>` : ""}
    <p class="code">${esc(f.description || `ERR_${Math.abs(f.code) || "FAILED"}`)}</p>
    <div class="actions">${action("details", f.details ? "Hide details" : "Details")}<span class="spacer"></span>${action("reload", "Reload", true)}</div></div></div>`;
}

function showFailure(s, failure) {
  s.failure = failure;
  s.loading = false;
  const tab = browserTab(s);
  if (tab) savePanel(tab);
  if (tab === active && shownSurface(tab) === s) renderBrowser(s);
}

/** Leaves the failure screen for the page itself, before the tab goes somewhere. */
function clearFailure(s) {
  if (!s.failure) return false;
  s.failure = null;
  metaFetched.delete(s.id);
  return true;
}

panelBody.addEventListener("click", (e) => {
  const button = e.target.closest(".page-failure [data-failure]");
  const s = active && shownSurface(active);
  if (!button || !isPage(s) || !s.failure) return;
  const f = s.failure;
  if (button.dataset.failure === "details") {
    f.details = !f.details;
    renderBrowser(s, true);
    panelBody.querySelector(".page-failure").outerHTML = failureHtml(s);
  } else if (button.dataset.failure === "reload") go(s, f.url || s.url);
  else if (button.dataset.failure === "open") openOutside(f.url);
  else if (button.dataset.failure === "reveal") invoke("reveal_file", { path: f.path }).catch((err) => flash(String(err)));
});

/** What a page's machine found at its address: the web build learns of a failure from it, since a
 * frame can't tell. */
function pageMetaFailure(s, url, meta) {
  if (meta?.failure) showFailure(s, { kind: "unreachable", url, description: meta.failure, code: 0 });
  else if (meta?.file) showFailure(s, { kind: "file", url, name: meta.file });
}

if (NATIVE_PAGES) {
  TAURI.event.listen("browser", ({ payload: e }) => {
    const s = (e.kind === "failed" || e.kind === "download") && surfaceById(e.id);
    if (!s) return;
    if (e.kind === "failed") return showFailure(s, { kind: "unreachable", url: e.url || s.url, description: e.description, code: e.code });
    const name = e.path.split(/[\\/]/).pop() || hostOf(e.url);
    if (!e.success) return flash(`Couldn't download ${name}`);
    showFailure(s, { kind: "file", url: e.url, name, path: e.path });
  });
}

/** T3's ZoomIndicator: the new zoom fades in at the page's top right for a moment and out again. A
 * native page draws over the panel, so the app draws its pill inside the page, sized against the zoom. */
let zoomPillTimer;
function showZoom(s) {
  const percent = `${Math.round((s.zoom ?? 1) * 100)}%`;
  if (NATIVE_PAGES) {
    if (!openPages.has(s.id)) return;
    const look = getComputedStyle(panelMenu);
    const style = `margin:12px;padding:4px 10px;border:1px solid ${look.borderTopColor};border-radius:999px;background:${look.backgroundColor};color:${look.color};font:500 12px/16px -apple-system,system-ui,sans-serif;box-shadow:0 4px 12px rgb(${getComputedStyle(document.documentElement).getPropertyValue("--g-shadow-rgb").trim()} / .1);transition:opacity .2s ease-out,transform .2s ease-out;opacity:0;transform:translateY(-4px)`;
    const js = `(() => {
      let host = document.getElementById("__pinned_zoom");
      if (!host) { host = document.createElement("div"); host.id = "__pinned_zoom"; host.attachShadow({ mode: "open" }).append(document.createElement("div")); document.documentElement.append(host); }
      host.style.cssText = "all:initial;position:fixed;top:0;right:0;z-index:2147483647;pointer-events:none;zoom:${1 / (s.zoom ?? 1)}";
      const pill = host.shadowRoot.firstChild;
      pill.textContent = ${JSON.stringify(percent)};
      pill.style.cssText = ${JSON.stringify(style)};
      requestAnimationFrame(() => requestAnimationFrame(() => { pill.style.opacity = "1"; pill.style.transform = "none"; }));
      clearTimeout(window.__pinnedZoomTimer);
      window.__pinnedZoomTimer = setTimeout(() => { pill.style.opacity = "0"; pill.style.transform = "translateY(-4px)"; }, 1500);
      return true;
    })()`;
    return invoke("browser_eval", { id: s.id, js }).catch(() => {});
  }
  const viewport = panelBody.querySelector(".browser .viewport");
  if (!viewport) return;
  const pill = viewport.querySelector(".zoom-pill") ?? viewport.appendChild(Object.assign(document.createElement("div"), { className: "zoom-pill" }));
  pill.textContent = percent;
  pill.setAttribute("aria-hidden", "false");
  requestAnimationFrame(() => pill.classList.add("on"));
  clearTimeout(zoomPillTimer);
  zoomPillTimer = setTimeout(() => {
    pill.classList.remove("on");
    pill.setAttribute("aria-hidden", "true");
  }, 1500);
}

/** The floating preview, T3's mini player: the thread's page over the chat while the panel shows
 * something else, fitted to a 320px square at the page's own shape, dragged by its corner handle and
 * resized from any edge with that shape kept. A native page draws over the HTML under it, so in the
 * app its controls sit in a strip above the page. */
const MINI_GAP = 12;
const MINI_SIZE = 320;
const MINI_MIN = { w: 240, h: 150 };
const MINI_STRIP = NATIVE_PAGES ? 32 : 0;
const MINI_GRIPS = ["n", "s", "e", "w", "ne", "nw", "se", "sw"];
const miniSource = (s) => (s.device ? { w: s.device.w * (s.zoom ?? 1), h: s.device.h * (s.zoom ?? 1) } : s.viewSize ?? { w: 1280, h: 800 });

function miniLayout(tab, s, at = panelOf(tab).pipAt ?? {}) {
  const chat = document.getElementById("chat").getBoundingClientRect();
  const source = miniSource(s);
  const aspect = source.w / source.h;
  const max = Math.max(1, Math.min(chat.width - MINI_GAP * 2, (chat.height - MINI_GAP * 2 - MINI_STRIP) * aspect));
  const min = Math.min(max, Math.max(MINI_MIN.w, MINI_MIN.h * aspect));
  const width = Math.min(max, Math.max(min, at.w ?? (aspect >= 1 ? MINI_SIZE : MINI_SIZE * aspect)));
  const w = Math.round(width);
  const h = Math.round(width / aspect) + MINI_STRIP;
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(Math.max(lo, hi), v));
  return {
    w, h, aspect, source,
    x: Math.round(clamp(at.x ?? chat.width - w - MINI_GAP, MINI_GAP, chat.width - w - MINI_GAP)),
    y: Math.round(clamp(at.y ?? chat.height - h - 140, MINI_GAP, chat.height - h - MINI_GAP)),
  };
}

function togglePip(s) {
  const p = panelOf(active);
  p.pip = p.pip === s.id ? null : s.id;
  savePanel(active);
  renderPanel();
}

const pipFrame = Object.assign(document.createElement("div"), { id: "preview-pip", hidden: true });
pipFrame.setAttribute("role", "region");
pipFrame.setAttribute("aria-label", "Floating browser preview");
document.getElementById("chat").append(pipFrame);

function placePip(tab, s, layout = miniLayout(tab, s)) {
  Object.assign(pipFrame.style, { left: `${layout.x}px`, top: `${layout.y}px`, width: `${layout.w}px`, height: `${layout.h}px` });
  const frame = pipFrame.querySelector("iframe");
  const zoom = s.zoom ?? 1;
  const scale = layout.w / layout.source.w;
  if (frame) frame.style.cssText = `width:${layout.source.w / zoom}px;height:${layout.source.h / zoom}px;transform:scale(${zoom * scale})`;
  pipFrame.dataset.scale = String(scale);
}

function renderPip(tab) {
  const p = panelOf(tab);
  const s = guiOn() && p.pip && p.surfaces.find((x) => x.id === p.pip);
  pipFrame.hidden = !s;
  if (!s) {
    pipFrame.dataset.id = "";
    pipFrame.innerHTML = "";
    return;
  }
  const own = s.window ? "Close separate window" : "Pop into separate window";
  const controls = `<button type="button" class="icon" data-pip="panel" title="Open in right panel" aria-label="Open in right panel">${icon("panel-right")}</button>
    ${NATIVE_PAGES ? `<button type="button" class="icon${s.window ? " on" : ""}" data-pip="window" title="${own}" aria-label="${own}">${icon("picture-in-picture-2")}</button>` : ""}
    <button type="button" class="icon" data-pip="close" title="Close floating preview" aria-label="Close floating preview">${icon("x")}</button>`;
  const recordingNow = recording?.id === s.id;
  if (pipFrame.dataset.id !== s.id) {
    pipFrame.dataset.id = s.id;
    pipFrame.innerHTML = `<div class="page">${NATIVE_PAGES ? "" : `<iframe src="${esc(s.url)}" title="Floating preview" sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-modals allow-downloads"></iframe>`}</div>
      ${NATIVE_PAGES ? `<div class="strip"><span class="title"></span></div>` : ""}
      <div class="pill" role="toolbar" aria-label="Floating preview">${controls}</div>
      <button type="button" class="handle" data-pip-drag aria-label="Move floating preview"><span class="dot"></span></button>
      ${MINI_GRIPS.map((dir) => `<span class="grip ${dir}" data-pip-resize="${dir}" aria-hidden="true"></span>`).join("")}`;
  } else pipFrame.querySelector(".pill").innerHTML = controls;
  pipFrame.classList.toggle("recording", recordingNow);
  const title = pipFrame.querySelector(".strip .title");
  if (title) title.textContent = surfaceTitle(tab, s);
  placePip(tab, s);
}
addEventListener("resize", () => active && renderPip(active));

pipFrame.addEventListener("click", (e) => {
  const b = e.target.closest("[data-pip]");
  if (!b || !active) return;
  const p = panelOf(active);
  const s = p.surfaces.find((x) => x.id === p.pip);
  if (b.dataset.pip === "window") return s && invoke("browser_window", { id: s.id, url: s.url }).catch((err) => flash(String(err)));
  p.pip = null;
  if (b.dataset.pip === "panel") {
    p.active = s?.id ?? p.active;
    p.open = true;
  }
  savePanel(active);
  renderPanel();
});

/** Moves the player by its handle once the pointer goes 6px, or resizes it from a grip. */
pipFrame.addEventListener("pointerdown", (down) => {
  const target = down.target.closest("[data-pip-drag], [data-pip-resize]");
  const tab = active;
  const p = tab && panelOf(tab);
  const s = p?.surfaces.find((x) => x.id === p.pip);
  if (!target || !s || down.button !== 0) return;
  down.preventDefault();
  target.setPointerCapture(down.pointerId);
  const start = miniLayout(tab, s);
  const dir = target.dataset.pipResize;
  let moved = false;
  const move = (e) => {
    const dx = e.clientX - down.clientX;
    const dy = e.clientY - down.clientY;
    if (!moved && !dir && Math.hypot(dx, dy) < 6) return;
    moved = true;
    let at;
    if (!dir) at = { ...p.pipAt, w: start.w, x: start.x + dx, y: start.y + dy };
    else {
      const sx = dir.includes("w") ? -dx : dir.includes("e") ? dx : 0;
      const sy = dir.includes("n") ? -dy : dir.includes("s") ? dy : 0;
      const grow = sx && sy ? ((sx * start.aspect + sy) / (start.aspect ** 2 + 1)) * start.aspect : sx || sy * start.aspect;
      const sized = miniLayout(tab, s, { w: start.w + grow, x: start.x, y: start.y });
      at = { w: sized.w, x: dir.includes("w") ? start.x + start.w - sized.w : start.x, y: dir.includes("n") ? start.y + start.h - sized.h : start.y };
    }
    p.pipAt = at;
    placePip(tab, s);
    placePages(true);
  };
  const end = () => {
    target.removeEventListener("pointermove", move);
    target.removeEventListener("lostpointercapture", end);
    if (moved) savePanel(tab);
  };
  target.addEventListener("pointermove", move);
  target.addEventListener("lostpointercapture", end);
});

/** T3's annotation editor (PickPreload.ts), run inside the page: Select, Region, Draw and Erase pick
 * targets; a card beside them takes a comment and, expanded, style changes the page shows live. The
 * pick waits in `__pinnedPick` for the panel, which captures the page and then calls `done()`.
 * It is serialized into the page, so it reads nothing from this window. */
function pinnedAnnotator(theme) {
  if (window.__pinnedPicker) return true;
  const SVG = "http://www.w3.org/2000/svg";
  const host = document.createElement("div");
  host.style.cssText = "all:initial;position:fixed;inset:0;z-index:2147483646";
  const root = host.attachShadow({ mode: "open" });
  root.innerHTML = `<style>
    :host { --p: ${theme.primary}; --on-p: ${theme.onPrimary}; --pop: ${theme.popover}; --fg: ${theme.fg}; --muted: ${theme.muted}; --border: ${theme.border}; --accent: ${theme.accent}; --input: ${theme.input}; }
    * { box-sizing: border-box; font-family: ${theme.font}; }
    .layer { position: fixed; inset: 0; cursor: crosshair; }
    .ink, .marks { position: fixed; inset: 0; pointer-events: none; overflow: visible; }
    .ink { width: 100vw; height: 100vh; }
    .box { position: fixed; border: 2px solid var(--p); background: color-mix(in srgb, var(--p) 10%, transparent); border-radius: 3px; pointer-events: none; }
    .box.region { background: color-mix(in srgb, var(--p) 6%, transparent); }
    .box.picked { background: none; }
    .chip { position: absolute; left: -2px; top: -24px; max-width: 280px; padding: 2px 6px; border-radius: 4px; background: var(--p); color: var(--on-p); font: 600 12px/18px inherit; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .bar { position: fixed; top: 10px; left: 50%; transform: translateX(-50%); display: flex; gap: 2px; padding: 4px; border: 1px solid var(--border); border-radius: 8px; background: color-mix(in srgb, var(--pop) 95%, transparent); backdrop-filter: blur(8px); box-shadow: 0 10px 15px -3px rgb(${theme.shadow} / .1); cursor: default; }
    button { font: 500 14px/20px inherit; color: var(--fg); background: none; border: 0; border-radius: 6px; cursor: pointer; }
    .bar button { height: 32px; padding: 0 12px; }
    .bar button:hover { background: var(--accent); }
    .bar button.on { background: color-mix(in srgb, var(--p) 10%, transparent); color: var(--p); }
    .card { position: fixed; width: min(360px, calc(100vw - 16px)); max-height: calc(100vh - 16px); display: flex; flex-direction: column; border: 1px solid var(--border); border-radius: 12px; background: color-mix(in srgb, var(--pop) 96%, transparent); backdrop-filter: blur(8px); box-shadow: 0 25px 50px -12px rgb(${theme.shadow} / .25); color: var(--fg); cursor: default; overflow: hidden; }
    .card[hidden], [hidden] { display: none !important; }
    .row { display: flex; align-items: flex-start; gap: 8px; padding: 8px; }
    .adjust { flex: none; width: 32px; height: 32px; display: grid; place-items: center; background: var(--accent); color: var(--muted); }
    .adjust svg { width: 15px; height: 15px; }
    textarea { flex: 1; min-height: 32px; max-height: 96px; resize: none; padding: 6px 2px; border: 0; border-bottom: 1px solid transparent; background: transparent; color: var(--fg); font: 14px/20px inherit; outline: none; }
    textarea:focus { border-bottom-color: var(--p); }
    textarea::placeholder { color: var(--muted); }
    .grab { flex: none; width: 24px; height: 32px; color: var(--muted); cursor: grab; }
    .attach { flex: none; height: 32px; padding: 0 12px; background: var(--p); color: var(--on-p); }
    .attach:disabled { opacity: .45; cursor: default; }
    .styles { max-height: min(176px, calc(100vh - 180px)); overflow-y: auto; padding: 0 12px; border-top: 1px solid var(--border); background: color-mix(in srgb, var(--accent) 40%, transparent); font: 12px/16px inherit; }
    .styles section { padding: 8px 0; border-top: 1px solid var(--border); }
    .styles section:first-child { border-top: 0; }
    .styles h4 { margin: 0 0 6px; font: 600 11px/16px inherit; color: var(--muted); text-transform: uppercase; letter-spacing: .03em; }
    .styles label { display: grid; grid-template-columns: 82px 1fr; align-items: center; gap: 8px; margin: 4px 0; color: var(--muted); }
    .styles .field { display: flex; align-items: center; gap: 4px; min-width: 0; }
    .styles input, .styles select { width: 100%; min-width: 0; height: 28px; padding: 0 6px; border: 1px solid var(--border); border-radius: 6px; background: var(--input); color: var(--fg); font: 12px ui-monospace, monospace; }
    .styles input[type=color] { flex: none; width: 20px; height: 20px; padding: 0; border-radius: 4px; }
    .styles input[type=range] { padding: 0; }
    .styles .unit { color: var(--muted); }
    .styles .lock { flex: none; width: 18px; height: 38px; padding: 0; color: var(--muted); }
    .styles .lock.on { color: var(--p); }
  </style>
  <div class="layer"></div><svg class="ink"></svg><div class="marks"></div><div class="box hover" hidden></div>
  <div class="bar" role="toolbar"><button data-tool="select" title="Select elements (V)">Select</button><button data-tool="region" title="Draw a region or marquee-select elements (R)">Region</button><button data-tool="draw" title="Draw freehand (D)">Draw</button><button data-tool="erase" title="Remove an annotation target (E)">Erase</button></div>
  <div class="card" hidden><div class="row">
    <button class="adjust" title="Expand annotation editor" aria-label="Expand annotation editor"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="21" x2="14" y1="4" y2="4"/><line x1="10" x2="3" y1="4" y2="4"/><line x1="21" x2="12" y1="12" y2="12"/><line x1="8" x2="3" y1="12" y2="12"/><line x1="21" x2="16" y1="20" y2="20"/><line x1="12" x2="3" y1="20" y2="20"/><line x1="14" x2="14" y1="2" y2="6"/><line x1="8" x2="8" y1="10" y2="14"/><line x1="16" x2="16" y1="18" y2="22"/></svg></button>
    <textarea rows="1" placeholder="Describe the change…"></textarea>
    <button class="grab" title="Drag annotation editor" hidden>⠿</button>
    <button class="attach" title="Attach annotation and screenshot (Enter). Send with Cmd/Ctrl+Enter.">Attach</button>
  </div><div class="styles" hidden></div></div>`;
  document.documentElement.append(host);
  const $ = (q) => root.querySelector(q);
  const layer = $(".layer"), ink = $(".ink"), marks = $(".marks"), hover = $(".hover"), card = $(".card"), note = $("textarea"), styles = $(".styles");
  const seed = Math.random().toString(36).slice(2, 8);
  let seq = 0;
  const nextId = (prefix) => `${prefix}_${seed}_${(++seq).toString(36)}`;
  const state = { tool: "select", elements: [], regions: [], strokes: [], changes: new Map(), originals: new Map(), expanded: false, pinned: null, focused: false };
  const page = (x, y) => ({ x: x + scrollX, y: y + scrollY });
  const under = (x, y) => {
    host.style.pointerEvents = "none";
    const el = document.elementFromPoint(x, y);
    host.style.pointerEvents = "";
    return el && el !== document.documentElement && el !== document.body ? el : null;
  };
  const selectorOf = (el) => {
    const parts = [];
    for (let e = el; e && e.nodeType === 1 && e !== document.body && e !== document.documentElement && parts.length < 5; e = e.parentElement) {
      let part = e.localName;
      if (e.id) { parts.unshift(`${part}#${e.id}`); break; }
      const cls = [...e.classList].slice(0, 2).join(".");
      if (cls) part += `.${cls}`;
      parts.unshift(part);
    }
    return parts.join(" > ");
  };
  const chipText = (el) => `${el.localName}${el.id ? `#${el.id}` : ""}${[...el.classList].slice(0, 2).map((c) => `.${c}`).join("")}`;
  const place = (node, r) => Object.assign(node.style, { left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px` });
  const smooth = (points) => {
    if (points.length < 2) return "";
    let d = `M${points[0][0] - scrollX} ${points[0][1] - scrollY}`;
    for (let i = 1; i < points.length - 1; i++) {
      const [x, y] = points[i], [nx, ny] = points[i + 1];
      d += ` Q${x - scrollX} ${y - scrollY} ${(x + nx) / 2 - scrollX} ${(y + ny) / 2 - scrollY}`;
    }
    const [lx, ly] = points.at(-1);
    return `${d} L${lx - scrollX} ${ly - scrollY}`;
  };
  const targets = () => state.elements.length + state.regions.length + state.strokes.length;
  const union = () => {
    const rects = [...state.elements.map((t) => t.el.getBoundingClientRect()), ...state.regions.map((r) => ({ left: r.x - scrollX, top: r.y - scrollY, right: r.x - scrollX + r.w, bottom: r.y - scrollY + r.h })),
      ...state.strokes.map((s) => ({ left: s.bounds.x - scrollX, top: s.bounds.y - scrollY, right: s.bounds.x - scrollX + s.bounds.w, bottom: s.bounds.y - scrollY + s.bounds.h }))];
    if (!rects.length) return null;
    const left = Math.min(...rects.map((r) => r.left)), top = Math.min(...rects.map((r) => r.top));
    const right = Math.max(...rects.map((r) => r.right)), bottom = Math.max(...rects.map((r) => r.bottom));
    return { left, top, right, bottom, width: right - left, height: bottom - top };
  };

  function draw() {
    marks.replaceChildren(...state.elements.map((t) => {
      const box = document.createElement("div");
      box.className = "box picked";
      place(box, t.el.getBoundingClientRect());
      const chip = document.createElement("span");
      chip.className = "chip";
      chip.textContent = chipText(t.el);
      box.append(chip);
      return box;
    }), ...state.regions.map((r) => {
      const box = document.createElement("div");
      box.className = "box region";
      place(box, { left: r.x - scrollX, top: r.y - scrollY, width: r.w, height: r.h });
      return box;
    }));
    ink.replaceChildren(...state.strokes.map((s) => {
      const path = document.createElementNS(SVG, "path");
      path.setAttribute("d", smooth(s.points));
      Object.entries({ fill: "none", stroke: theme.primary, "stroke-width": 4, "stroke-linecap": "round", "stroke-linejoin": "round" }).forEach(([k, v]) => path.setAttribute(k, v));
      return path;
    }));
    for (const b of root.querySelectorAll(".bar button")) b.classList.toggle("on", b.dataset.tool === state.tool);
    const had = !card.hidden;
    card.hidden = !targets();
    $(".attach").disabled = !targets();
    if (!state.elements.length && state.expanded) setExpanded(false);
    $(".adjust").disabled = !state.elements.length;
    if (!card.hidden) placeCard();
    if (!card.hidden && !had) note.focus();
  }

  function placeCard() {
    if (state.pinned) return Object.assign(card.style, { left: `${state.pinned.x}px`, top: `${state.pinned.y}px` });
    const u = union();
    const w = card.offsetWidth, h = card.offsetHeight, gap = 8;
    const spots = [[u.right + gap, u.top], [u.left - gap - w, u.top], [u.right - w, u.bottom + gap], [u.right - w, u.top - gap - h]];
    const overflow = ([x, y]) => Math.max(0, 8 - x) + Math.max(0, x + w - innerWidth + 8) + Math.max(0, 8 - y) + Math.max(0, y + h - innerHeight + 8);
    const [x, y] = spots.reduce((best, spot) => (overflow(spot) < overflow(best) ? spot : best));
    card.style.left = `${Math.max(8, Math.min(innerWidth - w - 8, x))}px`;
    card.style.top = `${Math.max(8, Math.min(innerHeight - h - 8, y))}px`;
  }

  const STYLE_GROUPS = [
    ["Text", [["Font", "font-family", "select", ["inherit", "system-ui", "sans-serif", "serif", "monospace"]], ["Font size", "font-size", "px", [1, 300]], ["Font weight", "font-weight", "select", ["300", "400", "500", "600", "700", "800", "900"]], ["Line height", "line-height", "text", "normal / 1.4"]]],
    ["Colors", [["Text color", "color", "color"], ["Background", "background-color", "color"], ["Opacity", "opacity", "range"]]],
    ["Borders", [["Radius", "border-radius", "px", [0, 300]], ["Border color", "border-color", "color"], ["Border width", "border-width", "px", [0, 100]]]],
    ["Sizing", [["Width", "width", "px", [0, 10000]], ["Height", "height", "px", [0, 10000]], ["Padding", "padding", "text", ""], ["Margin", "margin", "text", ""], ["Gap", "gap", "text", ""]]],
  ];
  const hex = (color) => {
    const m = /rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(color);
    return m ? `#${m.slice(1, 4).map((n) => Number(n).toString(16).padStart(2, "0")).join("")}` : "#000000";
  };
  function renderStyles() {
    const first = state.elements[0]?.el;
    if (!first) return;
    const computed = getComputedStyle(first);
    const value = (prop) => computed.getPropertyValue(prop).trim();
    styles.innerHTML = STYLE_GROUPS.map(([title, rows]) => `<section><h4>${title}</h4>${rows.map(([label, prop, kind, extra]) => {
      const v = value(prop);
      const control = kind === "select" ? `<select data-prop="${prop}">${extra.map((o) => `<option${o === v || (prop === "font-family" && o === "inherit") ? " selected" : ""}>${o}</option>`).join("")}</select>`
        : kind === "px" ? `<input type="number" data-prop="${prop}" data-unit="px" min="${extra[0]}" max="${extra[1]}" value="${parseFloat(v) || 0}"><span class="unit">px</span>${prop === "height" ? `<button class="lock on" data-lock title="Keep aspect ratio" aria-pressed="true"><svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M9 17H7A5 5 0 0 1 7 7h2"/><path d="M15 7h2a5 5 0 1 1 0 10h-2"/><line x1="8" x2="16" y1="12" y2="12"/></svg></button>` : ""}`
        : kind === "color" ? `<input type="color" data-prop="${prop}" data-swatch value="${hex(v)}"><input type="text" data-prop="${prop}" value="${v}">`
        : kind === "range" ? `<input type="range" data-prop="${prop}" min="0" max="1" step="0.05" value="${v || 1}">`
        : `<input type="text" data-prop="${prop}" placeholder="${extra}" value="${v}">`;
      return `<label><span>${label}</span><span class="field">${control}</span></label>`;
    }).join("")}</section>`).join("");
  }
  function setStyle(prop, value) {
    for (const t of state.elements) {
      if (!state.originals.has(t.el)) state.originals.set(t.el, t.el.getAttribute("style"));
      const key = `${t.id}|${prop}`;
      if (!state.changes.has(key)) state.changes.set(key, { targetId: t.id, selector: t.selector, property: prop, previousValue: getComputedStyle(t.el).getPropertyValue(prop).trim() });
      state.changes.get(key).value = value;
      t.el.style.setProperty(prop, value, "important");
      if (prop === "border-width") t.el.style.setProperty("border-style", "solid", "important");
    }
    draw();
  }
  function setExpanded(on) {
    state.expanded = on && !!state.elements.length;
    styles.hidden = !state.expanded;
    $(".grab").hidden = !state.expanded;
    const label = state.expanded ? "Collapse annotation editor" : "Expand annotation editor";
    $(".adjust").title = label;
    $(".adjust").setAttribute("aria-label", label);
    if (state.expanded) {
      const r = card.getBoundingClientRect();
      state.pinned = { x: r.left, y: r.top };
      renderStyles();
    } else state.pinned = null;
    placeCard();
  }
  styles.addEventListener("input", (e) => {
    const input = e.target.closest("[data-prop]");
    if (!input) return;
    const prop = input.dataset.prop;
    const value = input.dataset.unit ? `${input.value}${input.dataset.unit}` : input.value;
    if (input.dataset.swatch) root.querySelector(`input[type=text][data-prop="${prop}"]`).value = value;
    setStyle(prop, value);
    const lock = root.querySelector("[data-lock]");
    const first = state.elements[0]?.el;
    if (lock?.classList.contains("on") && first && (prop === "width" || prop === "height")) {
      const r = first.getBoundingClientRect();
      const other = prop === "width" ? "height" : "width";
      const ratio = state.ratio ?? (state.ratio = r.width / r.height);
      const next = Math.round(prop === "width" ? Number(input.value) / ratio : Number(input.value) * ratio);
      root.querySelector(`input[data-prop="${other}"]`).value = next;
      setStyle(other, `${next}px`);
    }
  });
  styles.addEventListener("click", (e) => {
    const lock = e.target.closest("[data-lock]");
    if (!lock) return;
    lock.classList.toggle("on");
    lock.setAttribute("aria-pressed", String(lock.classList.contains("on")));
    state.ratio = null;
  });

  let down = null;
  layer.addEventListener("pointermove", (e) => {
    if (down?.stroke) {
      down.stroke.points.push(Object.values(page(e.clientX, e.clientY)));
      return draw();
    }
    if (down && state.tool === "region") {
      const r = { left: Math.min(down.x, e.clientX), top: Math.min(down.y, e.clientY), width: Math.abs(e.clientX - down.x), height: Math.abs(e.clientY - down.y) };
      hover.hidden = false;
      hover.className = "box region";
      return place(hover, r);
    }
    if (state.tool === "select" && !down) {
      const el = under(e.clientX, e.clientY);
      hover.hidden = !el;
      hover.className = "box";
      if (el) place(hover, el.getBoundingClientRect());
    }
  });
  layer.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    down = { x: e.clientX, y: e.clientY };
    if (state.tool === "draw") {
      down.stroke = { id: nextId("stroke"), points: [Object.values(page(e.clientX, e.clientY))] };
      state.strokes.push(down.stroke);
    }
    try {
      layer.setPointerCapture(e.pointerId);
    } catch {}
  });
  layer.addEventListener("pointerup", (e) => {
    const d = down;
    down = null;
    hover.hidden = true;
    if (!d) return;
    if (state.tool === "draw") {
      const s = d.stroke;
      if (s.points.length < 2) state.strokes.pop();
      else {
        const xs = s.points.map((p) => p[0]), ys = s.points.map((p) => p[1]), pad = 7;
        s.bounds = { x: Math.min(...xs) - pad, y: Math.min(...ys) - pad, w: Math.max(...xs) - Math.min(...xs) + pad * 2, h: Math.max(...ys) - Math.min(...ys) + pad * 2 };
      }
    } else if (state.tool === "region") {
      const r = { left: Math.min(d.x, e.clientX), top: Math.min(d.y, e.clientY), right: Math.max(d.x, e.clientX), bottom: Math.max(d.y, e.clientY) };
      if (r.right - r.left >= 3 && r.bottom - r.top >= 3) {
        const inside = [...document.body.querySelectorAll("*")].filter((el) => {
          if (!(el.children.length === 0 || el.matches("button, a, [role=button]"))) return false;
          const b = el.getBoundingClientRect();
          const cx = b.left + b.width / 2, cy = b.top + b.height / 2;
          return b.width > 0 && b.height > 0 && cx >= r.left && cx <= r.right && cy >= r.top && cy <= r.bottom;
        }).sort((a, b) => a.getBoundingClientRect().width * a.getBoundingClientRect().height - b.getBoundingClientRect().width * b.getBoundingClientRect().height).slice(0, 20);
        for (const el of inside) if (!state.elements.some((t) => t.el === el)) state.elements.push({ id: nextId("element"), el, selector: selectorOf(el) });
        if (!inside.length) {
          const at = page(r.left, r.top);
          state.regions.push({ id: nextId("region"), x: at.x, y: at.y, w: r.right - r.left, h: r.bottom - r.top });
        }
      }
    } else if (state.tool === "erase") {
      const { x, y } = page(e.clientX, e.clientY);
      const inRect = (r) => e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
      const el = [...state.elements].reverse().find((t) => inRect(t.el.getBoundingClientRect()));
      const region = !el && [...state.regions].reverse().find((r) => x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h);
      const stroke = !el && !region && [...state.strokes].reverse().find((s) => x >= s.bounds.x && x <= s.bounds.x + s.bounds.w && y >= s.bounds.y && y <= s.bounds.y + s.bounds.h);
      state.elements = state.elements.filter((t) => t !== el);
      state.regions = state.regions.filter((r) => r !== region);
      state.strokes = state.strokes.filter((s) => s !== stroke);
    } else if (Math.hypot(e.clientX - d.x, e.clientY - d.y) < 6) {
      const el = under(e.clientX, e.clientY);
      if (el) {
        const had = state.elements.find((t) => t.el === el);
        if (had) state.elements = state.elements.filter((t) => t !== had);
        else state.elements = [...(e.shiftKey ? state.elements : []), { id: nextId("element"), el, selector: selectorOf(el) }];
      }
    }
    draw();
  });
  root.querySelector(".bar").addEventListener("click", (e) => {
    const b = e.target.closest("[data-tool]");
    if (!b) return;
    state.tool = b.dataset.tool;
    draw();
  });
  $(".adjust").addEventListener("click", () => setExpanded(!state.expanded));
  $(".grab").addEventListener("pointerdown", (e) => {
    const grab = e.currentTarget;
    grab.setPointerCapture(e.pointerId);
    const start = { ...state.pinned }, from = { x: e.clientX, y: e.clientY };
    const move = (m) => {
      state.pinned = { x: Math.max(8, Math.min(innerWidth - card.offsetWidth - 8, start.x + m.clientX - from.x)), y: Math.max(8, Math.min(innerHeight - card.offsetHeight - 8, start.y + m.clientY - from.y)) };
      placeCard();
    };
    grab.addEventListener("pointermove", move);
    grab.addEventListener("lostpointercapture", () => grab.removeEventListener("pointermove", move), { once: true });
  });
  note.addEventListener("input", () => {
    note.style.height = "auto";
    note.style.height = `${Math.min(96, note.scrollHeight)}px`;
    placeCard();
  });
  note.addEventListener("keydown", (e) => {
    if (e.key !== "Enter" || e.shiftKey || e.isComposing) return;
    e.preventDefault();
    if (e.metaKey || e.ctrlKey) submit("send");
    else if (!e.altKey) submit("attach");
  });
  $(".attach").addEventListener("click", () => submit("attach"));
  const reflow = () => draw();
  addEventListener("scroll", reflow, true);
  addEventListener("resize", reflow);
  const key = (e) => {
    const typing = e.composedPath().some((n) => n instanceof HTMLElement && n.matches?.("textarea, input, select"));
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      return finish({ cancelled: true });
    }
    const tool = { v: "select", r: "region", d: "draw", e: "erase" }[e.key.toLowerCase()];
    if (tool && !typing && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      state.tool = tool;
      draw();
    }
  };
  addEventListener("keydown", key, true);
  function teardown() {
    for (const [el, style] of state.originals) style === null ? el.removeAttribute("style") : el.setAttribute("style", style);
    removeEventListener("keydown", key, true);
    removeEventListener("scroll", reflow, true);
    removeEventListener("resize", reflow);
    host.remove();
    window.__pinnedPicker = null;
  }
  function finish(pick) {
    if (pick.cancelled) teardown();
    window.__pinnedPick = pick;
  }
  function submit(how) {
    if (!targets()) return;
    const u = union();
    const crop = { left: Math.max(0, u.left - 20), top: Math.max(0, u.top - 20) };
    Object.assign(crop, { width: Math.min(innerWidth, u.right + 20) - crop.left, height: Math.min(innerHeight, u.bottom + 20) - crop.top });
    $(".attach").textContent = "Capturing…";
    $(".bar").hidden = true;
    card.hidden = true;
    const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
    const summary = [state.elements.length && plural(state.elements.length, "selected element", "selected elements"), state.regions.length && plural(state.regions.length, "marked region", "marked regions"), state.strokes.length && plural(state.strokes.length, "drawing", "drawings")].filter(Boolean).join(", ");
    finish({
      submit: how, id: nextId("annotation"), pageUrl: location.href, pageTitle: document.title, comment: note.value.trim(), summary, crop, width: innerWidth,
      elements: state.elements.map((t) => ({ id: t.id, tag: t.el.localName, selector: t.selector, html: t.el.outerHTML.slice(0, 4000) })),
      changes: [...state.changes.values()].filter((c) => c.value !== c.previousValue),
    });
  }
  window.__pinnedPicker = { cancel: () => finish({ cancelled: true }), done: teardown, root };
  window.__pinnedPick = null;
  draw();
  return true;
}

/** The theme the annotation editor draws with, read from this window's tokens. */
function annotatorTheme() {
  const css = getComputedStyle(document.documentElement);
  const token = (name) => css.getPropertyValue(name).trim();
  return { primary: token("--g-primary"), onPrimary: token("--g-on-primary"), popover: token("--g-overlay"), fg: token("--g-fg"), muted: token("--g-muted"), border: token("--g-border"), accent: token("--g-accent"), input: token("--g-input"), shadow: token("--g-shadow-rgb"), font: getComputedStyle(document.body).fontFamily };
}

/** What an annotation adds to the prompt, as T3 sends one to an agent without its context records: a
 * label where the caret is, and the annotation's block after the message. */
function annotationText(pick) {
  const label = (pick.comment.replace(/\s+/g, " ").trim() || pick.pageTitle || "Preview annotation");
  const short = label.length > 48 ? `${label.slice(0, 47)}…` : label;
  const lines = [`Id: ${pick.id}`, `Page: ${pick.pageUrl || pick.pageTitle}`];
  if (pick.comment) lines.push(`Comment: ${pick.comment}`);
  if (pick.summary) lines.push(`Targets: ${pick.summary}`);
  if (pick.changes.length) lines.push("Requested visual changes:", ...pick.changes.map((c) => `- ${c.property}: ${c.previousValue || "(unset)"} → ${c.value}`));
  if (pick.elements.length) {
    const indent = (text) => text.split("\n").map((l) => (l ? `  ${l}` : "")).join("\n");
    lines.push("<element_context>", pick.elements.map((el) => `- <${el.tag}>:\n${indent([`url: ${pick.pageUrl}`, `selector: ${el.selector}`, `html:\n${indent(el.html)}`].join("\n"))}`).join("\n"), "</element_context>");
  }
  return { label: `[Preview annotation: ${short}]`, block: `<preview_annotation>\n${lines.join("\n")}\n</preview_annotation>` };
}

async function pickElement(s) {
  if (s.picking) {
    s.picking = false;
    invoke("browser_eval", { id: s.id, js: "(() => { window.__pinnedPicker?.cancel(); return true; })()" }).catch(() => {});
    return syncChrome(s);
  }
  if (!openPages.has(s.id)) return;
  s.picking = true;
  syncChrome(s);
  await invoke("browser_eval", { id: s.id, js: `(${pinnedAnnotator})(${JSON.stringify(annotatorTheme())})` }).catch((e) => flash(String(e)));
  let pick = null;
  while (s.picking && !pick) {
    await new Promise((r) => setTimeout(r, 250));
    pick = await invoke("browser_eval", { id: s.id, js: "(() => { const p = window.__pinnedPick; window.__pinnedPick = null; return p ?? null; })()" }).catch(() => null);
  }
  s.picking = false;
  syncChrome(s);
  if (!pick || pick.cancelled) return;
  const tab = browserTab(s);
  try {
    const shot = await createImageBitmap(await snapshotOf(s));
    const ratio = shot.width / pick.width;
    const r = pick.crop;
    const canvas = Object.assign(document.createElement("canvas"), { width: Math.max(1, Math.round(r.width * ratio)), height: Math.max(1, Math.round(r.height * ratio)) });
    canvas.getContext("2d").drawImage(shot, r.left * ratio, r.top * ratio, r.width * ratio, r.height * ratio, 0, 0, canvas.width, canvas.height);
    const blob = await new Promise((done) => canvas.toBlob(done, "image/png"));
    await attachFiles(tab, [new File([blob], `preview-annotation-${pick.id}.png`, { type: "image/png" })]);
  } catch {
    flash("Could not capture the picked element. The annotation was kept without the screenshot.");
  }
  invoke("browser_eval", { id: s.id, js: "(() => { window.__pinnedPicker?.done(); return true; })()" }).catch(() => {});
  const { label, block } = annotationText(pick);
  const at = prompt.selectionStart ?? prompt.value.length;
  const head = prompt.value.slice(0, at);
  const tail = prompt.value.slice(at);
  const spaced = `${head && !/\s$/.test(head) ? " " : ""}${label}${tail && !/^\s/.test(tail) ? " " : ""}`;
  prompt.value = `${`${head}${spaced}${tail}`.trimEnd()}\n\n${block}`;
  prompt.dispatchEvent(new Event("input"));
  if (pick.submit === "send") send();
}

/** What the page tells a recording: the pointer, presses and released buttons, and key presses labelled
 * as T3's RecordingInput.ts labels them. Password fields keep their keys. */
const RECORD_INPUT_JS = `(() => {
  if (window.__pinnedRec) return innerWidth;
  const rec = window.__pinnedRec = { events: [] };
  const mac = /Mac|iPhone|iPad/.test(navigator.platform);
  const SPECIAL = { Enter: "↵", Tab: "⇥", Backspace: "⌫", Delete: "⌦", Escape: "Esc", ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→", " ": "Space" };
  const label = (e) => {
    if (["Dead", "Process", "Unidentified", "Shift", "Control", "Alt", "Meta"].includes(e.key)) return "";
    const key = SPECIAL[e.key] ?? (e.key.length === 1 ? e.key.toUpperCase() : e.key);
    if (mac) return (e.ctrlKey ? "⌃" : "") + (e.altKey ? "⌥" : "") + (e.shiftKey ? "⇧" : "") + (e.metaKey ? "⌘" : "") + key;
    return [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Win", key].filter(Boolean).join(" + ");
  };
  const hidden = () => { const a = document.activeElement; return a && (a.type === "password" || a.localName === "iframe" || a.localName.includes("-")); };
  let frame = 0;
  const on = {
    pointermove: (e) => { if (e.pointerType === "touch" || frame) return; frame = requestAnimationFrame(() => (frame = 0)); rec.events.push({ t: "move", x: e.clientX, y: e.clientY }); },
    pointerdown: (e) => e.pointerType !== "touch" && rec.events.push({ t: "down", x: e.clientX, y: e.clientY }),
    pointerup: (e) => e.pointerType !== "touch" && rec.events.push({ t: "up", x: e.clientX, y: e.clientY }),
    keydown: (e) => { if (e.repeat || hidden()) return; const l = label(e); if (l) rec.events.push({ t: "key", label: l }); },
    keyup: () => rec.events.push({ t: "keyup" }),
    blur: () => rec.events.push({ t: "clear" }),
    pointerleave: () => rec.events.push({ t: "clear" }),
  };
  for (const [name, fn] of Object.entries(on)) addEventListener(name, fn, true);
  rec.stop = () => { for (const [name, fn] of Object.entries(on)) removeEventListener(name, fn, true); window.__pinnedRec = null; };
  return innerWidth;
})()`;

/** The decorations a recording draws over each frame, from what the page told it. */
function recordingOverlay(ctx, input, scale, now) {
  const css = getComputedStyle(document.documentElement);
  const token = (name) => css.getPropertyValue(name).trim();
  const { pointer, press, key } = input;
  if (press && recordsMouse()) {
    const fade = press.upAt ? Math.min(1, (now - press.upAt) / 600) : 0;
    if (fade < 1) {
      ctx.save();
      ctx.globalAlpha = 0.9 * (1 - fade);
      ctx.strokeStyle = ctx.fillStyle = token("--g-primary");
      ctx.lineWidth = 2 * scale;
      ctx.beginPath();
      ctx.arc(press.x * scale, press.y * scale, 20 * scale * (1 + fade * 0.5), 0, Math.PI * 2);
      ctx.stroke();
      ctx.globalAlpha *= 0.15;
      ctx.fill();
      ctx.restore();
    }
  }
  if (pointer) {
    ctx.save();
    ctx.translate(pointer.x * scale, pointer.y * scale);
    ctx.scale(scale, scale);
    ctx.beginPath();
    ctx.moveTo(0, 0);
    ctx.lineTo(0, 17);
    ctx.lineTo(4.5, 13);
    ctx.lineTo(7.5, 20);
    ctx.lineTo(10.5, 18.8);
    ctx.lineTo(7.6, 12);
    ctx.lineTo(13.5, 12);
    ctx.closePath();
    ctx.fillStyle = token("--g-picker-black");
    ctx.strokeStyle = token("--g-picker-white");
    ctx.lineWidth = 1.5;
    ctx.stroke();
    ctx.fill();
    ctx.restore();
  }
  if (key && recordsKeys() && (!key.upAt || now - key.upAt < 900)) {
    ctx.save();
    ctx.font = `500 ${26 * scale}px system-ui, -apple-system, sans-serif`;
    const width = Math.min(ctx.canvas.width - 32 * scale, ctx.measureText(key.label).width + 36 * scale);
    const height = 54 * scale;
    const x = (ctx.canvas.width - width) / 2;
    const y = ctx.canvas.height - 24 * scale - height;
    ctx.globalAlpha = 0.86;
    ctx.fillStyle = token("--g-picker-black");
    ctx.beginPath();
    ctx.roundRect(x, y, width, height, 14 * scale);
    ctx.fill();
    ctx.globalAlpha = 1;
    ctx.fillStyle = token("--g-picker-white");
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(key.label, ctx.canvas.width / 2, y + height / 2, width - 18 * scale);
    ctx.restore();
  }
}

let recording = null;
async function capture(s, record) {
  if (recording) return stopRecording();
  if (!openPages.has(s.id)) return;
  if (!record) {
    try {
      await saveCapture(`Screenshot ${stamp()}.png`, await snapshotOf(s), "Screenshot");
    } catch (e) {
      flash(`Couldn't capture the page: ${e}`);
    }
    return;
  }
  const canvas = document.createElement("canvas");
  const first = await createImageBitmap(await snapshotOf(s)).catch(() => null);
  if (!first) return flash("Couldn't capture the page");
  Object.assign(canvas, { width: first.width, height: first.height });
  const ctx = canvas.getContext("2d");
  ctx.drawImage(first, 0, 0);
  const type = ["video/webm", "video/mp4"].find((t) => window.MediaRecorder?.isTypeSupported(t));
  if (!type) return flash("This system can't record video");
  const pageWidth = await invoke("browser_eval", { id: s.id, js: RECORD_INPUT_JS }).catch(() => null);
  const scale = canvas.width / (Number(pageWidth) || canvas.width);
  const chunks = [];
  const recorder = new MediaRecorder(canvas.captureStream(10), { mimeType: type });
  recorder.ondataavailable = (e) => e.data.size && chunks.push(e.data);
  const input = { pointer: null, press: null, key: null };
  recording = { id: s.id, recorder, input, done: new Promise((r) => (recorder.onstop = r)) };
  recorder.start(1000);
  syncChrome(s);
  if (active) renderPip(active);
  (async () => {
    while (recording?.recorder === recorder) {
      const [frame, events] = await Promise.all([
        snapshotOf(s).then(createImageBitmap).catch(() => null),
        invoke("browser_eval", { id: s.id, js: "(() => window.__pinnedRec ? window.__pinnedRec.events.splice(0) : [])()" }).catch(() => []),
      ]);
      const now = performance.now();
      for (const e of Array.isArray(events) ? events : []) {
        if (e.t === "move" || e.t === "down" || e.t === "up") input.pointer = { x: e.x, y: e.y };
        if (e.t === "move" && input.press && !input.press.upAt) Object.assign(input.press, { x: e.x, y: e.y });
        if (e.t === "down") input.press = { x: e.x, y: e.y, upAt: 0 };
        if (e.t === "up" && input.press) input.press.upAt = now;
        if (e.t === "key") input.key = { label: e.label, upAt: 0 };
        if (e.t === "keyup" && input.key && !input.key.upAt) input.key.upAt = now;
        if (e.t === "clear") Object.assign(input, { pointer: null, press: null, key: null });
      }
      if (frame && recording?.recorder === recorder) {
        ctx.drawImage(frame, 0, 0, canvas.width, canvas.height);
        recordingOverlay(ctx, input, scale, now);
      }
      await new Promise((r) => setTimeout(r, 80));
    }
  })();
  recording.save = async () => saveCapture(`Recording ${stamp()}.${type.split("/")[1]}`, new Blob(chunks, { type }), "Recording");
}
async function stopRecording() {
  const r = recording;
  recording = null;
  r.recorder.stop();
  invoke("browser_eval", { id: r.id, js: "(() => { window.__pinnedRec?.stop(); return true; })()" }).catch(() => {});
  await r.done;
  const s = surfaceById(r.id);
  if (s && shownSurface(active) === s) syncChrome(s);
  if (active) renderPip(active);
  r.save().catch((e) => flash(String(e)));
}
