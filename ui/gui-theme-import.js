/** T3 Code's Add theme dialog (ThemeImportDialog.tsx, ThemeSearchSection.tsx): community themes from
 * Open VSX, VS Code theme files turned into palettes (vscodeThemeImport.ts), a JSON paste box, and
 * dropped or chosen files. An Open VSX extension installs as one collection of themes. */
Object.assign(ICON_PATHS, {
  "package-plus": `<path d="M16 16h6"/><path d="M19 13v6"/><path d="M21 10V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l2-1.14"/><path d="m7.5 4.27 9 5.15"/><polyline points="3.29 7 12 12 20.71 7"/><line x1="12" x2="12" y1="22" y2="12"/>`,
});

const MAX_THEME_FILE_BYTES = 256 * 1024;
const MAX_HIGHLIGHTED_JSON = 20000;
const byteSize = (n) => (n >= 1024 * 1024 ? `${(n / (1024 * 1024)).toFixed(1)} MB` : n >= 1024 ? `${Math.round(n / 1024)} KB` : `${n} bytes`);
const oversizedTheme = (n) => (n > MAX_THEME_FILE_BYTES ? `That file is ${byteSize(n)}. Theme files are only a few KB, so this one was not read (limit ${byteSize(MAX_THEME_FILE_BYTES)}).` : null);
const isRecord = (v) => typeof v === "object" && v !== null && !Array.isArray(v);

/* vscodeThemeImport.ts */

function vsColor(value) {
  if (typeof value !== "string") return null;
  const v = value.trim();
  const fn = /^color\(\s*(display-p3|srgb)\s+([^)]+)\)$/i.exec(v);
  if (fn) {
    const [part, alphaPart] = fn[2].split("/");
    const ch = part.trim().split(/\s+/).map((p) => (p.endsWith("%") ? parseFloat(p) / 100 : parseFloat(p)));
    const a = alphaPart === undefined ? 1 : alphaPart.trim().endsWith("%") ? parseFloat(alphaPart) / 100 : parseFloat(alphaPart);
    if (ch.length !== 3 || !ch.every(Number.isFinite) || !Number.isFinite(a)) return null;
    const alpha = Math.max(0, Math.min(1, a));
    if (fn[1].toLowerCase() === "srgb") return { r: ch[0] * 255, g: ch[1] * 255, b: ch[2] * 255, a: alpha };
    const dec = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
    const enc = (c) => ((c = Math.max(0, Math.min(1, c))) <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055) * 255;
    const [r, g, b] = ch.map(dec);
    return { r: enc(1.2249401762805 * r - 0.2249401762805 * g), g: enc(-0.042056961239 * r + 1.042056961239 * g), b: enc(-0.0196375547643 * r - 0.0786360655012 * g + 1.0982736202656 * b), a: alpha };
  }
  const hex = v.replace(/^#/, "");
  if (!/^(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(hex)) return null;
  const full = hex.length <= 4 ? [...hex].map((c) => c + c).join("") : hex;
  const n = (i) => parseInt(full.slice(i, i + 2), 16);
  return { r: n(0), g: n(2), b: n(4), a: full.length === 8 ? n(6) / 255 : 1 };
}
/** VS Code's overlays are translucent; a palette's roles are opaque, so they land on their surface. */
const flattenOver = (c, base) => rgbHex(c.a >= 1 ? c : mixRgb(base, c, c.a));
const hexRgb = (value) => vsColor(colorHex(value) ?? value) ?? { r: 0, g: 0, b: 0, a: 1 };
const standsApart = (a, b) => contrast(hexRgb(a), hexRgb(b)) >= 1.1;
/** T3's readableThemeColorOn: the color, moved lighter or darker until it reads on the surface. */
function readableColorOn(color, surface, min) {
  const c = parseColor(color);
  const s = parseColor(surface);
  if (!c || !s) return color;
  const against = oklchToRgb(s.color);
  if (contrast(oklchToRgb(c.color), against) >= min) return color;
  return rgbHex(oklchToRgb(solveLightness(c.color, against, min, isDarkRgb(against))));
}

function isVsCodeThemeFile(value) {
  if (!isRecord(value) || value.version === 1) return false;
  return (isRecord(value.colors) && Object.keys(value.colors).some((k) => k.includes("."))) || Array.isArray(value.tokenColors);
}
function humanizeThemeName(raw) {
  const t = raw.trim();
  if (/\s/.test(t) || !/[-_.]/.test(t)) return t;
  return t.split(/[-_.]+/).filter(Boolean).map((w) => w[0].toUpperCase() + w.slice(1)).join(" ");
}

function parseVsCodeThemeFile(value) {
  if (!isRecord(value)) throw new Error("Theme files must contain a JSON object.");
  const colors = isRecord(value.colors) ? value.colors : {};
  const pick = (...keys) => keys.map((k) => vsColor(colors[k])).find(Boolean) ?? null;
  const solidOver = (base, ...keys) => {
    const c = pick(...keys);
    return c ? flattenOver(c, base) : null;
  };
  const canvasColor = pick("editor.background", "editorPane.background");
  if (!canvasColor) throw new Error('That VS Code theme has no "editor.background" color, so there is nothing to build a palette from.');
  const canvas = { r: canvasColor.r, g: canvasColor.g, b: canvasColor.b };
  const type = typeof value.type === "string" ? value.type.toLowerCase() : null;
  const appearance = type === "light" || type === "hc-light" ? "light" : type === "dark" || type === "hc-black" ? "dark" : isDarkRgb(canvas) ? "dark" : "light";
  const canvasHex = rgbHex(canvas);
  const raisedCandidate = solidOver(canvas, "editorWidget.background", "dropdown.background");
  const apartFromSurfaces = (hex) => standsApart(hex, canvasHex) && (raisedCandidate === null || standsApart(hex, raisedCandidate));

  let accentColor = null;
  let accentHex = null;
  for (const key of ["focusBorder", "button.background", "textLink.foreground", "activityBarBadge.background", "progressBar.background", "badge.background"]) {
    const c = vsColor(colors[key]);
    if (!c || !apartFromSurfaces(flattenOver(c, canvas))) continue;
    [accentColor, accentHex] = [c, flattenOver(c, canvas)];
    break;
  }
  if (!accentColor) {
    const standardAccent = colorHex(standardColors(appearance).accent);
    accentHex = [standardAccent, "#ffffff", "#000000"].find(apartFromSurfaces) ?? standardAccent;
    accentColor = vsColor(accentHex);
  }

  const derived = Object.fromEntries(Object.entries(vividThemeColors(chatColors(appearance), canvasHex, flattenOver({ ...accentColor, a: 0.2 }, canvas), appearance)).map(([k, v]) => [k, colorHex(v) ?? v]));
  const sidebarHex = solidOver(canvas, "sideBar.background", "activityBar.background") ?? derived.sidebar;
  const sidebar = hexRgb(sidebarHex);
  const terminalHex = solidOver(canvas, "terminal.background", "panel.background") ?? derived.terminalBackground;
  const terminal = hexRgb(terminalHex);
  const readableOn = (surface, fallback, ...keys) => {
    const s = hexRgb(surface);
    const reads = (c) => contrast(hexRgb(c), s) >= 4.5;
    const given = solidOver(s, ...keys);
    if (given && reads(given)) return given;
    if (reads(fallback)) return fallback;
    return isDarkRgb(s) ? "#ffffff" : "#000000";
  };
  // A status color also fills buttons with white text, so a dim one is lightened along its hue.
  const readableStatus = (fallback, ...keys) => {
    const given = solidOver(canvas, ...keys);
    return readableColorOn(given && standsApart(given, canvasHex) ? given : fallback, canvasHex, 4.5);
  };
  const raisedHex = raisedCandidate ?? derived.surfaceRaised;
  const buttonHex = solidOver(canvas, "button.background");
  const actionHex = buttonHex && standsApart(buttonHex, canvasHex) && standsApart(buttonHex, raisedHex) ? buttonHex : accentHex;
  const inputOk = (c) => c && standsApart(c, canvasHex) && standsApart(c, actionHex);
  let inputHex = [derived.input, derived.surfaceRaised, colorHex(standardColors(appearance).input), "#000000", "#ffffff", "#808080"].find(inputOk) ?? "#808080";
  inputHex = ["input.background", "input.border"].map((k) => solidOver(canvas, k)).find(inputOk) ?? inputHex;
  const matchBg = solidOver(canvas, "editor.findMatchHighlightBackground") ?? derived.searchMatchBackground;
  const matchActiveBg = solidOver(canvas, "editor.findMatchBackground") ?? derived.searchMatchActiveBackground;

  const overrides = {
    canvas: canvasHex,
    text: readableOn(canvasHex, derived.text, "editor.foreground", "foreground"),
    textMuted: readableOn(canvasHex, derived.textMuted, "descriptionForeground", "disabledForeground"),
    surface: solidOver(canvas, "editorWidget.background") ?? derived.surface,
    surfaceRaised: raisedHex,
    surfaceOverlay: solidOver(canvas, "menu.background", "quickInput.background", "dropdown.background") ?? derived.surfaceOverlay,
    border: solidOver(canvas, "panel.border", "editorGroup.border", "contrastBorder") ?? derived.border,
    input: inputHex,
    placeholder: readableOn(raisedHex, derived.placeholder, "input.placeholderForeground"),
    error: readableStatus(derived.error, "editorError.foreground", "errorForeground"),
    warning: readableStatus(derived.warning, "editorWarning.foreground"),
    accentSurface: solidOver(canvas, "list.activeSelectionBackground", "list.hoverBackground") ?? derived.accentSurface,
    codeBackground: solidOver(canvas, "textCodeBlock.background") ?? derived.codeBackground,
    searchMatchBackground: matchBg,
    searchMatchForeground: readableOn(matchBg, derived.searchMatchForeground, "editor.findMatchHighlightForeground", "editor.foreground", "foreground"),
    searchMatchActiveBackground: matchActiveBg,
    searchMatchActiveForeground: readableOn(matchActiveBg, derived.searchMatchActiveForeground, "editor.findMatchForeground", "editor.foreground", "foreground"),
    sidebar: sidebarHex,
    sidebarForeground: readableOn(sidebarHex, derived.sidebarForeground, "sideBar.foreground"),
    sidebarBorder: solidOver(sidebar, "sideBar.border") ?? derived.sidebarBorder,
    sidebarRowHover: solidOver(sidebar, "list.hoverBackground") ?? derived.sidebarRowHover,
    sidebarRowActive: solidOver(sidebar, "list.inactiveSelectionBackground", "list.hoverBackground") ?? derived.sidebarRowActive,
    sidebarRowSelected: solidOver(sidebar, "list.activeSelectionBackground") ?? derived.sidebarRowSelected,
    terminalBackground: terminalHex,
    terminalForeground: readableOn(terminalHex, derived.terminalForeground, "terminal.foreground"),
    terminalCursor: solidOver(terminal, "terminalCursor.foreground", "editorCursor.foreground") ?? derived.terminalCursor,
    terminalSelection: solidOver(terminal, "terminal.selectionBackground", "editor.selectionBackground") ?? derived.terminalSelection,
    terminalScrollbar: solidOver(terminal, "scrollbarSlider.background") ?? derived.terminalScrollbar,
    accent: accentHex,
    focus: accentHex,
    messageAction: actionHex,
    messageActionForeground: readableOn(actionHex, derived.messageActionForeground, "button.foreground"),
    accentForeground: readableOn(accentHex, derived.accentForeground, "button.foreground"),
  };
  const name = [value.displayName, value.name].filter((n) => typeof n === "string").map(humanizeThemeName).find(Boolean)?.slice(0, 48) ?? "VS Code theme";
  const id = themeIdFromName(name);
  return parseThemeFile({ version: 1, ...(RESERVED_THEME_IDS.has(id) ? { id: `${id}-vscode` } : {}), name, appearance, colors: { ...derived, ...overrides } });
}

/** A light and a dark theme that differ only by "Light"/"Dark" in their names become one theme. */
function pairVsCodeThemes(themes, pairedId) {
  const strip = (label) => label.replace(/\b(?:light|dark)\b/gi, " ").replace(/\s+/g, " ").trim();
  const groups = new Map();
  const out = [];
  themes.forEach((theme, order) => {
    const key = strip(theme.label);
    if (modesOf(theme).length !== 1 || key === theme.label || !/[a-z0-9]/i.test(key)) return out.push({ theme, order });
    const group = groups.get(key) ?? { light: [], dark: [], order };
    group[theme.appearance].push(theme);
    groups.set(key, group);
  });
  for (const [key, g] of groups) {
    if (g.light.length === 1 && g.dark.length === 1) {
      try {
        out.push({ order: g.order, theme: parseThemeFile({ version: 1, ...(pairedId ? { id: pairedId(g.light[0], g.dark[0]) } : {}), name: key, appearance: "light", colors: g.light[0].colors, variants: { dark: g.dark[0].colors } }) });
        continue;
      } catch {}
    }
    for (const theme of [...g.light, ...g.dark]) out.push({ theme, order: g.order });
  }
  return out.sort((a, b) => a.order - b.order).map((e) => e.theme);
}

/** Some extensions reuse one name across files (Dracula and Dracula Soft both say "Dracula"). */
function resolveThemeLabelCollisions(entries) {
  const rename = (theme, name) => {
    try {
      return parseThemeFile({ version: 1, name: name.slice(0, 48), appearance: theme.appearance, colors: theme.colors, ...(theme.variants ? { variants: theme.variants } : {}), ...(theme.managed ? { managed: true } : {}) });
    } catch {
      return null;
    }
  };
  const counts = new Map();
  for (const { theme } of entries) counts.set(theme.id, (counts.get(theme.id) ?? 0) + 1);
  const relabelled = entries.map(({ theme, sourceName }) => {
    if (counts.get(theme.id) < 2) return theme;
    const fromFile = sourceName ? humanizeThemeName(sourceName.replace(/\.[^.]+$/, "")) : null;
    return (fromFile && fromFile.toLowerCase() !== theme.label.toLowerCase() && rename(theme, fromFile)) || theme;
  });
  const seen = new Set();
  return relabelled.map((theme) => {
    if (!seen.has(theme.id)) return seen.add(theme.id), theme;
    for (let n = 2; n < 100; n++) {
      const c = rename(theme, `${theme.label} ${n}`);
      if (c && !seen.has(c.id)) return seen.add(c.id), c;
    }
    return theme;
  });
}
const parseAnyThemeFile = (value) => (isVsCodeThemeFile(value) ? parseVsCodeThemeFile(value) : parseThemeFile(value));

/* openVsxThemes.ts */

const OPEN_VSX = "https://open-vsx.org";
const VSX_LIMITS = { vsix: 20 * 1024 * 1024, search: 512 * 1024, detail: 256 * 1024, manifest: 256 * 1024, theme: 256 * 1024, entries: 5000, unpacked: 100 * 1024 * 1024, ratio: 200, themes: 40, depth: 8, path: 1024, color: 128, timeout: 10000 };
const VSX_LICENSES = new Set(["0BSD", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "CC0-1.0", "ISC", "MIT", "MPL-2.0", "Unlicense"]);
const USED_WORKBENCH_COLORS = new Set(["activityBar.background", "activityBarBadge.background", "badge.background", "button.background", "button.foreground", "contrastBorder", "descriptionForeground", "disabledForeground", "dropdown.background", "dropdown.border", "editor.background", "editor.foreground", "editor.findMatchBackground", "editor.findMatchForeground", "editor.findMatchHighlightBackground", "editor.findMatchHighlightForeground", "editor.selectionBackground", "editorCursor.foreground", "editorError.foreground", "editorGroup.border", "editorPane.background", "editorWarning.foreground", "editorWidget.background", "errorForeground", "focusBorder", "foreground", "input.border", "input.placeholderForeground", "list.activeSelectionBackground", "list.hoverBackground", "list.inactiveSelectionBackground", "menu.background", "panel.background", "panel.border", "progressBar.background", "quickInput.background", "scrollbarSlider.background", "sideBar.background", "sideBar.border", "sideBar.foreground", "terminal.background", "terminal.foreground", "terminal.selectionBackground", "terminalCursor.foreground", "textCodeBlock.background", "textLink.foreground"]);

async function sha256Hex(bytes) {
  if (!crypto.subtle) throw new Error("Adding themes from Open VSX needs a secure (https) connection.");
  return [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))].map((b) => b.toString(16).padStart(2, "0")).join("");
}
const shortHash = async (text) => (await sha256Hex(new TextEncoder().encode(text))).slice(0, 12);
const openVsxThemeId = async (extensionId, source) => `ovx-theme-${await shortHash(`${extensionId}:${source}`)}`;

function trustedOpenVsxUrl(value) {
  try {
    const url = new URL(value);
    return url.protocol === "https:" && url.hostname.toLowerCase() === "open-vsx.org" ? url.toString() : null;
  } catch {
    return null;
  }
}
function publicSourceUrl(value) {
  const raw = typeof value === "string" ? value : isRecord(value) && typeof value.url === "string" ? value.url : null;
  try {
    const url = new URL(raw);
    return url.protocol === "https:" && !url.username && !url.password ? url.toString() : null;
  } catch {
    return null;
  }
}
const themeContributions = (manifest) => (Array.isArray(manifest.contributes?.themes) ? manifest.contributes.themes.filter(isRecord) : []);
const licenseMatches = (manifest, license) => typeof manifest.license === "string" && manifest.license.trim().toLowerCase() === license.toLowerCase();

async function extensionFromDetail(value) {
  if (!isRecord(value) || !isRecord(value.files)) throw new Error("Open VSX returned malformed theme details.");
  const str = (v) => (typeof v === "string" ? v.trim() : "");
  const [namespace, name, version, license] = [str(value.namespace), str(value.name), str(value.version), str(value.license)];
  const [manifestUrl, sha256Url, vsixUrl] = [value.files.manifest, value.files.sha256, value.files.download].map(trustedOpenVsxUrl);
  if (!namespace || !name || !version || !manifestUrl || !sha256Url || !vsixUrl) throw new Error("Open VSX returned malformed theme details.");
  if (!VSX_LICENSES.has(license)) return null;
  const id = `${namespace}.${name}`;
  const collectionId = /^[a-z0-9][a-z0-9.:-]{0,127}$/.test(`open-vsx:${id.toLowerCase()}`) ? `open-vsx:${id.toLowerCase()}` : `open-vsx:${await shortHash(id)}`;
  return {
    id, collectionId, name: str(value.displayName) || name, publisher: namespace,
    description: typeof value.description === "string" ? value.description : "",
    downloadCount: Number.isFinite(value.downloadCount) ? value.downloadCount : 0,
    iconUrl: trustedOpenVsxUrl(value.files.icon),
    sourceUrl: publicSourceUrl(value.repository) ?? publicSourceUrl(value.homepage) ?? publicSourceUrl(value.url),
    manifestUrl, sha256Url, vsixUrl, version, license,
  };
}

async function withTimeout(run, parent) {
  const controller = new AbortController();
  const abort = () => controller.abort();
  if (parent?.aborted) abort();
  else parent?.addEventListener("abort", abort, { once: true });
  const timer = setTimeout(abort, VSX_LIMITS.timeout);
  try {
    return await run(controller.signal);
  } catch (err) {
    if (controller.signal.aborted && !parent?.aborted) throw new Error("Open VSX took too long to respond.");
    throw err;
  } finally {
    clearTimeout(timer);
    parent?.removeEventListener("abort", abort);
  }
}
async function readCapped(response, limit, tooLarge) {
  if (Number(response.headers.get("content-length")) > limit) throw new Error(tooLarge);
  if (!response.body) return new Uint8Array(0);
  const reader = response.body.getReader();
  const chunks = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > limit) {
      await reader.cancel();
      throw new Error(tooLarge);
    }
    chunks.push(value);
  }
  return concatBytes(chunks, size);
}
function concatBytes(chunks, size) {
  const out = new Uint8Array(size);
  let at = 0;
  for (const c of chunks) out.set(c, at), (at += c.byteLength);
  return out;
}
const decodeText = (bytes) => new TextDecoder().decode(bytes);

/** JSON with comments and trailing commas, as VS Code reads theme and manifest files. */
function parseJsonc(source, what) {
  let out = "";
  for (let i = 0; i < source.length; i++) {
    const c = source[i];
    if (c === '"') {
      let j = i + 1;
      while (j < source.length && source[j] !== '"') j += source[j] === "\\" ? 2 : 1;
      out += source.slice(i, j + 1);
      i = j;
    } else if (c === "/" && source[i + 1] === "/") {
      while (i < source.length && source[i] !== "\n") i++;
      out += "\n";
    } else if (c === "/" && source[i + 1] === "*") {
      const end = source.indexOf("*/", i + 2);
      i = end < 0 ? source.length : end + 1;
    } else out += c;
  }
  try {
    const value = JSON.parse(out.replace(/,(\s*[}\]])/g, "$1"));
    if (isRecord(value)) return value;
  } catch {}
  throw new Error(`${what} is not valid JSON.`);
}

async function searchOpenVsxThemes(query, { signal, sortBy = "downloadCount" } = {}) {
  const text = query.trim();
  if (!text) return [];
  const url = new URL(`${OPEN_VSX}/api/-/search`);
  for (const [k, v] of Object.entries({ query: text, category: "Themes", sortBy, sortOrder: "desc", size: "16" })) url.searchParams.set(k, v);
  const value = await withTimeout(async (s) => {
    const response = await fetch(url, { signal: s });
    if (!response.ok) throw new Error("Open VSX search is unavailable right now.");
    const bytes = await readCapped(response, VSX_LIMITS.search, "Open VSX returned an unexpectedly large response.");
    try {
      return JSON.parse(decodeText(bytes));
    } catch {
      throw new Error("Open VSX returned an unreadable response.");
    }
  }, signal);
  if (!isRecord(value) || !Array.isArray(value.extensions)) throw new Error("Open VSX returned an unreadable search response.");
  const identities = value.extensions.filter((e) => isRecord(e) && typeof e.namespace === "string" && e.namespace && typeof e.name === "string" && e.name).map((e) => [e.namespace, e.name]);
  const details = await Promise.allSettled(identities.slice(0, 16).map(([namespace, name]) => withTimeout(async (s) => {
    const response = await fetch(`${OPEN_VSX}/api/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}`, { signal: s });
    if (!response.ok) throw new Error("Open VSX theme details are unavailable.");
    const bytes = await readCapped(response, VSX_LIMITS.detail, "Open VSX returned an unexpectedly large detail response.");
    try {
      const extension = await extensionFromDetail(JSON.parse(decodeText(bytes)));
      if (!extension) return null;
      const [manifestResponse, packageResponse] = await Promise.all([fetch(extension.manifestUrl, { signal: s }), fetch(extension.vsixUrl, { method: "HEAD", signal: s })]);
      if (!manifestResponse.ok) throw new Error("manifest unavailable");
      if (!packageResponse.ok || Number(packageResponse.headers.get("content-length")) > VSX_LIMITS.vsix) return null;
      const manifest = parseJsonc(decodeText(await readCapped(manifestResponse, VSX_LIMITS.manifest, "Open VSX returned an unexpectedly large manifest.")), "Extension manifest");
      return themeContributions(manifest).length && licenseMatches(manifest, extension.license) ? extension : null;
    } catch {
      throw new Error("Open VSX returned unreadable theme details.");
    }
  }, signal)));
  if (signal?.aborted) throw new DOMException("The operation was aborted.", "AbortError");
  const done = details.filter((d) => d.status === "fulfilled");
  if (identities.length && !done.length) throw new Error("Open VSX theme details are unavailable right now.");
  return done.map((d) => d.value).filter(Boolean).slice(0, 8);
}

function packagePath(path, relativeTo = "extension/") {
  if (path.length > VSX_LIMITS.path || path.includes("\0") || path.startsWith("/") || /^[a-zA-Z]:/.test(path)) throw new Error("Theme path is not a safe relative package path.");
  const segments = relativeTo.split("/").slice(0, -1);
  for (const seg of path.replaceAll("\\", "/").split("/")) {
    if (!seg || seg === ".") continue;
    if (seg === "..") {
      if (segments.length <= 1) throw new Error("Theme path escapes the extension package.");
      segments.pop();
    } else segments.push(seg);
  }
  if (segments[0] !== "extension") segments.unshift("extension");
  return segments.join("/");
}

/** The .vsix's ZIP directory, checked against the limits T3 sets before anything is unpacked. */
function readZipDirectory(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const floor = Math.max(0, bytes.byteLength - 65557);
  let end = bytes.byteLength - 22;
  while (end >= floor && (view.getUint32(end, true) !== 0x06054b50 || end + 22 + view.getUint16(end + 20, true) !== bytes.byteLength)) end--;
  if (end < floor) throw new Error("That extension package has no ZIP directory.");
  const dirSize = view.getUint32(end + 12, true);
  const dirStart = view.getUint32(end + 16, true);
  if (dirStart + dirSize !== end) throw new Error("That extension package has an invalid ZIP directory.");
  const entries = new Map();
  let total = 0;
  let at = dirStart;
  while (at < end) {
    if (at + 46 > end || view.getUint32(at, true) !== 0x02014b50) throw new Error("That extension package has an invalid ZIP directory.");
    if (entries.size >= VSX_LIMITS.entries) throw new Error("That extension package has too many files.");
    const compressed = view.getUint32(at + 20, true);
    const size = view.getUint32(at + 24, true);
    if (compressed === 0xffffffff || size === 0xffffffff) throw new Error("That extension package has unsupported ZIP64 metadata.");
    total += size;
    if (total > VSX_LIMITS.unpacked) throw new Error("That extension package expands beyond the safe import limit.");
    if (size > 0 && (compressed === 0 || size / compressed > VSX_LIMITS.ratio)) throw new Error("That extension package has an unsafe compression ratio.");
    const nameLength = view.getUint16(at + 28, true);
    const name = decodeText(bytes.subarray(at + 46, at + 46 + nameLength));
    packagePath(name, "");
    entries.set(name, { method: view.getUint16(at + 10, true), compressed, size, local: view.getUint32(at + 42, true) });
    at += 46 + nameLength + view.getUint16(at + 30, true) + view.getUint16(at + 32, true);
  }
  if (at !== end) throw new Error("That extension package has an invalid ZIP directory.");
  return { bytes, view, entries };
}
async function readZipText(zip, path, what) {
  const entry = zip.entries.get(path);
  if (!entry) throw new Error(`${what} is missing from the extension package.`);
  if (entry.size > VSX_LIMITS.theme) throw new Error(`${what} is too large.`);
  const { view, bytes } = zip;
  if (view.getUint32(entry.local, true) !== 0x04034b50) throw new Error("That Open VSX extension package could not be opened.");
  const start = entry.local + 30 + view.getUint16(entry.local + 26, true) + view.getUint16(entry.local + 28, true);
  const data = bytes.subarray(start, start + entry.compressed);
  if (entry.method === 0) return decodeText(data);
  if (entry.method !== 8) throw new Error(`${what} uses an unsupported compression.`);
  const reader = new Blob([data]).stream().pipeThrough(new DecompressionStream("deflate-raw")).getReader();
  const chunks = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > VSX_LIMITS.theme) {
      await reader.cancel();
      throw new Error(`${what} is too large.`);
    }
    chunks.push(value);
  }
  return decodeText(concatBytes(chunks, size));
}

function sanitizeThemeObject(value) {
  const colors = {};
  for (const [k, c] of Object.entries(isRecord(value.colors) ? value.colors : {})) if (USED_WORKBENCH_COLORS.has(k) && typeof c === "string" && c.length <= VSX_LIMITS.color) colors[k] = c;
  return { ...(typeof value.include === "string" ? { include: value.include } : {}), colors };
}
async function loadThemeObject(zip, path, cache, budget, ancestors = new Set()) {
  if (ancestors.size >= VSX_LIMITS.depth) throw new Error("Theme includes are nested too deeply.");
  if (ancestors.has(path)) throw new Error("Theme includes contain a cycle.");
  if (cache.has(path)) return cache.get(path);
  if (++budget.files > VSX_LIMITS.themes * VSX_LIMITS.depth) throw new Error("That extension references too many theme files.");
  const value = sanitizeThemeObject(parseJsonc(await readZipText(zip, path, path), path));
  if (typeof value.include !== "string") return cache.set(path, value), value;
  const base = await loadThemeObject(zip, packagePath(value.include, path), cache, budget, new Set([...ancestors, path]));
  const resolved = { ...base, ...value, colors: { ...base.colors, ...value.colors } };
  cache.set(path, resolved);
  return resolved;
}

async function importOpenVsxThemeExtension(extension, signal) {
  const check = () => signal?.throwIfAborted();
  const get = async (url, opts) => fetch(url, signal ? { ...opts, signal } : opts);
  const manifestResponse = await get(extension.manifestUrl);
  if (!manifestResponse.ok) throw new Error("That Open VSX extension has no readable manifest.");
  const manifest = parseJsonc(decodeText(await readCapped(manifestResponse, VSX_LIMITS.manifest, "That Open VSX extension manifest is too large.")), "Extension manifest");
  const advertised = themeContributions(manifest);
  if (!advertised.length) throw new Error("That extension does not contain color themes.");
  if (advertised.length > VSX_LIMITS.themes) throw new Error("That extension contains too many color themes to import safely.");
  const packageResponse = await get(extension.vsixUrl);
  if (!packageResponse.ok) throw new Error("That Open VSX theme could not be downloaded.");
  const packageBytes = await readCapped(packageResponse, VSX_LIMITS.vsix, "That theme extension is too large to import safely.");
  check();
  const sumResponse = await get(extension.sha256Url);
  if (!sumResponse.ok) throw new Error("That Open VSX theme has no readable checksum.");
  const expected = decodeText(await readCapped(sumResponse, 256, "That Open VSX checksum response is invalid.")).trim().split(/\s+/)[0];
  if (!/^[a-f\d]{64}$/i.test(expected)) throw new Error("That Open VSX theme has an invalid checksum.");
  if ((await sha256Hex(packageBytes)) !== expected.toLowerCase()) throw new Error("That Open VSX theme failed its integrity check.");
  check();
  let zip;
  try {
    zip = readZipDirectory(packageBytes);
  } catch (err) {
    if (err.message.startsWith("That extension package")) throw err;
    throw new Error("That Open VSX extension package could not be opened.");
  }
  const packaged = parseJsonc(await readZipText(zip, "extension/package.json", "Extension manifest"), "Extension manifest");
  if (typeof packaged.publisher !== "string" || packaged.publisher.toLowerCase() !== extension.publisher.toLowerCase() || typeof packaged.name !== "string" || `${packaged.publisher}.${packaged.name}`.toLowerCase() !== extension.id.toLowerCase() || packaged.version !== extension.version) {
    throw new Error("That extension package does not match the selected Open VSX theme.");
  }
  if (!licenseMatches(packaged, extension.license)) throw new Error("That extension package does not match its advertised license.");
  const contributions = themeContributions(packaged);
  if (!contributions.length) throw new Error("That extension does not contain color themes.");
  if (contributions.length > VSX_LIMITS.themes) throw new Error("That extension contains too many color themes to import safely.");

  const parsed = [];
  let failed = false;
  const cache = new Map();
  const budget = { files: 0 };
  for (const c of contributions) {
    check();
    try {
      if (typeof c.path !== "string") throw new Error("theme path is missing");
      const path = packagePath(c.path);
      const type = { vs: "light", "vs-dark": "dark", "hc-black": "hc-black", "hc-light": "hc-light" }[c.uiTheme];
      const decorated = { ...(await loadThemeObject(zip, path, cache, budget)), displayName: typeof c.label === "string" && c.label.trim() ? c.label.trim() : extension.name, ...(type ? { type } : {}) };
      if (!isVsCodeThemeFile(decorated)) throw new Error("not a VS Code color theme");
      parsed.push({ theme: parseVsCodeThemeFile(decorated), sourceName: path.split("/").at(-1), sourcePath: path });
    } catch {
      check();
      failed = true;
    }
  }
  if (failed) throw new Error("One or more color themes in that extension could not be imported safely.");
  if (!parsed.length) throw new Error("That extension has no compatible color themes.");
  const extId = extension.id.toLowerCase();
  const pathCounts = new Map();
  for (const { sourcePath } of parsed) pathCounts.set(sourcePath, (pathCounts.get(sourcePath) ?? 0) + 1);
  const seenPaths = new Map();
  const sources = parsed.map(({ sourcePath }) => {
    if (pathCounts.get(sourcePath) === 1) return sourcePath;
    const n = seenPaths.get(sourcePath) ?? 0;
    seenPaths.set(sourcePath, n + 1);
    return n ? `${sourcePath}\0${n}` : sourcePath;
  });
  const named = resolveThemeLabelCollisions(parsed);
  const ided = await Promise.all(named.map(async (t, i) => ({ ...t, id: await openVsxThemeId(extId, sources[i]) })));
  const pairIds = new Map();
  for (const t of ided) for (const u of ided) if (t !== u) pairIds.set(`${t.id}|${u.id}`, await openVsxThemeId(extId, [t.id, u.id].sort().join(":")));
  const paired = pairVsCodeThemes(ided, (light, dark) => pairIds.get(`${light.id}|${dark.id}`));
  const collection = { id: extension.collectionId, label: extension.name.slice(0, 48) };
  return resolveThemeLabelCollisions(paired.map((theme) => ({ theme }))).map((t) => ({ ...t, collection }));
}

/* The dialog */

const VSX_SORTS = [["downloadCount", "Most downloaded"], ["rating", "Best rated"], ["timestamp", "Newest"], ["relevance", "Most relevant"]];
const VSX_SUGGESTED = ["Dracula", "Catppuccin", "Nord", "Tokyo Night"];
const downloads = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 });
const GITLAB_ICON = `<svg class="i" viewBox="0 0 32 32" fill="currentColor"><path d="m31.46 12.78-.04-.12-4.35-11.35A1.14 1.14 0 0 0 25.94.6c-.24 0-.47.1-.66.24-.19.15-.33.36-.39.6l-2.94 9h-11.9l-2.94-9A1.14 1.14 0 0 0 6.07.58a1.15 1.15 0 0 0-1.14.72L.58 12.68l-.05.11a8.1 8.1 0 0 0 2.68 9.34l.02.01.04.03 6.63 4.97 3.28 2.48 2 1.52a1.35 1.35 0 0 0 1.62 0l2-1.52 3.28-2.48 6.67-5h.02a8.09 8.09 0 0 0 2.7-9.36Z"/></svg>`;
let themeImport = null;
const importOpen = (t) => themeImport === t && !!shellDialog.querySelector(".theme-import");

function openThemeImport() {
  themeImport?.controller?.abort();
  themeImport = { json: "", fileName: null, error: null, reading: false, conflicts: null, request: 0, query: "", sort: "downloadCount", results: null, searchError: null, searching: false, installing: null, pendingUpdate: null, shownKey: null, controller: null, timer: 0 };
  showDialog(`<div class="dialog-head"><h2>Add a theme</h2></div><div class="dialog-body theme-import"></div>`);
  const body = shellDialog.querySelector(".theme-import");
  body.innerHTML = `<section class="ti-search" aria-labelledby="theme-search-heading">
      <div><h3 id="theme-search-heading">Search community themes</h3><p class="muted">Find open-source themes from Open VSX.</p></div>
      <label class="ti-query">${icon("search")}<span class="spinner small" style="display:none"></span><input type="search" placeholder="Search themes..." aria-label="Search Open VSX themes" autocomplete="off" spellcheck="false"></label>
      <div class="ti-tools"></div><div class="sr-only" role="status"></div><div class="ti-search-error"></div><div class="ti-results"></div><div class="ti-confirm"></div></section>
    <div class="ti-rule" aria-hidden="true"><span>or import a file</span></div>
    <div class="ti-file"></div><div class="ti-error" aria-live="polite"></div>`;
  bindThemeImport(body);
  renderThemeSearch();
  renderThemeFile();
  body.querySelector(".ti-query input").focus();
  shellDialog.querySelector(".close").onclick = shellDialog.querySelector(".backdrop").onclick = endThemeImport;
}
function endThemeImport() {
  themeImport?.controller?.abort();
  clearTimeout(themeImport?.timer);
  themeImport = null;
  closeDialog();
}
const importBody = () => (themeImport && shellDialog.querySelector(".theme-import")) || null;

function renderThemeSearch() {
  const body = importBody();
  if (!body) return;
  const t = themeImport;
  body.querySelector(".ti-query .i").style.display = t.searching ? "none" : "";
  body.querySelector(".ti-query .spinner").style.display = t.searching ? "" : "none";
  const tools = body.querySelector(".ti-tools");
  tools.innerHTML = !t.searching || t.results
    ? `<div class="ti-popular"><span class="muted">Popular</span>${VSX_SUGGESTED.map((s) => `<button type="button" class="ti-chip" data-ti-suggest="${s}"${t.installing ? " disabled" : ""}>${s}</button>`).join("")}</div>
      ${t.results?.length ? `<label class="ti-sort"><span class="muted">Sort</span><select aria-label="Sort themes"${t.installing ? " disabled" : ""}>${VSX_SORTS.map(([v, l]) => `<option value="${v}"${v === t.sort ? " selected" : ""}>${l}</option>`).join("")}</select></label>` : ""}`
    : "";
  body.querySelector('[role="status"]').textContent = t.searching ? "Searching themes..." : t.results ? `${t.results.length} supported ${t.results.length === 1 ? "theme" : "themes"} found.` : "";
  body.querySelector(".ti-search-error").innerHTML = t.searchError ? `<p class="ti-alert" role="status">${esc(t.searchError)}</p>` : "";
  const results = body.querySelector(".ti-results");
  if (t.searching && !t.results) results.innerHTML = `<p class="ti-wait"><span class="spinner small"></span> Searching themes...</p>`;
  else if (!t.results) results.innerHTML = "";
  else if (!t.results.length) results.innerHTML = `<div class="ti-empty"><b>No supported open-source themes found</b><span class="muted">Try a broader search.</span></div>`;
  else {
    results.innerHTML = `<div class="ti-grid">${t.results.map((x) => {
      const installed = customThemes.some((th) => th.collection?.id === x.collectionId);
      const busy = t.installing === x.id;
      const [act, doing] = installed ? ["Update", "Updating"] : ["Install", "Installing"];
      const host = (() => {
        try {
          return new URL(x.sourceUrl).hostname.toLowerCase();
        } catch {
          return "";
        }
      })();
      const sourceIcon = host === "github.com" || host.endsWith(".github.com") ? icon("github") : host === "gitlab.com" || host.endsWith(".gitlab.com") ? GITLAB_ICON : icon("external-link");
      return `<article class="ti-card" data-ti-ext="${esc(x.id)}">
        <div class="ti-card-head"><span class="ti-icon">${x.iconUrl ? `<img alt="" loading="lazy" referrerpolicy="no-referrer" src="${esc(x.iconUrl)}">` : icon("palette")}</span>
          <div><h4>${esc(x.name)}</h4><p class="muted">${esc(x.publisher)} · ${downloads.format(x.downloadCount)} downloads</p></div></div>
        <p class="ti-desc muted">${esc(x.description || "A community color theme for your editor.")}</p>
        <div class="ti-card-foot">${x.sourceUrl ? `<a class="ti-source" href="${esc(x.sourceUrl)}" target="_blank" rel="noreferrer" aria-label="View source for ${esc(x.name)}">${sourceIcon}</a>` : "<span></span>"}
          <button type="button" class="outline ti-install" data-ti-install="${esc(x.id)}" aria-label="${busy ? doing : act} ${esc(x.name)}"${t.installing ? " disabled" : ""}>${busy ? `<span class="spinner small"></span>` : icon(installed ? "refresh-cw" : "package-plus")}${busy ? `${doing}...` : act}</button></div></article>`;
    }).join("")}</div>`;
    for (const img of results.querySelectorAll(".ti-icon img")) img.onerror = () => (img.parentElement.innerHTML = icon("palette"));
  }
  const confirm = body.querySelector(".ti-confirm");
  const p = t.pendingUpdate;
  confirm.innerHTML = p ? `<div class="ti-alert-dialog" role="alertdialog" aria-labelledby="ti-update-title"><div class="ti-alert-card">
      <h2 id="ti-update-title">Update “${esc(p.name)}”?</h2><p class="muted">This replaces its installed variants, including any local edits. Variants no longer in the extension will be removed.</p>
      <div class="theme-dialog-actions"><button type="button" class="outline" data-ti-update-cancel>Cancel</button><button type="button" class="primary" data-ti-update>Update theme</button></div></div></div>` : "";
}

/** ThemeSearchSection: a search runs 350 ms after typing stops, or at once on Enter or a suggestion. */
function queueThemeSearch() {
  const t = themeImport;
  clearTimeout(t.timer);
  if (!t.query.trim() && !t.installing) {
    t.controller?.abort();
    Object.assign(t, { results: null, searchError: null, searching: false, shownKey: null });
    return renderThemeSearch();
  }
  t.timer = setTimeout(() => runThemeSearch(), 350);
}
function runThemeSearch(force = false) {
  const t = themeImport;
  const query = t.query.trim();
  if (!t || !query || t.installing) return;
  const key = `${query}\0${t.sort}`;
  if (!force && key === t.shownKey) return;
  t.controller?.abort();
  const controller = new AbortController();
  t.controller = controller;
  Object.assign(t, { searchError: null, searching: true });
  renderThemeSearch();
  searchOpenVsxThemes(query, { signal: controller.signal, sortBy: t.sort }).then(
    (results) => {
      if (controller.signal.aborted || !importOpen(t)) return;
      Object.assign(t, { results, shownKey: key });
    },
    (err) => {
      if (controller.signal.aborted || !importOpen(t)) return;
      Object.assign(t, { results: null, shownKey: null, searchError: err?.message || "Open VSX search failed." });
    },
  ).then(() => {
    if (!importOpen(t) || t.controller !== controller) return;
    t.controller = null;
    t.searching = false;
    renderThemeSearch();
  });
}

async function installOpenVsx(extension, allowUpdate) {
  const t = themeImport;
  t.searchError = null;
  const installed = customThemes.filter((th) => th.collection?.id === extension.collectionId);
  if (installed.length && !allowUpdate) {
    t.pendingUpdate = extension;
    return renderThemeSearch();
  }
  t.controller?.abort();
  const controller = new AbortController();
  Object.assign(t, { controller, searching: false, installing: extension.id });
  renderThemeSearch();
  try {
    const themes = await importOpenVsxThemeExtension(extension, controller.signal);
    if (!controller.signal.aborted && importOpen(t)) {
      const imported = replaceThemeCollection(extension.collectionId, themes, installed);
      themesImported(imported, installed.length > 0);
      return endThemeImport();
    }
  } catch (err) {
    if (!controller.signal.aborted && importOpen(t)) t.searchError = err?.message || "That theme could not be added.";
  }
  if (!importOpen(t) || t.controller !== controller) return;
  Object.assign(t, { controller: null, installing: null });
  renderThemeSearch();
  runThemeSearch();
}

/** The file half: a drop zone and chooser, the JSON box, or the Already installed choice. */
function renderThemeFile() {
  const body = importBody();
  if (!body) return;
  const t = themeImport;
  const file = body.querySelector(".ti-file");
  body.querySelector(".ti-error").innerHTML = t.error ? `<p class="ti-alert">${esc(t.error)}</p>` : "";
  if (t.conflicts) {
    file.innerHTML = `<div class="ti-conflict"><b>Already installed</b><p class="muted">${esc(t.conflicts.map((c) => c.label).join(", "))}</p></div>
      <div class="ti-actions start"><button type="button" class="primary" data-ti-resolve="update">Update existing</button><button type="button" class="outline" data-ti-resolve="copy">Keep both</button>
      <button type="button" class="ghost" data-ti-back>Back</button><button type="button" class="ghost" data-ti-cancel>Cancel</button></div>`;
    return;
  }
  if (!file.querySelector(".ti-drop")) {
    file.innerHTML = `<div class="ti-drop"><div><b>Theme file</b><p class="muted ti-file-name"></p></div>
        <button type="button" class="outline" data-ti-choose>${icon("download")}<span>Choose files</span></button><input type="file" accept=".json,application/json" multiple hidden></div>
      <div class="ti-json"><label for="theme-json-editor">Theme JSON</label><div class="ti-editor"><pre aria-hidden="true"><code></code></pre>
        <textarea id="theme-json-editor" aria-label="Theme JSON" spellcheck="false" placeholder='{\n  "version": 1,\n  "name": "Aurora",\n  "appearance": "light",\n  "colors": { ... }\n}'></textarea></div></div>
      <div class="ti-actions"><button type="button" class="ghost" data-ti-cancel>Cancel</button><button type="button" class="primary" data-ti-add>${icon("plus")}Add theme</button></div>`;
  }
  file.querySelector(".ti-file-name").textContent = t.fileName ?? "Drop T3 Code or VS Code .json files";
  const choose = file.querySelector("[data-ti-choose]");
  choose.disabled = t.reading;
  choose.querySelector("span").textContent = t.reading ? "Reading…" : "Choose files";
  const area = file.querySelector("textarea");
  if (area.value !== t.json) area.value = t.json;
  paintThemeJson();
  file.querySelector("[data-ti-add]").disabled = !t.json.trim() || t.reading;
}
function paintThemeJson() {
  const body = importBody();
  const area = body.querySelector("#theme-json-editor");
  const pre = body.querySelector(".ti-editor pre");
  const plain = themeImport.json.length > MAX_HIGHLIGHTED_JSON;
  area.classList.toggle("plain", plain);
  pre.hidden = plain;
  if (!plain) pre.firstChild.innerHTML = highlightJson(themeImport.json);
  pre.scrollTop = area.scrollTop;
  pre.scrollLeft = area.scrollLeft;
}
function highlightJson(value) {
  let out = "";
  let at = 0;
  for (const m of value.matchAll(/"(?:\\.|[^"\\])*"|-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?|true|false|null/g)) {
    const token = m[0];
    const cls = token[0] === '"' ? (/^\s*:/.test(value.slice(m.index + token.length)) ? "key" : "str") : /^(true|false|null)$/.test(token) ? "lit" : "num";
    out += `${esc(value.slice(at, m.index))}<span class="${cls}">${esc(token)}</span>`;
    at = m.index + token.length;
  }
  return out + esc(value.slice(at));
}

async function readThemeFiles(files) {
  const t = themeImport;
  if (!files.length) return;
  if (files.length === 1) {
    const f = files[0];
    const big = oversizedTheme(f.size);
    if (big) return (t.error = big), renderThemeFile();
    const request = ++t.request;
    t.reading = true;
    renderThemeFile();
    try {
      const text = await f.text();
      if (!importOpen(t) || request !== t.request) return;
      Object.assign(t, { json: text, fileName: f.name, error: null });
    } catch {
      if (!importOpen(t) || request !== t.request) return;
      t.error = "Could not read that file. Paste the JSON below instead.";
    }
    t.reading = false;
    return renderThemeFile();
  }
  const request = ++t.request;
  t.reading = true;
  renderThemeFile();
  const failures = [];
  const parsed = [];
  for (const f of files) {
    if (oversizedTheme(f.size)) {
      failures.push(`${f.name}: too large`);
      continue;
    }
    try {
      parsed.push({ sourceName: f.name, theme: parseAnyThemeFile(JSON.parse(await f.text())) });
    } catch (err) {
      failures.push(`${f.name}: ${err?.message || "not a theme file"}`);
    }
  }
  if (!importOpen(t) || request !== t.request) return;
  t.reading = false;
  const installed = [];
  const conflicting = [];
  for (const theme of pairVsCodeThemes(resolveThemeLabelCollisions(parsed))) {
    if (customThemes.some((c) => c.id === theme.id)) {
      conflicting.push(theme);
      continue;
    }
    try {
      installed.push(installCustomTheme(theme));
    } catch (err) {
      failures.push(`${theme.label}: ${err?.message || "could not install"}`);
    }
  }
  if (installed.length) themesImported(installed, false);
  if (failures.length) t.error = failures.join("; ");
  else if (conflicting.length) t.conflicts = conflicting;
  else if (installed.length) return endThemeImport();
  renderThemeFile();
}

function addPastedTheme() {
  const t = themeImport;
  const big = oversizedTheme(t.json.length);
  if (big) return (t.error = big), renderThemeFile();
  try {
    const theme = parseAnyThemeFile(JSON.parse(t.json));
    if (customThemes.some((c) => c.id === theme.id)) {
      Object.assign(t, { error: null, conflicts: [theme] });
      return renderThemeFile();
    }
    useImportedTheme(installCustomTheme(theme));
    endThemeImport();
  } catch (err) {
    t.error = err?.message || "That theme file is invalid.";
    renderThemeFile();
  }
}

/** Keep both: the file's own name if it is free, else "Name (n)". */
function versionedCopy(theme, preferred) {
  const copy = (name) => parseThemeFile({ version: 1, name, appearance: theme.appearance, colors: theme.colors, ...(theme.variants ? { variants: theme.variants } : {}), ...(theme.managed ? { managed: true } : {}) });
  if (preferred && preferred.toLowerCase() !== theme.label.toLowerCase()) {
    const c = copy(preferred.slice(0, 48));
    if (!customThemes.some((x) => x.id === c.id)) return c;
  }
  for (let n = 1; n < 100; n++) {
    const c = copy(`${theme.label.slice(0, 48 - ` (${n})`.length)} (${n})`);
    if (!customThemes.some((x) => x.id === c.id)) return c;
  }
  throw new Error(`Too many copies of "${theme.label}".`);
}
function resolveConflicts(mode) {
  const t = themeImport;
  const done = [];
  const failures = [];
  const preferred = t.conflicts.length === 1 && t.fileName ? humanizeThemeName(t.fileName.replace(/\.[^.]+$/, "")) : null;
  for (const theme of t.conflicts) {
    try {
      const existing = customThemes.find((c) => c.id === theme.id);
      done.push(mode === "update" ? updateCustomTheme(existing?.collection ? { ...theme, collection: existing.collection } : theme) : installCustomTheme(versionedCopy(theme, preferred)));
    } catch (err) {
      failures.push(`${theme.label}: ${err?.message || "failed"}`);
    }
  }
  if (done.length) themesImported(done, mode === "update");
  t.conflicts = null;
  if (failures.length) {
    t.error = failures.join("; ");
    return renderThemeFile();
  }
  endThemeImport();
}

function bindThemeImport(body) {
  const input = body.querySelector(".ti-query input");
  input.addEventListener("input", () => {
    themeImport.query = input.value;
    queueThemeSearch();
  });
  input.addEventListener("keydown", (e) => {
    if (e.isComposing || e.keyCode === 229 || e.key !== "Enter") return;
    if (!themeImport.searching && !themeImport.installing) {
      clearTimeout(themeImport.timer);
      runThemeSearch(true);
    }
  });
  body.addEventListener("change", (e) => {
    if (e.target.matches(".ti-sort select")) {
      themeImport.sort = e.target.value;
      runThemeSearch();
    }
    if (e.target.matches('.ti-file input[type="file"]')) {
      const files = [...e.target.files];
      e.target.value = "";
      readThemeFiles(files);
    }
  });
  body.addEventListener("input", (e) => {
    if (e.target.id !== "theme-json-editor") return;
    themeImport.json = e.target.value;
    paintThemeJson();
    body.querySelector("[data-ti-add]").disabled = !themeImport.json.trim() || themeImport.reading;
  });
  body.addEventListener("scroll", (e) => e.target.id === "theme-json-editor" && paintThemeJson(), true);
  body.addEventListener("click", (e) => {
    const t = themeImport;
    const el = e.target.closest("button");
    if (!el || el.disabled) return;
    if (el.dataset.tiSuggest) {
      const s = el.dataset.tiSuggest;
      input.value = t.query = s;
      clearTimeout(t.timer);
      return runThemeSearch(true);
    }
    if (el.dataset.tiInstall) return installOpenVsx(t.results.find((x) => x.id === el.dataset.tiInstall), false);
    if (el.hasAttribute("data-ti-update")) {
      const ext = t.pendingUpdate;
      t.pendingUpdate = null;
      return installOpenVsx(ext, true);
    }
    if (el.hasAttribute("data-ti-update-cancel")) return (t.pendingUpdate = null), renderThemeSearch();
    if (el.hasAttribute("data-ti-choose")) return body.querySelector('.ti-file input[type="file"]').click();
    if (el.hasAttribute("data-ti-add")) return addPastedTheme();
    if (el.hasAttribute("data-ti-cancel")) return endThemeImport();
    if (el.hasAttribute("data-ti-back")) return (t.conflicts = null), renderThemeFile();
    if (el.dataset.tiResolve) return resolveConflicts(el.dataset.tiResolve);
  });
  const drop = (on) => body.querySelector(".ti-drop")?.classList.toggle("over", on);
  body.addEventListener("dragenter", (e) => e.target.closest?.(".ti-drop") && (e.preventDefault(), drop(true)));
  body.addEventListener("dragover", (e) => e.target.closest?.(".ti-drop") && (e.preventDefault(), drop(true)));
  body.addEventListener("dragleave", (e) => e.target.closest?.(".ti-drop") && !e.target.closest(".ti-drop").contains(e.relatedTarget) && drop(false));
  body.addEventListener("drop", (e) => {
    if (!e.target.closest?.(".ti-drop")) return;
    e.preventDefault();
    drop(false);
    readThemeFiles([...e.dataTransfer.files]);
  });
}
