/** T3 Code's themes for the app view (themePalette.ts, hooks/useTheme.ts): a light and a dark
 * appearance, System following prefers-color-scheme, the built-in palettes and the user's own, a
 * palette per appearance ("halves"), and the keys, ⌘K items, Settings section and editor that pick
 * them. gui-theme.css maps what this sets on <html> onto the app view's color tokens. */
Object.assign(ICON_PATHS, {
  sun: `<circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/>`,
  moon: `<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>`,
  palette: `<circle cx="13.5" cy="6.5" r=".5" fill="currentColor"/><circle cx="17.5" cy="10.5" r=".5" fill="currentColor"/><circle cx="8.5" cy="7.5" r=".5" fill="currentColor"/><circle cx="6.5" cy="12.5" r=".5" fill="currentColor"/><path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.926 0 1.648-.746 1.648-1.688 0-.437-.18-.835-.437-1.125-.29-.289-.438-.652-.438-1.125a1.64 1.64 0 0 1 1.668-1.668h1.996c3.051 0 5.555-2.503 5.555-5.554C21.965 6.012 17.461 2 12 2z"/>`,
  paintbrush: `<path d="m14.622 17.897-10.68-2.913"/><path d="M18.376 2.622a1 1 0 1 1 3.002 3.002L17.36 9.643a.5.5 0 0 0 0 .707l.944.944a2.41 2.41 0 0 1 0 3.408l-.944.944a.5.5 0 0 1-.707 0L8.354 7.348a.5.5 0 0 1 0-.707l.944-.944a2.41 2.41 0 0 1 3.408 0l.944.944a.5.5 0 0 0 .707 0z"/><path d="M9 8c-1.804 2.71-3.97 3.46-6.583 3.948a.507.507 0 0 0-.302.819l7.32 8.883a1 1 0 0 0 1.185.204C12.735 20.405 16 16.792 16 15"/>`,
  upload: `<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" x2="12" y1="3" y2="15"/>`,
});

const THEME_KEY = "gui-theme";
const THEME_MODE_KEY = "gui-theme-mode";
const THEME_HALVES_KEY = "gui-theme-halves";
const CUSTOM_THEMES_KEY = "gui-themes";
const THEME_PREVIEW_ID = "__preview";
const APPEARANCE_OPTIONS = [
  ["system", "System", "monitor"],
  ["light", "Light", "sun"],
  ["dark", "Dark", "moon"],
];
const RESERVED_THEME_IDS = new Set(["system", "light", "dark", "default", THEME_PREVIEW_ID, ...THEME_PALETTES.builtIn.map((t) => t.id)]);
/** ThemePreviewCircles.tsx's STANDARD_THEME_PREVIEW_COLORS: how the T3 Code card and the Color scheme
 * tiles draw the default theme. */
const STANDARD_PREVIEW = {
  light: { sidebar: "#fafafa", surface: "#ffffff", accentSurface: "#f4f4f5", messageSurface: "#e4e4e7", canvas: "#fcfcfc", accent: "#f4f4f5", messageAction: "#4f46e5" },
  dark: { sidebar: "#0f0f10", surface: "#121212", accentSurface: "#27272a", messageSurface: "#27272a", canvas: "#0a0a0a", accent: "#1c1c1f", messageAction: "#8b9cff" },
};
const TERMINAL_ANSI = {
  light: ["#000000", "#cd3131", "#107c10", "#949800", "#0451a5", "#bc05bc", "#0598bc", "#555555", "#666666", "#cd3131", "#14ce14", "#b5ba00", "#0451a5", "#bc05bc", "#0598bc", "#a5a5a5"],
  dark: ["#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5", "#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#e5e5e5"],
};
const ANSI_NAMES = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
const systemDark = matchMedia("(prefers-color-scheme: dark)");
const themeRoot = document.documentElement;

/** The roles the app view also reads as channels, for translucent uses (gui-theme.css). */
const CHANNEL_ROLES = ["canvas", "surface", "surfaceOverlay", "text", "mutedForeground", "sidebarForeground", "sidebarMutedForeground", "messageAction", "error", "warning", "accent"];
const themeVar = (role) => `--app-theme-${role.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}${role === "terminalSelection" ? "-background" : ""}`;
const colorsFor = (theme, mode) => (theme.appearance === mode ? theme.colors : theme.variants?.[mode] ?? null);
const modesOf = (theme) => ["light", "dark"].filter((m) => colorsFor(theme, m));
const standardColors = (mode) => THEME_PALETTES.standard[mode];
const chatColors = (mode) => colorsFor(THEME_PALETTES.builtIn.find((t) => t.id === "t3-chat"), mode);

function readThemeJson(key) {
  try {
    return JSON.parse(load(key) ?? "null");
  } catch {
    return null;
  }
}
let customThemes = (readThemeJson(CUSTOM_THEMES_KEY) ?? []).filter((t) => t && typeof t.id === "string" && typeof t.label === "string" && t.colors && ["light", "dark"].includes(t.appearance));
const saveCustomThemes = () => save(CUSTOM_THEMES_KEY, JSON.stringify(customThemes));
function installCustomTheme(theme) {
  if (RESERVED_THEME_IDS.has(theme.id) || customThemes.some((t) => t.id === theme.id)) throw new Error(`A theme named "${theme.label}" is already installed.`);
  customThemes = [...customThemes, theme];
  saveCustomThemes();
  applyTheme();
  return theme;
}
function updateCustomTheme(theme) {
  if (!customThemes.some((t) => t.id === theme.id)) throw new Error(`The theme "${theme.label}" is not installed.`);
  customThemes = customThemes.map((t) => (t.id === theme.id ? theme : t));
  saveCustomThemes();
  applyTheme();
  return theme;
}
/** An Open VSX install or update: the collection's themes replace the ones installed, in place. */
function replaceThemeCollection(collectionId, themes, expected) {
  const inCollection = (t) => t.collection?.id === collectionId;
  if (JSON.stringify(customThemes.filter(inCollection)) !== JSON.stringify(expected)) throw new Error("Your installed themes changed while this package was downloading. Try again.");
  const taken = themes.find((t) => RESERVED_THEME_IDS.has(t.id) || customThemes.some((c) => c.id === t.id && !inCollection(c)));
  if (taken) throw new Error(`A theme named "${taken.label}" is already installed.`);
  const at = customThemes.findIndex(inCollection);
  const rest = customThemes.filter((t) => !inCollection(t));
  customThemes = at < 0 ? [...rest, ...themes] : [...rest.slice(0, at), ...themes, ...rest.slice(at)];
  saveCustomThemes();
  applyTheme();
  return themes;
}
/** components/ui/toast.tsx at the window's top right: a glass card with a bold title, the type's icon
 * and a muted description, gone after `timeout` ms unless the pointer rests on it. A new one takes
 * the place of the last. */
const themeToastEl = Object.assign(document.createElement("div"), { id: "g-toast", role: "status", hidden: true });
themeToastEl.setAttribute("aria-live", "polite");
document.body.append(themeToastEl);
let themeToastTimer = 0;
function themeToast({ title, description = "", type = "", timeout = 5000 }) {
  const mark = { success: "circle-check", error: "circle-alert" }[type];
  themeToastEl.innerHTML = `<button type="button" class="dismiss" aria-label="Dismiss notification">${icon("x")}</button>
    <div class="title">${mark ? icon(mark, type) : ""}<b>${esc(title)}</b></div>${description ? `<p>${esc(description)}</p>` : ""}`;
  themeToastEl.hidden = false;
  themeToastEl.classList.remove("leaving");
  const close = () => {
    themeToastEl.classList.add("leaving");
    themeToastTimer = setTimeout(() => (themeToastEl.hidden = true), 300);
  };
  clearTimeout(themeToastTimer);
  themeToastTimer = setTimeout(close, timeout);
  themeToastEl.onpointerenter = () => clearTimeout(themeToastTimer);
  themeToastEl.onpointerleave = () => (themeToastTimer = setTimeout(close, timeout));
  themeToastEl.querySelector(".dismiss").onclick = () => {
    clearTimeout(themeToastTimer);
    close();
  };
}
/** The toast T3 shows after an import: one theme by name, or how many, then their names. */
function themesImported(themes, updated) {
  const verb = updated ? "updated" : "added";
  themeToast({ type: "success", title: themes.length === 1 ? `${themes[0].label} ${verb}` : `${themes.length} themes ${verb}`, description: themes.map((t) => t.label).join(", ") });
}
/** A pasted or single file theme is put to use: its one appearance, or the whole theme. */
function useImportedTheme(theme) {
  const modes = modesOf(theme);
  if (modes.length === 1) {
    setThemeHalf(modes[0], theme.id);
    themeToast({ type: "success", title: `${theme.label} added`, description: `It’s now your ${modes[0]} theme.` });
  } else {
    setTheme(theme.id);
    themeToast({ type: "success", title: `${theme.label} added`, description: "It’s now active." });
  }
}
const themeById = (id) => (id ? THEME_PALETTES.builtIn.find((t) => t.id === id) ?? customThemes.find((t) => t.id === id) ?? null : null);

/** The stored choice, as T3 keeps it: a base theme ("system", "light", "dark" or a palette id), the
 * appearance mode, and optional per-appearance palettes that override the base. */
function themeState() {
  let theme = load(THEME_KEY) || "system";
  if (!["system", "light", "dark"].includes(theme) && !themeById(theme)) theme = "system";
  const stored = load(THEME_MODE_KEY);
  const mode = ["system", "light", "dark"].includes(stored) ? stored : ["light", "dark"].includes(theme) ? theme : "system";
  const raw = readThemeJson(THEME_HALVES_KEY) ?? {};
  const halves = {};
  for (const m of ["light", "dark"]) {
    const t = themeById(raw[m]);
    if (t && colorsFor(t, m)) halves[m] = t.id;
  }
  return { theme, mode, halves };
}

/** T3's resolveThemeAppearance: the mode, or the system's under System, unless the palette lacks it. */
function resolvedAppearance(s = themeState()) {
  const want = s.mode === "system" ? (systemDark.matches ? "dark" : "light") : s.mode;
  if (s.halves[want]) return want;
  const t = themeById(s.theme);
  return t && !colorsFor(t, want) ? t.appearance : want;
}
const activeThemeId = (appearance, s = themeState()) => s.halves[appearance] ?? themeById(s.theme)?.id ?? null;

function paintTheme(appearance, colors, id) {
  themeRoot.dataset.appearance = appearance;
  if (colors) {
    themeRoot.dataset.themeId = id;
    for (const role of THEME_PALETTES.roles) if (colors[role]) themeRoot.style.setProperty(themeVar(role), colors[role]);
    for (const role of CHANNEL_ROLES) {
      const p = parseColor(colors[role]);
      if (p) themeRoot.style.setProperty(`${themeVar(role)}-rgb`, Object.values(oklchToRgb(p.color)).join(" "));
    }
  } else {
    delete themeRoot.dataset.themeId;
    for (const role of THEME_PALETTES.roles) themeRoot.style.removeProperty(themeVar(role));
    for (const role of CHANNEL_ROLES) themeRoot.style.removeProperty(`${themeVar(role)}-rgb`);
  }
  applyContrast();
  themeSurroundings();
}

function applyTheme() {
  if (themeRoot.dataset.themeId === THEME_PREVIEW_ID) return;
  const s = themeState();
  const appearance = resolvedAppearance(s);
  const t = themeById(activeThemeId(appearance, s));
  const was = themeRoot.dataset.appearance;
  paintTheme(appearance, t ? colorsFor(t, appearance) ?? t.colors : null, t?.id);
  if (was && was !== appearance) rehighlight();
  if (settingsOpen() && document.getElementById("theme-section")) renderThemeSection();
}

const codeTheme = () => (themeRoot.dataset.appearance === "light" ? "pierre-light" : "pierre-dark");
/** Shiki writes its colors inline, so code shown in the old theme is drawn again. */
function rehighlight() {
  if (!guiOn()) return;
  rendered.clear();
  rerender();
  renderPanel();
}

/** T3 themes its terminal drawer: the drawer's xterm takes the theme's terminal colors in the app
 * view, and the terminal view keeps its own dark look. */
function terminalTheme() {
  if (!guiOn()) return { background: "#00000000" };
  const css = getComputedStyle(themeRoot);
  const v = (name) => css.getPropertyValue(name).trim();
  const ansi = TERMINAL_ANSI[themeRoot.dataset.appearance === "light" ? "light" : "dark"];
  const named = Object.fromEntries(ANSI_NAMES.flatMap((n, i) => [[n, ansi[i]], [`bright${n[0].toUpperCase()}${n.slice(1)}`, ansi[i + 8]]]));
  return { background: "#00000000", foreground: v("--g-term-fg"), cursor: v("--g-term-cursor"), cursorAccent: v("--g-term-bg"), selectionBackground: v("--g-term-selection"), ...named };
}
function themeSurroundings() {
  const theme = terminalTheme();
  for (const t of tabs) t.term.options.theme = theme;
  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.content = guiOn() ? getComputedStyle(themeRoot).getPropertyValue("--g-canvas").trim() : "#0f1115";
  syncWindowTheme();
}

/** T3's resolveDesktopTheme: the appearance the window itself should take, or null to follow the
 * system. Under System that is still the palette's own appearance when it has only one. */
function desktopTheme(s = themeState()) {
  if (s.mode === "system") {
    const t = themeById(s.theme);
    const has = (m) => !!s.halves[m] || !!(t && colorsFor(t, m));
    return t && !(has("light") && has("dark")) ? t.appearance : null;
  }
  if (s.halves[s.mode]) return s.mode;
  const t = themeById(s.theme);
  return t && !colorsFor(t, s.mode) ? t.appearance : s.mode;
}
/** T3's desktop shell sets nativeTheme.themeSource, so the window's own chrome, menus, scrollbars and
 * the browser panel's page follow the app's appearance. The terminal view leaves it to the system. */
let windowTheme;
function syncWindowTheme() {
  if (!TAURI || themeRoot.dataset.themeId === THEME_PREVIEW_ID) return;
  const want = guiOn() ? desktopTheme() : null;
  if (want === windowTheme) return;
  windowTheme = want;
  TAURI.window.getCurrentWindow().setTheme(want).catch((err) => {
    console.error("Couldn't set the window's appearance", err);
    if (windowTheme === want) windowTheme = undefined;
  });
}
new MutationObserver(themeSurroundings).observe(document.body, { attributes: true, attributeFilter: ["data-mode"] });
new MutationObserver(themeSurroundings).observe(document.getElementById("terms"), { childList: true });
systemDark.addEventListener("change", () => themeState().mode === "system" && applyTheme());
window.addEventListener("storage", (e) => {
  if (e.key === CONTRAST_KEY || e.key === null) applyContrast(), showContrast();
  if (![THEME_KEY, THEME_MODE_KEY, THEME_HALVES_KEY, CUSTOM_THEMES_KEY, null].includes(e.key)) return;
  if (e.key === CUSTOM_THEMES_KEY || e.key === null) customThemes = (readThemeJson(CUSTOM_THEMES_KEY) ?? []).filter((t) => t?.id && t.colors);
  applyTheme();
});

/** T3's setTheme: keeps the mode, clears the halves, stores the base. */
function setTheme(next) {
  save(THEME_MODE_KEY, themeState().mode);
  localStorage.removeItem(THEME_HALVES_KEY);
  save(THEME_KEY, next);
  applyTheme();
}
function setAppearanceMode(mode) {
  save(THEME_MODE_KEY, mode);
  applyTheme();
}
function setThemeHalf(appearance, id) {
  const halves = readThemeJson(THEME_HALVES_KEY) ?? {};
  if (id) halves[appearance] = id;
  else delete halves[appearance];
  if (halves.light || halves.dark) save(THEME_HALVES_KEY, JSON.stringify(halves));
  else localStorage.removeItem(THEME_HALVES_KEY);
  applyTheme();
}

function cycleAppearance() {
  const next = { system: "light", light: "dark", dark: "system" }[themeState().mode];
  setAppearanceMode(next);
  themeToast({ title: `Appearance: ${APPEARANCE_OPTIONS.find(([m]) => m === next)[1]}`, timeout: 1500 });
}

/** The cards Settings and ⌘K offer: T3 Code (the standard light and dark), then each palette. */
function themeCards() {
  const standard = { id: null, label: "T3 Code", previews: ["light", "dark"].map((mode) => ({ mode, colors: { ...standardColors(mode), ...STANDARD_PREVIEW[mode] } })) };
  const seen = new Set();
  return [standard, ...[...THEME_PALETTES.builtIn, ...customThemes].filter((t) => !seen.has(t.id) && seen.add(t.id)).map((t) => ({ id: t.id, label: t.label, custom: !RESERVED_THEME_IDS.has(t.id), previews: modesOf(t).map((mode) => ({ mode, colors: colorsFor(t, mode) })) }))];
}
function chooseCard(card) {
  if (card.previews.length === 1 && card.id) setThemeHalf(card.previews[0].mode, card.id);
  else setTheme(card.id ?? themeState().mode);
}

function previewCircle({ mode, colors }, cls = "") {
  const c = (role) => esc(colors[role]);
  const bg = mode === "light"
    ? `background-color: color-mix(in oklab, ${c("canvas")} 80%, #ffffff); background-image: radial-gradient(circle at 72% 22% in oklab, ${c("accent")} 0%, color-mix(in oklab, ${c("accent")} 72%, transparent) 28%, transparent 58%), radial-gradient(circle at 18% 82% in oklab, color-mix(in oklab, ${c("messageAction")} 45%, transparent) 0%, transparent 55%)`
    : `background-color: color-mix(in oklab, ${c("canvas")} 80%, #09090b); background-image: radial-gradient(circle at 28% 78% in oklab, ${c("accent")} 0%, color-mix(in oklab, ${c("accent")} 62%, transparent) 28%, transparent 58%), radial-gradient(circle at 82% 18% in oklab, color-mix(in oklab, ${c("messageAction")} 45%, transparent) 0%, transparent 55%)`;
  return `<span class="theme-circle ${mode} ${cls}"><span style="${bg}"></span></span>`;
}

PALETTE_VIEWS.themes = {
  placeholder: "Search themes...",
  groups(query) {
    const s = themeState();
    const current = s.halves[resolvedAppearance(s)] ?? themeById(s.theme)?.id ?? null;
    const items = themeCards().map((card) => ({
      icon: "palette",
      title: esc(card.label),
      sub: card.previews.length === 1 ? `For ${card.previews[0].mode} mode` : "",
      trail: `<span class="theme-trail">${card.id === current ? `<span class="current">Current</span>` : ""}${card.previews.map((p) => previewCircle(p, "dot")).join("")}</span>`,
      terms: [card.label, "theme", "appearance"],
      run: () => chooseCard(card),
    }));
    return filterGroups([{ label: "Change theme", items }], query);
  },
};
PALETTE_VIEWS.appearance = {
  placeholder: "Search appearance...",
  groups(query) {
    const { mode } = themeState();
    const items = APPEARANCE_OPTIONS.map(([m, label, ic]) => ({ icon: ic, title: label, trail: m === mode ? `<span class="theme-trail"><span class="current">Current</span></span>` : "", terms: [label, "appearance", "mode"], run: () => setAppearanceMode(m) }));
    return filterGroups([{ label: "Change appearance", items }], query);
  },
};
/** CommandPaletteThemeItems.tsx: Change theme, Change appearance, Toggle theme editor. */
function themeActionItems() {
  return [
    { icon: "palette", title: "Change theme", page: "themes", hint: chordLabel("A", { alt: true }), terms: ["change theme", "appearance", "colors", "palette"] },
    { icon: "monitor", title: "Change appearance", page: "appearance", hint: chordLabel("A", { alt: true, shift: true }), terms: ["change appearance", "light", "dark", "system", "mode", "toggle"] },
    { icon: "palette", title: "Toggle theme editor", hint: chordLabel("T", { alt: true, shift: true }), terms: ["toggle theme editor", "appearance", "colors", "palette", "customize"], run: () => toggleThemeEditor() },
  ];
}
function openThemePicker() {
  openPalette();
  palette.views.push({ view: "themes", query: "" });
  renderPalette();
}

// T3 takes these anywhere in the app except in a focused terminal (keybindings.ts), once per press.
const themeKeyOptions = (shift) => ({ alt: true, shift, anywhere: true, when: (e) => !e.target.closest?.(".xterm") });
onGuiKey("a", (e) => e.repeat || openThemePicker(), themeKeyOptions(false));
onGuiKey("a", (e) => e.repeat || cycleAppearance(), themeKeyOptions(true));
onGuiKey("t", (e) => e.repeat || toggleThemeEditor(), themeKeyOptions(true));

/** appearanceContrast.ts: 50 to 200%. Under 100 the text and borders fade toward their surface, over
 * 100 the text moves toward black or white and the borders toward the text (gui-theme.css). */
const CONTRAST_KEY = "gui-contrast";
const CONTRAST = { min: 50, max: 200, standard: 100 };
function contrastLevel() {
  const n = Number(load(CONTRAST_KEY));
  return Number.isInteger(n) && n >= CONTRAST.min && n <= CONTRAST.max ? n : CONTRAST.standard;
}
/** Text is moved on its channels (--g-fg-rgb and kin), so the many rules that read them translucently
 * follow too, as everything reading T3's --contrast-* variables does. Each against its own surface. */
const CONTRAST_TEXT = [["--g-fg-rgb", "--g-canvas-rgb"], ["--g-muted-rgb", "--g-canvas-rgb"], ["--g-side-fg-rgb", "--g-sidebar"], ["--g-side-muted-rgb", "--g-sidebar"]];
function applyContrast() {
  const c = contrastLevel();
  for (const [name] of CONTRAST_TEXT) themeRoot.style.removeProperty(name);
  if (c === CONTRAST.standard) delete themeRoot.dataset.contrast;
  else themeRoot.dataset.contrast = c;
  themeRoot.style.setProperty("--g-contrast-base", `${Math.min(c, 100)}%`);
  themeRoot.style.setProperty("--g-contrast-boost", `${Math.max(c - 100, 0)}%`);
  themeRoot.style.setProperty("--g-contrast-border-boost", `${Math.max(c - 100, 0) / 4}%`);
  if (c === CONTRAST.standard) return;
  const css = getComputedStyle(themeRoot);
  const color = (name) => {
    const v = css.getPropertyValue(name).trim();
    return parseColor(/^[\d.]+\s+[\d.]+\s+[\d.]+$/.test(v) ? `rgb(${v})` : v)?.color;
  };
  const lab = ({ L, C, h }) => [L, C * Math.cos((h * Math.PI) / 180), C * Math.sin((h * Math.PI) / 180)];
  const mix = (x, y, t) => x.map((v, i) => v + (y[i] - v) * t);
  const target = lab(rgbToOklch(themeRoot.dataset.appearance === "light" ? BLACK : WHITE));
  for (const [name, surface] of CONTRAST_TEXT) {
    const text = color(name);
    const under = color(surface);
    if (!text || !under) continue;
    const [L, a, b] = mix(mix(lab(under), lab(text), Math.min(c, 100) / 100), target, Math.max(c - 100, 0) / 100);
    const rgb = oklchToRgb({ L, C: Math.hypot(a, b), h: (Math.atan2(b, a) * 180) / Math.PI });
    themeRoot.style.setProperty(name, `${rgb.r} ${rgb.g} ${rgb.b}`);
  }
}
function setContrast(c) {
  if (c === CONTRAST.standard) localStorage.removeItem(CONTRAST_KEY);
  else save(CONTRAST_KEY, String(c));
  applyContrast();
}

/** AppearanceSettingsPanel.tsx's Contrast row. */
resettable("contrast", { changed: () => contrastLevel() !== CONTRAST.standard, reset: () => setContrast(CONTRAST.standard) });
const contrastRowHtml = () => settingsRowHtml({ id: "setting-appearance-contrast", title: "Contrast", about: "Adjust the contrast of colors and borders across the interface.", reset: "contrast",
  control: sliderHtml({ id: "appearance-contrast", label: "Contrast", min: CONTRAST.min, max: CONTRAST.max, step: 5, value: contrastLevel(), unit: "%" }) });
function showContrast() {
  const input = document.getElementById("appearance-contrast");
  if (!input) return;
  input.value = contrastLevel();
  paintSlider(input, `${contrastLevel()}%`);
  refreshResets();
}
document.addEventListener("input", (e) => {
  if (e.target.id !== "appearance-contrast") return;
  const c = Number(e.target.value);
  if (!Number.isInteger(c) || c < CONTRAST.min || c > CONTRAST.max) return;
  setContrast(c);
  showContrast();
});

/** ThemeSettings.tsx: the Color scheme tiles, then the theme library. */
function themeSettingsHtml() {
  return settingsSectionHtml({ id: "appearance", title: "Colors & themes", plain: true, rows: `<div id="theme-section"></div>` });
}
function bindThemeSettings() {
  renderThemeSection();
}

const WIREFRAME_LINE = "rgb(127 127 127 / 0.25)";
function wireframePane(colors, half = "") {
  const c = (role) => esc(colors[role]);
  return `<span class="wire-pane ${half}" style="--w-canvas:${c("canvas")};--w-sidebar:${c("sidebar")};--w-surface:${c("surface")};--w-accent-surface:${c("accentSurface")};--w-message:${c("messageSurface")};--w-action:${c("messageAction")};--w-line:${WIREFRAME_LINE}">
    <i class="w-side"></i><i class="w-row r1"></i><i class="w-row r2"></i><i class="w-row r3"></i><i class="w-row r4"></i>
    <i class="w-bubble"></i><i class="w-text t1"></i><i class="w-text t2"></i>
    <i class="w-composer"><b></b><s></s></i>
    <i class="w-panel"><b><u style="background:#34d399"></u><em></em></b><b><u></u><em></em></b><b><u style="background:#fbbf24"></u><em></em></b></i></span>`;
}

function renderThemeSection() {
  const section = document.getElementById("theme-section");
  if (!section) return;
  const s = themeState();
  const owners = { light: activeThemeId("light", s), dark: activeThemeId("dark", s) };
  const ownerColors = (mode) => {
    const t = themeById(owners[mode]);
    return (t && colorsFor(t, mode)) ?? (t && t.colors) ?? { ...standardColors(mode), ...STANDARD_PREVIEW[mode] };
  };
  const tiles = APPEARANCE_OPTIONS.map(([mode, label]) => {
    const frame = mode === "system" ? wireframePane(ownerColors("light"), "left") + wireframePane(ownerColors("dark"), "right") : wireframePane(ownerColors(mode));
    const aria = { system: "Follow the system appearance", light: "Use light mode", dark: "Use dark mode" }[mode];
    return `<button type="button" class="mode-tile${s.mode === mode ? " on" : ""}" data-theme-mode="${mode}" aria-pressed="${s.mode === mode}" aria-label="${aria}"><span class="wireframe">${frame}</span><span class="label">${label}</span></button>`;
  }).join("");
  const cardHtml = (card) => {
    const tip = card.previews.length === 1 ? `Use for ${card.previews[0].mode} mode only` : "Use for both light and dark";
    const circles = card.previews.map((p) => {
      const picked = owners[p.mode] === card.id;
      return `<button type="button" class="pick${picked ? " picked" : ""}" data-theme-half="${p.mode}" data-theme-card="${esc(card.id ?? "")}" aria-label="Use ${esc(card.label)} ${p.mode} mode" title="Use for ${p.mode} mode only">${previewCircle(p)}${picked ? `<span class="badge">${icon(p.mode === "light" ? "sun" : "moon")}</span>` : ""}</button>`;
    }).join("");
    const action = (name, ic, label, aria) => `<button type="button" class="icon${name === "remove" ? " danger" : ""}" data-theme-action="${name}" data-theme-card="${esc(card.id ?? "")}" title="${label}" aria-label="${aria} ${esc(card.label)}">${icon(ic)}</button>`;
    return `<div class="theme-card" data-theme-card="${esc(card.id ?? "")}" title="${tip}">
      <div class="circles">${circles}</div>
      <div class="foot"><button type="button" class="name" data-theme-choose="${esc(card.id ?? "")}">${esc(card.label)}</button>
        ${action("duplicate", "copy", "Duplicate theme", "Duplicate")}${card.custom ? action("edit", "pencil", "Edit theme", "Edit") + action("export", "upload", "Export theme file", "Export") + action("remove", "trash-2", "Remove theme", "Remove") : ""}</div></div>`;
  };
  const cards = themeGroups().map((g) => (g.length > 1 ? collectionCardHtml(g, owners) : cardHtml(g[0]))).join("");
  section.innerHTML = `<h3 class="theme-head">Color scheme</h3>
    <div class="mode-tiles" role="group" aria-label="Appearance mode">${tiles}</div>
    <div class="themes-head"><h3>Themes</h3><span class="actions">
      <button type="button" class="outline" data-theme-action="create">${icon("paintbrush")}Create theme</button>
      <button type="button" class="outline" data-theme-action="import">${icon("plus")}Add theme</button></span></div>
    <div class="theme-cards">${cards}</div>`;
}

document.addEventListener("click", (e) => {
  const section = e.target.closest?.("#theme-section");
  if (!section) return;
  const card = (id) => themeCards().find((c) => (c.id ?? "") === id);
  const variant = e.target.closest("[data-theme-variant]");
  const collection = e.target.closest(".theme-card.collection")?.dataset.themeCollection;
  if (variant) {
    const theme = collectionOf(collection)[Number(variant.dataset.themeVariant)];
    collectionVariant.set(collection, Number(variant.dataset.themeVariant));
    return chooseHalf(variant.dataset.themeVariantMode, theme.id);
  }
  if (collection && !e.target.closest("[data-theme-action]")) return useCollectionDefaults(collection);
  const el = e.target.closest("[data-theme-mode], [data-theme-half], [data-theme-action], [data-theme-choose], .theme-card");
  if (!el) return;
  if (el.dataset.themeMode) return setAppearanceMode(el.dataset.themeMode);
  if (el.dataset.themeHalf) return chooseHalf(el.dataset.themeHalf, el.dataset.themeCard || null);
  const action = el.dataset.themeAction;
  if (action === "create") return openThemeEditor({ seedId: activeThemeId(resolvedAppearance()) });
  if (action === "import") return openThemeImport();
  const target = card(el.dataset.themeCard ?? el.dataset.themeChoose ?? "");
  if (!target) return;
  if (action === "duplicate") return openThemeEditor({ seedId: target.id, seedName: `${target.label} copy` });
  if (action === "edit") return openThemeEditor({ editingId: target.id });
  if (action === "export") return exportTheme(themeById(target.id));
  if (action === "remove") return confirmRemoveTheme(themeById(target.id));
  chooseCard(target);
});
/** The library's cards, with an Open VSX extension's themes on one card (T3's customThemeCollections). */
function themeGroups() {
  const groups = new Map();
  for (const card of themeCards()) {
    const key = card.custom && themeById(card.id).collection ? `collection:${themeById(card.id).collection.id}` : `card:${card.id}`;
    groups.set(key, [...(groups.get(key) ?? []), card]);
  }
  return [...groups.values()];
}
/** The words every variant's name shares are dropped: "Catppuccin Latte" shows as "Latte". */
function variantLabels(labels) {
  const words = labels.map((l) => l.trim().split(/\s+/));
  const shared = words[0].findIndex((w, i) => words.some((ws) => ws[i]?.toLowerCase() !== w.toLowerCase()));
  const prefix = shared === -1 ? words[0].length - 1 : shared;
  return labels.map((l, i) => words[i].slice(Math.max(0, prefix)).join(" ").trim() || l);
}
const collectionVariant = new Map();
/** CustomThemeCollectionCard: per appearance, the variant in use (or the first) as a circle that fans
 * out the others on hover or focus; the footer acts on the variant last picked. */
function collectionCardHtml(cards, owners) {
  const themes = cards.map((c) => themeById(c.id));
  const cid = themes[0].collection.id;
  const label = themes[0].collection.label;
  const names = variantLabels(themes.map((t) => t.label));
  const active = themes.findIndex((t) => owners.light === t.id || owners.dark === t.id);
  const index = Math.min(collectionVariant.get(cid) ?? (active < 0 ? 0 : active), themes.length - 1);
  const current = cards[index];
  const modes = ["light", "dark"].map((mode) => {
    const options = cards.flatMap((card, i) => card.previews.filter((p) => p.mode === mode).map((preview) => ({ i, preview, name: names[i], on: owners[mode] === card.id })));
    if (!options.length) return "";
    const sel = options.find((o) => o.on) ?? options[0];
    const x = mode === "light" ? -52 : 52;
    const fan = options.length > 1 ? options.map((o, n) => {
      const t = n / (options.length - 1) - 0.5;
      return `<button type="button" class="variant-option${o.on ? " picked" : ""}" style="--x:${x}px;--cx:${x + t * 68}px;--cy:${Math.abs(t) * 10}px;--d:${n * 35}ms" data-theme-variant="${o.i}" data-theme-variant-mode="${mode}" aria-label="Use ${esc(o.name)} for ${mode} mode${o.on ? ", currently active" : ""}" aria-pressed="${o.on}" title="Use ${esc(o.name)} for ${mode} mode">${previewCircle(o.preview, "mini")}</button>`;
    }).join("") : "";
    const aria = options.length > 1 ? `Choose ${mode} variant, ${options.length} options, currently ${esc(sel.name)}` : `Use ${mode} variant, currently ${esc(sel.name)}`;
    return `<div class="variant-mode" data-mode="${mode}" style="--x:${x}px">
      <button type="button" class="pick root${sel.on ? " picked" : ""}" data-theme-variant="${sel.i}" data-theme-variant-mode="${mode}" aria-label="${aria}" aria-pressed="${sel.on}" title="${mode === "light" ? "Light" : "Dark"}: ${esc(sel.name)}">${previewCircle(sel.preview)}${sel.on ? `<span class="badge">${icon(mode === "light" ? "sun" : "moon")}</span>` : ""}</button>
      <span class="variant-label"><span>${esc(sel.name)}</span>${options.length > 1 ? `<span class="more">+${options.length - 1}</span>` : ""}</span>${fan}</div>`;
  }).join("");
  const action = (name, ic, tip, aria) => `<button type="button" class="icon${name === "remove" ? " danger" : ""}" data-theme-action="${name}" data-theme-card="${esc(current.id)}" title="${tip}" aria-label="${aria}">${icon(ic)}</button>`;
  return `<div class="theme-card collection" data-theme-collection="${esc(cid)}" title="Use the first variants for light and dark">
    <div class="circles variants" role="group" aria-label="Light and dark theme variants">${modes}</div>
    <div class="foot"><button type="button" class="name" data-theme-collection-use aria-label="Use ${esc(label)}, ${esc(current.label)} variant">${esc(label)}</button>
      ${action("duplicate", "copy", "Duplicate theme", `Duplicate ${esc(current.label)}`)}${action("edit", "pencil", "Edit theme", `Edit ${esc(current.label)}`)}${action("export", "upload", "Export theme file", `Export ${esc(current.label)}`)}${action("remove", "trash-2", "Remove themes", `Remove themes from ${esc(label)}`)}</div></div>`;
}
const collectionOf = (cid) => customThemes.filter((t) => t.collection?.id === cid);
function useCollectionDefaults(cid) {
  const themes = collectionOf(cid);
  if (themes.length === 1) return chooseCard(themeCards().find((c) => c.id === themes[0].id));
  const light = themes.find((t) => colorsFor(t, "light"));
  const dark = themes.find((t) => colorsFor(t, "dark"));
  collectionVariant.set(cid, 0);
  if (light) chooseHalf("light", light.id);
  if (dark) chooseHalf("dark", dark.id);
}

/** Picking one circle: T3's assignHalf, with its case for the standard theme under a palette base. */
function chooseHalf(mode, id) {
  const s = themeState();
  if (!id && themeById(s.theme)) {
    const other = mode === "light" ? "dark" : "light";
    const keep = activeThemeId(other, s);
    setTheme(s.mode === "system" ? "system" : s.mode);
    if (keep) setThemeHalf(other, keep);
    return;
  }
  setThemeHalf(mode, id);
}

function confirmRemoveTheme(theme) {
  const collection = theme.collection ? collectionOf(theme.collection.id) : [];
  if (collection.length < 2) {
    showDialog(`<div class="dialog-head"><h2>Remove “${esc(theme.label)}”?</h2><p class="muted">You can bring it back anytime by importing its JSON file.</p></div>
      <div class="theme-dialog-actions"><button type="button" class="outline" data-dialog-cancel>Cancel</button><button type="button" class="danger" data-dialog-remove>Remove theme</button></div>`);
  } else {
    showDialog(`<div class="dialog-head"><h2>Remove themes from “${esc(theme.collection.label)}”?</h2><p class="muted">Select the variants you want to remove. You can restore them by importing the extension again.</p></div>
      <div class="theme-remove-grid">${collection.map((t) => `<label class="theme-remove-option"><input type="checkbox" value="${esc(t.id)}" aria-label="${esc(t.label)}">
        <span class="previews">${modesOf(t).map((mode) => previewCircle({ mode, colors: colorsFor(t, mode) }, "small")).join("")}</span><span class="label">${esc(t.label)}</span></label>`).join("")}</div>
      <div class="theme-dialog-actions"><button type="button" class="outline" data-dialog-cancel>Cancel</button><button type="button" class="danger" data-dialog-remove disabled>Remove selected</button></div>`);
    const remove = shellDialog.querySelector("[data-dialog-remove]");
    shellDialog.querySelector(".theme-remove-grid").onchange = () => {
      const n = shellDialog.querySelectorAll(".theme-remove-grid input:checked").length;
      remove.disabled = !n;
      remove.textContent = `Remove selected${n ? ` (${n})` : ""}`;
    };
  }
  shellDialog.querySelector("[data-dialog-cancel]").onclick = closeDialog;
  shellDialog.querySelector("[data-dialog-remove]").onclick = () => {
    const ids = collection.length < 2 ? [theme.id] : [...shellDialog.querySelectorAll(".theme-remove-grid input:checked")].map((i) => i.value);
    closeDialog();
    removeThemes(ids);
  };
}
/** T3's handleConfirmRemoveTheme: a removed base falls back to the mode, keeping the other halves. */
function removeThemes(ids) {
  const removed = new Set(ids);
  const s = themeState();
  const halves = readThemeJson(THEME_HALVES_KEY) ?? {};
  const removesBase = removed.has(s.theme);
  if (removesBase) setTheme(s.mode === "system" ? "system" : s.mode);
  for (const m of ["light", "dark"]) {
    if (halves[m] === undefined) continue;
    if (removed.has(halves[m])) setThemeHalf(m, null);
    else if (removesBase) setThemeHalf(m, halves[m]);
  }
  customThemes = customThemes.filter((t) => !removed.has(t.id));
  saveCustomThemes();
  applyTheme();
}

function themeFile(theme) {
  const canon = (colors) => Object.fromEntries(Object.entries(colors).map(([k, v]) => [k, canonicalColor(v) ?? v]));
  const file = { version: 1, id: theme.id, name: theme.label, appearance: theme.appearance, colors: canon(theme.colors) };
  if (theme.variants) file.variants = Object.fromEntries(Object.entries(theme.variants).map(([m, c]) => [m, canon(c)]));
  if (theme.collection) file.collection = theme.collection;
  if (theme.managed) file.managed = true;
  return `${JSON.stringify(file, null, 2)}\n`;
}
async function exportTheme(theme) {
  const blob = new Blob([themeFile(theme)], { type: "application/json" });
  if (TAURI) return saveCapture(`${theme.id}.json`, blob, "Theme");
  const a = Object.assign(document.createElement("a"), { href: URL.createObjectURL(blob), download: `${theme.id}.json` });
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 30000);
}

const themeIdFromName = (name) => name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 48).replace(/-+$/, "") || "custom-theme";
/** T3's parseThemeFile: version 1, a name, an appearance, and every role a literal color. */
function parseThemeFile(file) {
  if (file?.version !== 1) throw new Error("The file isn't a version 1 theme.");
  const name = typeof file.name === "string" ? file.name.trim() : "";
  if (!name || name.length > 48) throw new Error("The theme needs a name of 1 to 48 characters.");
  if (!["light", "dark"].includes(file.appearance)) throw new Error('The appearance must be "light" or "dark".');
  const id = file.id ?? themeIdFromName(name);
  if (!/^[a-z0-9][a-z0-9-]{0,47}$/.test(id) || RESERVED_THEME_IDS.has(id)) throw new Error(`The id "${id}" can't be used.`);
  const colorsOf = (colors, mode) => {
    if (!colors || typeof colors !== "object") throw new Error("The colors must be an object.");
    for (const [role, value] of Object.entries(colors)) {
      if (!THEME_PALETTES.roles.includes(role)) throw new Error(`"${role}" isn't a theme color.`);
      if (!parseColor(value)) throw new Error(`The color for "${role}" must be a literal CSS color such as oklch(0.62 0.2 280).`);
    }
    return { ...chatColors(mode), ...colors };
  };
  const theme = { id, label: name, appearance: file.appearance, colors: colorsOf(file.colors, file.appearance) };
  const other = file.appearance === "light" ? "dark" : "light";
  if (file.variants?.[other]) theme.variants = { [other]: colorsOf(file.variants[other], other) };
  if (file.collection !== undefined) {
    const c = file.collection;
    if (!c || typeof c.id !== "string" || !/^[a-z0-9][a-z0-9.:-]{0,127}$/i.test(c.id) || typeof c.label !== "string" || !c.label.trim() || c.label.trim().length > 48) throw new Error("Theme collections need a valid id and label.");
    theme.collection = { id: c.id, label: c.label.trim() };
  }
  if (file.managed) theme.managed = true;
  return theme;
}

/** ThemeEditorPanel.tsx: a floating panel that edits a palette with a live preview of the app. */
const THEME_FAMILIES = [
  ["Foundation", [["canvas", "Background"], ["surface", "Surface"], ["surfaceRaised", "Raised surface"], ["surfaceOverlay", "Overlay"], ["text", "Text"], ["mutedForeground", "Muted text"], ["border", "Border"], ["input", "Input"]]],
  ["Brand & content", [["secondary", "Subtle surface"], ["accentSurface", "Highlight surface"], ["accent", "Accent"], ["messageAction", "Action"], ["messageSurface", "Message surface"], ["codeBackground", "Code surface"]]],
  ["Search", [["searchMatchBackground", "Search matches"], ["searchMatchActiveBackground", "Current match"]]],
  ["Context", [["sidebar", "Sidebar background"], ["sidebarControlSurface", "Sidebar controls"], ["sidebarRowSelected", "Sidebar selection"], ["terminalBackground", "Terminal background"]]],
  ["Status", [["error", "Error"], ["warning", "Warning"]]],
];
let themeEditor = null;

function toggleThemeEditor() {
  if (themeEditor) return closeThemeEditor();
  const appearance = resolvedAppearance();
  openThemeEditor({ seedId: activeThemeId(appearance), appearance });
}

function openThemeEditor({ editingId = null, seedId = null, seedName = "", appearance = resolvedAppearance() }) {
  closeThemeEditor();
  const source = themeById(editingId ?? seedId);
  const colors = { light: { ...standardColors("light") }, dark: { ...standardColors("dark") } };
  if (source) for (const m of modesOf(source)) colors[m] = { ...colorsFor(source, m) };
  themeEditor = {
    editingId,
    name: editingId ? source.label : seedName,
    appearance: source && !colorsFor(source, appearance) ? source.appearance : appearance,
    colors,
    advanced: !!source && !source.managed,
    regenerate: !!source && !source.managed,
    invalid: { light: {}, dark: {} },
    filter: "",
    error: "",
    minimized: false,
  };
  const panel = document.createElement("div");
  panel.id = "theme-editor";
  panel.dataset.themeEditorPanel = "";
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-label", editingId ? "Edit theme" : "Create theme");
  document.body.append(panel);
  bindThemeEditor(panel);
  renderThemeEditor();
  previewThemeEditor();
  panel.querySelector("#theme-name")?.focus();
}

function closeThemeEditor() {
  clearInspectHover();
  closeColorPicker();
  stopUsageWatch();
  document.getElementById("theme-editor")?.remove();
  themeEditor = null;
  if (themeRoot.dataset.themeId === THEME_PREVIEW_ID) delete themeRoot.dataset.themeId;
  applyTheme();
}

const editingTheme = () => themeById(themeEditor?.editingId);
/** Another custom theme the name lands on, which saving merges into, as T3's mergeTarget. */
function mergeTarget() {
  const id = themeIdFromName(themeEditor.name);
  const label = themeEditor.name.trim().toLowerCase();
  return customThemes.find((t) => t.id !== themeEditor.editingId && (t.id === id || t.label.toLowerCase() === label)) ?? null;
}
function lockedReason(mode) {
  const editing = editingTheme();
  if (editing && !colorsFor(editing, mode)) return `“${editing.label}” has no ${mode} palette. Create a theme with the same name to add one.`;
  const target = !editing && mergeTarget();
  if (target && colorsFor(target, mode)) return `“${target.label}” already has a ${mode} palette.`;
  return "";
}

function previewThemeEditor() {
  const was = themeRoot.dataset.appearance;
  paintTheme(themeEditor.appearance, themeEditor.colors[themeEditor.appearance], THEME_PREVIEW_ID);
  if (was !== themeEditor.appearance) rehighlight();
}

/** ThemeColorPicker.tsx's ThemeColorField: the label toggles where the color is used, the swatch opens
 * the picker, and the hex field types a value. */
function themeField(role, label) {
  const invalid = themeEditor.invalid[themeEditor.appearance][role];
  const value = invalid === undefined ? themeEditor.colors[themeEditor.appearance][role] : "#000000";
  const selected = themeEditor.selected === role;
  return `<div class="te-field${selected ? " selected" : ""}" data-theme-color-role="${role}">
    <button type="button" class="te-label" data-te-usage="${role}" aria-pressed="${selected}" aria-label="${selected ? "Hide" : "Show"} ${esc(label)} usage" title="${selected ? "Hide" : "Show"} where ${esc(label)} is used">${esc(label)}</button>
    <button type="button" class="te-swatch" data-te-pick="${role}" aria-label="Choose ${esc(label)} color" title="Choose ${esc(label)} color"><span style="background:${esc(value)}"></span></button>
    <input class="te-value" value="${esc(invalid ?? colorHex(value) ?? value)}" data-role="${role}" spellcheck="false" autocomplete="off" aria-label="${esc(label)} hex value"${invalid === undefined ? "" : ' aria-invalid="true"'}></div>`;
}

function renderThemeEditor() {
  const panel = document.getElementById("theme-editor");
  const ed = themeEditor;
  if (!panel || !ed) return;
  const editing = editingTheme();
  const filter = ed.filter.trim().toLowerCase();
  const groups = ed.advanced
    ? THEME_FAMILIES.map(([group, fields]) => [group, fields.filter(([role, label]) => !filter || [label, ...(FAMILY_PROBES[role] ?? [role]).map(roleLabel)].join(" ").toLowerCase().includes(filter))]).filter(([, f]) => f.length)
    : [["", [["canvas", "Background"], ["accent", "Accent"]]]];
  const fields = groups.length
    ? groups.map(([group, f]) => `<div class="te-group">${group ? `<h4>${esc(group)}</h4>` : ""}${f.map(([role, label]) => themeField(role, label)).join("")}</div>`).join("")
    : `<p class="te-empty">No matches.</p>`;
  const sub = editorSubtitle();
  panel.dataset.minimized = ed.minimized;
  panel.innerHTML = `<header class="te-head"><div class="te-title"><b>${editing ? "Edit theme" : "Create theme"}</b>${ed.minimized ? "" : `<span>${sub}</span>`}</div>
      <button type="button" class="te-inspect${ed.inspecting ? " on" : ""}" data-te="inspect" aria-label="${ed.inspecting ? "Cancel inspecting app colors" : "Inspect app colors"}" title="${ed.inspecting ? "Cancel and clear the selection" : "Pick a color from the app"}">${ed.inspecting ? "Cancel" : "Inspect"}</button>
      <button type="button" class="icon" data-te="minimize" aria-label="${ed.minimized ? "Expand the theme editor" : "Minimize the theme editor"}">${icon(ed.minimized ? "chevron-up" : "chevron-down")}</button>
      <button type="button" class="icon" data-te="close" aria-label="Close the theme editor">${icon("x")}</button></header>
    <div class="te-body">
      <div class="te-row"><label for="theme-name">Theme name</label><input id="theme-name" value="${esc(ed.name)}" placeholder="${editing ? "Theme name" : "e.g. Aurora"}" maxlength="48" autocomplete="off" spellcheck="false"></div>
      ${ed.error ? `<p class="te-error" aria-live="polite">${esc(ed.error)}</p>` : ""}
      <div class="te-row"><span>Appearance</span><span class="te-seg" role="group" aria-label="Theme appearance">${["light", "dark"].map((m) => `<button type="button" data-te-mode="${m}" class="${ed.appearance === m ? "on" : ""}">${m === "light" ? "Light" : "Dark"}</button>`).join("")}</span></div>
      <div class="te-colors-head"><div><b>Colors</b>${ed.advanced ? "" : `<span>Two colors, rest derived</span>`}</div>
        ${ed.advanced ? `<input class="te-filter" placeholder="Filter colors" aria-label="Filter colors" value="${esc(ed.filter)}">` : ""}
        <label class="te-advanced">Advanced <span class="switch"><input type="checkbox" data-te="advanced"${ed.advanced ? " checked" : ""} aria-label="Use advanced theme colors"><span></span></span></label></div>
      ${fields}
    </div>
    <footer class="te-foot"><button type="button" class="ghost" data-te="cancel">Cancel</button><button type="button" class="primary" data-te="save"></button></footer>
    <span class="te-grip" aria-hidden="true"></span>`;
  refreshThemeEditorControls();
  watchUsage();
  if (!colorPicker) return;
  const anchor = panel.querySelector(`[data-te-pick="${colorPicker.role}"]`);
  if (!anchor) return closeColorPicker();
  colorPicker.anchor = anchor;
  syncColorPicker(true);
  placeColorPicker();
}

const editorSubtitle = () => {
  const ed = themeEditor;
  if (ed.inspecting) return "Select an element · Esc to cancel";
  if (!ed.selected) return "Select a color below";
  const n = usageWatch?.count ?? 0;
  return `${ed.advanced ? familyLabel(ed.selected) : roleLabel(ed.selected)} · ${n} ${n === 1 ? "use" : "uses"}`;
};

/** What the name decides, kept in place while typing: the save label and the locked appearances. */
function refreshThemeEditorControls() {
  const panel = document.getElementById("theme-editor");
  const ed = themeEditor;
  const editing = editingTheme();
  const target = mergeTarget();
  // A new theme named after a one-palette theme moves to the free appearance, once per name, as T3.
  const mergeKey = !editing && target ? `${target.id}:${modesOf(target).join(",")}` : null;
  if (mergeKey !== ed.mergeKey) {
    ed.mergeKey = mergeKey;
    const taken = target ? modesOf(target) : [];
    if (mergeKey && taken.length === 1 && taken[0] === ed.appearance) {
      ed.appearance = ed.appearance === "light" ? "dark" : "light";
      previewThemeEditor();
      const typing = document.activeElement?.id === "theme-name";
      renderThemeEditor();
      if (typing) panel.querySelector("#theme-name").focus();
      return;
    }
  }
  const save = panel.querySelector('[data-te="save"]');
  save.innerHTML = editing ? (target ? `Merge into “${esc(target.label)}”` : "Save changes") : target ? `${icon("plus")}Add ${ed.appearance} palette` : `${icon("paintbrush")}Create theme`;
  save.disabled = !ed.name.trim();
  for (const b of panel.querySelectorAll("[data-te-mode]")) {
    const locked = lockedReason(b.dataset.teMode);
    if (locked) Object.assign(b, { title: locked }).setAttribute("aria-disabled", "true");
    else b.removeAttribute("aria-disabled"), b.removeAttribute("title");
  }
}

function setThemeEditorColor(role, value) {
  const ed = themeEditor;
  if (!parseColor(value)) return;
  const mode = ed.appearance;
  if (ed.advanced) {
    ed.colors[mode] = updateColorFamily(ed.colors[mode], role, value, mode);
    ed.regenerate = true;
  } else {
    const colors = { ...ed.colors[mode], [role]: value };
    ed.colors[mode] = vividThemeColors(chatColors(mode), colors.canvas, colors.accent, mode);
  }
  ed.changed = new Set([...(ed.changed ?? []), mode]);
  previewThemeEditor();
}

function bindThemeEditor(panel) {
  panel.addEventListener("click", (e) => {
    const ed = themeEditor;
    const mode = e.target.closest("[data-te-mode]");
    if (mode) {
      if (mode.hasAttribute("aria-disabled")) return;
      ed.appearance = mode.dataset.teMode;
      previewThemeEditor();
      return renderThemeEditor();
    }
    const act = e.target.closest("[data-te]")?.dataset.te;
    if (act === "close" || act === "cancel") return closeThemeEditor();
    if (act === "minimize") {
      ed.minimized = !ed.minimized;
      return renderThemeEditor();
    }
    if (act === "save") return saveThemeEditor();
    if (act === "inspect") return ed.inspecting ? clearInspectorSelection() : setInspecting(true);
    const usage = e.target.closest("[data-te-usage]")?.dataset.teUsage;
    if (usage) {
      ed.selected = ed.selected === usage ? null : usage;
      renderThemeEditor();
      return panel.querySelector(`[data-te-usage="${usage}"]`)?.focus();
    }
    const pick = e.target.closest("[data-te-pick]");
    if (pick) return colorPicker?.role === pick.dataset.tePick ? closeColorPicker() : openColorPicker(pick.dataset.tePick, pick);
  });
  for (const type of ["focusin", "pointerdown"]) {
    panel.addEventListener(type, (e) => {
      const field = e.target.closest(".te-value, .te-swatch")?.closest("[data-theme-color-role]");
      if (field) selectThemeField(field.dataset.themeColorRole);
    });
  }
  panel.addEventListener("input", (e) => {
    const ed = themeEditor;
    if (e.target.id === "theme-name") {
      ed.name = e.target.value;
      ed.error = "";
      panel.querySelector(".te-error")?.remove();
      return refreshThemeEditorControls();
    }
    if (e.target.classList.contains("te-filter")) {
      ed.filter = e.target.value;
      renderThemeEditor();
      const input = panel.querySelector(".te-filter");
      input.focus();
      input.setSelectionRange(input.value.length, input.value.length);
      return;
    }
    const role = e.target.dataset.role;
    if (!role) return;
    const valid = !!parseColor(e.target.value);
    if (valid) delete ed.invalid[ed.appearance][role];
    else ed.invalid[ed.appearance][role] = e.target.value;
    if (valid) e.target.removeAttribute("aria-invalid");
    else e.target.setAttribute("aria-invalid", "true");
    if (valid) setThemeEditorColor(role, e.target.value);
    e.target.closest(".te-field").querySelector(".te-swatch span").style.background = valid ? themeEditor.colors[ed.appearance][role] : "#000000";
    if (colorPicker?.role === role) syncColorPicker(true);
  });
  panel.addEventListener("change", (e) => {
    const ed = themeEditor;
    if (e.target.dataset.te === "advanced") {
      ed.advanced = e.target.checked;
      if (!ed.advanced && !["canvas", "accent"].includes(ed.selected)) ed.selected = null;
      closeColorPicker();
      if (!ed.advanced && ed.regenerate) {
        const editing = editingTheme();
        const modes = editing && modesOf(editing).length > 1 ? ["light", "dark"] : [ed.appearance];
        for (const m of modes) ed.colors[m] = vividThemeColors(chatColors(m), ed.colors[m].canvas, ed.colors[m].accent, m);
        ed.regenerate = false;
        previewThemeEditor();
      }
      return renderThemeEditor();
    }
  });
  panel.addEventListener("pointerdown", (e) => {
    if (!e.target.closest(".te-grip")) return;
    e.preventDefault();
    const r = panel.getBoundingClientRect();
    const move = (ev) => {
      const width = Math.min(Math.max(280, ev.clientX - r.left), innerWidth - r.left - 8);
      const height = Math.min(Math.max(220, ev.clientY - r.top), innerHeight - r.top - 8);
      Object.assign(panel.style, { left: `${r.left}px`, top: `${r.top}px`, right: "auto", bottom: "auto", width: `${width}px`, height: `${height}px`, maxHeight: "none" });
    };
    const up = () => removeEventListener("pointermove", move) || removeEventListener("pointerup", up);
    addEventListener("pointermove", move);
    addEventListener("pointerup", up);
  });
  panel.addEventListener("pointerdown", (e) => {
    if (!e.target.closest(".te-head") || e.target.closest("button, input, a")) return;
    const r = panel.getBoundingClientRect();
    const dx = e.clientX - r.left;
    const dy = e.clientY - r.top;
    const move = (ev) => {
      const left = Math.min(Math.max(8, ev.clientX - dx), innerWidth - r.width - 8);
      const top = Math.min(Math.max(8, ev.clientY - dy), innerHeight - 48);
      Object.assign(panel.style, { left: `${left}px`, top: `${top}px`, right: "auto", bottom: "auto" });
      placeColorPicker();
    };
    const up = () => removeEventListener("pointermove", move) || removeEventListener("pointerup", up);
    addEventListener("pointermove", move);
    addEventListener("pointerup", up);
  });
}

function saveThemeEditor() {
  const ed = themeEditor;
  const name = ed.name.trim();
  if (!name) {
    ed.error = "Name your theme first.";
    return renderThemeEditor();
  }
  const mode = ed.appearance;
  const editing = editingTheme();
  const target = mergeTarget();
  const managed = !ed.advanced;
  try {
    const invalid = Object.entries(ed.invalid[mode])[0];
    if (invalid) throw new Error(`The color for "${invalid[0]}" must be a literal CSS color such as oklch(0.62 0.2 280).`);
    if (target) {
      // An edited theme renamed onto another brings all its palettes; a new one brings the one it shows.
      const modes = editing ? modesOf(editing) : [mode];
      const taken = modes.find((m) => colorsFor(target, m));
      if (taken) throw new Error(editing ? `“${target.label}” already has a ${taken} palette. Pick another name.` : `“${target.label}” already has ${modesOf(target).length === 2 ? "light and dark palettes" : `a ${mode} palette`}. Pick another name.`);
      const merged = { ...target, variants: { ...target.variants, ...Object.fromEntries(modes.map((m) => [m, ed.colors[m]])) } };
      customThemes = customThemes.filter((t) => t.id !== editing?.id).map((t) => (t.id === target.id ? merged : t));
      saveCustomThemes();
      closeThemeEditor();
      setTheme(target.id);
      themeToast({ type: "success", title: `${target.label} updated`, description: `Its ${modes[0]} palette was added.` });
      return;
    }
    if (editing) {
      const updated = { ...editing, label: name, colors: ed.colors[editing.appearance], managed };
      const other = editing.appearance === "light" ? "dark" : "light";
      if (editing.variants?.[other]) updated.variants = { [other]: ed.colors[other] };
      customThemes = customThemes.map((t) => (t.id === editing.id ? updated : t));
      saveCustomThemes();
      const s = themeState();
      const active = s.theme === editing.id || s.halves.light === editing.id || s.halves.dark === editing.id;
      closeThemeEditor();
      themeToast({ type: "success", title: `${name} saved`, description: active ? "Your changes are now active." : "Your changes are saved." });
      return;
    }
    const id = themeIdFromName(name);
    if (RESERVED_THEME_IDS.has(id) || customThemes.some((t) => t.id === id)) throw new Error(`A theme named "${name}" is already installed.`);
    customThemes = [...customThemes, { id, label: name, appearance: mode, colors: ed.colors[mode], ...(managed ? { managed } : {}) }];
    saveCustomThemes();
    closeThemeEditor();
    setTheme(id);
    themeToast({ type: "success", title: `${name} created`, description: "It’s now active." });
  } catch (err) {
    ed.error = err.message || (editing ? "Could not save the theme." : "Could not create the theme.");
    renderThemeEditor();
  }
}

/** themeInspector.ts: the family that paints an element, found by swapping each family's colors for a
 * probe color and seeing which of the element's paints change. A fill wins over a border, a border over
 * text, as T3 orders them. */
const FAMILY_PROBES = {
  canvas: ["canvas"], surface: ["surface"], surfaceRaised: ["surfaceRaised"], surfaceOverlay: ["surfaceOverlay"], text: ["text"],
  mutedForeground: ["mutedForeground", "sidebarMutedForeground"], border: ["border", "sidebarBorder"], input: ["input"],
  accentSurface: ["accentSurface"], accent: ["accent"], messageAction: ["messageAction", "messageActionForeground"],
  messageSurface: ["messageSurface"], codeBackground: ["codeBackground", "codeForeground"],
  searchMatchBackground: ["searchMatchBackground", "searchMatchForeground"], searchMatchActiveBackground: ["searchMatchActiveBackground", "searchMatchActiveForeground"],
  sidebar: ["sidebar", "sidebarForeground"], sidebarControlSurface: ["sidebarControlSurface"], sidebarRowSelected: ["sidebarRowHover", "sidebarRowActive", "sidebarRowSelected"],
  terminalBackground: ["terminalBackground", "terminalForeground"], error: ["error"], warning: ["warning", "warningForeground"],
};
const PROBE_PAINTS = [["backgroundColor", "backgroundImage"], ["borderTopColor", "borderRightColor", "borderBottomColor", "borderLeftColor", "outlineColor", "boxShadow"], ["color", "fill", "stroke"]];
/** Paints each role's variables (and their channels) in the probe color; the return undoes it. */
function probeRoles(roles) {
  const vars = roles.flatMap((r) => [[themeVar(r), "#01fea7"], ...(CHANNEL_ROLES.includes(r) ? [[`${themeVar(r)}-rgb`, "1 254 167"]] : [])]);
  const saved = vars.map(([v]) => themeRoot.style.getPropertyValue(v));
  for (const [v, probe] of vars) themeRoot.style.setProperty(v, probe);
  return () => vars.forEach(([v], i) => themeRoot.style.setProperty(v, saved[i]));
}
function withProbeSession(run) {
  themeRoot.dataset.themeTokenProbe = "";
  try {
    return run();
  } finally {
    void getComputedStyle(themeRoot).color;
    delete themeRoot.dataset.themeTokenProbe;
  }
}
function familyAt(el) {
  const paints = () => {
    const s = getComputedStyle(el);
    return PROBE_PAINTS.map((props) => props.map((p) => s[p]).join("|"));
  };
  const before = paints();
  const found = [null, null, null];
  withProbeSession(() => {
    for (const [family, roles] of Object.entries(FAMILY_PROBES)) {
      const restore = probeRoles(roles);
      paints().forEach((p, i) => (found[i] ??= p !== before[i] ? family : null));
      restore();
    }
  });
  return found.find(Boolean) ?? null;
}
/** ThemeColorPicker.tsx's getThemeRoleLabel. */
const ROLE_LABELS = { canvas: "Background", toolbar: "Toolbar background", toolbarForeground: "Toolbar text", toolbarBorder: "Toolbar border", toolbarControl: "Toolbar control",
  toolbarControlForeground: "Toolbar control text", toolbarControlHover: "Toolbar control hover", accent: "Accent color", errorForeground: "Error text", errorSurface: "Error background",
  warningForeground: "Warning text", warningSurface: "Warning background", updateForeground: "Update text", updateSurface: "Update background" };
const roleLabel = (role) => ROLE_LABELS[role] ?? role.replace(/([A-Z])/g, " $1").replace(/^./, (c) => c.toUpperCase());
const familyLabel = (role) => THEME_FAMILIES.flatMap(([, f]) => f).find(([r]) => r === role)?.[1] ?? role;

let inspectHover = null;
let inspectTimer = 0;
function showInspectHover(el, family) {
  let box = document.getElementById("theme-inspector-hover");
  if (!box) {
    box = Object.assign(document.createElement("div"), { id: "theme-inspector-hover" });
    box.innerHTML = `<span class="label"></span>`;
    document.body.append(box);
  }
  const r = el.getBoundingClientRect();
  Object.assign(box.style, { left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${r.height}px` });
  box.dataset.placement = r.top < 32 ? "below" : "above";
  box.querySelector(".label").textContent = familyLabel(family);
  box.hidden = false;
}
function clearInspectHover() {
  clearTimeout(inspectTimer);
  inspectHover = null;
  const box = document.getElementById("theme-inspector-hover");
  if (box) box.hidden = true;
}
const outsideEditor = (e) => e.target instanceof Element && !e.target.closest("[data-theme-editor-panel]");
const inspecting = () => themeEditor?.inspecting;
document.addEventListener("pointerover", (e) => {
  if (!inspecting() || !outsideEditor(e)) return;
  clearTimeout(inspectTimer);
  inspectTimer = setTimeout(() => {
    const family = familyAt(e.target);
    if (!family) return clearInspectHover();
    inspectHover = { el: e.target, family };
    showInspectHover(e.target, family);
  }, 140);
}, true);
document.addEventListener("pointerdown", (e) => {
  if (!inspecting() || !outsideEditor(e)) return;
  e.preventDefault();
  e.stopPropagation();
  const family = inspectHover?.el === e.target ? inspectHover.family : familyAt(e.target);
  clearInspectHover();
  if (family) selectThemeFamily(family);
}, true);
document.addEventListener("click", (e) => {
  if (!themeEditor?.disarming || !outsideEditor(e)) return;
  e.preventDefault();
  e.stopPropagation();
  themeEditor.disarming = false;
}, true);
document.addEventListener("keydown", (e) => {
  if (!inspecting() || e.key !== "Escape") return;
  e.preventDefault();
  e.stopPropagation();
  clearInspectorSelection();
}, true);

function clearInspectorSelection() {
  themeEditor.selected = null;
  setInspecting(false);
}
function setInspecting(on) {
  themeEditor.inspecting = on;
  if (!on) clearInspectHover();
  renderThemeEditor();
}
/** A pick ends inspecting on the family's field, in Advanced unless it is Background or Accent. */
function selectThemeFamily(role) {
  const ed = themeEditor;
  ed.inspecting = false;
  ed.disarming = true;
  ed.selected = role;
  ed.filter = "";
  if (!["canvas", "accent"].includes(role)) ed.advanced = true;
  renderThemeEditor();
  const field = document.querySelector(`#theme-editor [data-theme-color-role="${role}"]`);
  field?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  field?.querySelector(".te-value")?.focus({ preventScroll: true });
}
/** ThemeColorPicker.tsx's popover: a saturation and brightness plane, a hue slider, and HEX and RGB
 * fields. Drags commit once per frame; the alpha of the original color is kept. */
let colorPicker = null;
function hexToHsv(hex) {
  const n = parseInt(hex.slice(1, 7), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((c) => c / 255);
  const max = Math.max(r, g, b);
  const d = max - Math.min(r, g, b);
  let h = 0;
  if (d) h = (max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4) * 60;
  return { h: h < 0 ? h + 360 : h, s: max ? d / max : 0, v: max };
}
function hsvToHex({ h, s, v }) {
  const hue = ((h % 360) + 360) % 360;
  const c = v * s;
  const x = c * (1 - Math.abs(((hue / 60) % 2) - 1));
  const m = v - c;
  const [r, g, b] = hue < 60 ? [c, x, 0] : hue < 120 ? [x, c, 0] : hue < 180 ? [0, c, x] : hue < 240 ? [0, x, c] : hue < 300 ? [x, 0, c] : [c, 0, x];
  return `#${[r, g, b].map((ch) => Math.round((ch + m) * 255).toString(16).padStart(2, "0")).join("")}`;
}
const pickerHex = (value) => (colorHex(value) ?? "#000000").slice(0, 7);
const rgbText = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(", ");
function rgbTextToHex(text) {
  const ch = text.trim().replace(/^rgb\(\s*/i, "").replace(/\s*\)$/, "").split(/[,\s]+/).filter(Boolean).map(Number);
  return ch.length === 3 && ch.every((c) => Number.isInteger(c) && c >= 0 && c <= 255) ? `#${ch.map((c) => c.toString(16).padStart(2, "0")).join("")}` : null;
}

function openColorPicker(role, anchor) {
  closeColorPicker();
  const label = anchor.closest(".te-field").querySelector(".te-label").textContent;
  const value = themeEditor.colors[themeEditor.appearance][role];
  const full = colorHex(value) ?? "#000000";
  colorPicker = { role, label, anchor, alpha: full.length === 9 && !full.endsWith("ff") ? full.slice(7) : "", hsv: hexToHsv(pickerHex(value)), pending: null, frame: 0 };
  const el = document.createElement("div");
  el.id = "theme-color-picker";
  el.dataset.themeEditorPanel = "";
  el.setAttribute("role", "dialog");
  el.setAttribute("aria-label", `${label} color`);
  el.innerHTML = `<div class="cp-head"><div><p class="cp-title">${esc(label)}</p><p class="cp-sub">Choose a color</p></div><span class="cp-dot"></span></div>
    <div class="cp-body">
      <div class="cp-plane" role="group" aria-label="${esc(label)} saturation and brightness">
        <input type="range" class="cp-axis" data-axis="s" min="0" max="100" step="any" aria-label="${esc(label)} saturation">
        <input type="range" class="cp-axis" data-axis="v" min="0" max="100" step="any" aria-label="${esc(label)} brightness">
        <span class="cp-thumb"></span></div>
      <div class="cp-hue" role="slider" tabindex="0" aria-label="${esc(label)} hue" aria-valuemin="0" aria-valuemax="360"><span class="cp-track"></span><span class="cp-thumb"></span></div>
      <div class="cp-fields"><label><span>HEX</span><span class="cp-input"><i></i><input data-cp="hex" aria-label="${esc(label)} picker hex value" spellcheck="false" autocomplete="off"></span></label>
        <label><span>RGB</span><span class="cp-input"><input data-cp="rgb" aria-label="${esc(label)} picker RGB value" spellcheck="false" autocomplete="off"></span></label></div></div>`;
  document.body.append(el);
  bindColorPicker(el);
  syncColorPicker(true);
  placeColorPicker();
}
function closeColorPicker() {
  if (!colorPicker) return;
  flushColorPicker();
  document.getElementById("theme-color-picker")?.remove();
  colorPicker = null;
}
/** Below the swatch, its right edge on the swatch's, flipped above when there is no room. */
function placeColorPicker() {
  const el = document.getElementById("theme-color-picker");
  const a = colorPicker?.anchor;
  if (!el || !a?.isConnected) return;
  const r = a.getBoundingClientRect();
  const w = el.offsetWidth;
  const h = el.offsetHeight;
  const left = Math.min(Math.max(8, r.right - w), innerWidth - w - 8);
  const top = r.bottom + 10 + h <= innerHeight - 8 ? r.bottom + 10 : Math.max(8, r.top - 10 - h);
  Object.assign(el.style, { left: `${left}px`, top: `${top}px` });
}
/** Paints the picker from its HSV, and the text fields from it unless one is being typed in. */
function syncColorPicker(fromValue = false) {
  const el = document.getElementById("theme-color-picker");
  if (!el || !colorPicker) return;
  const p = colorPicker;
  if (fromValue) {
    const hex = pickerHex(themeEditor.colors[themeEditor.appearance][p.role]);
    if (hsvToHex(p.hsv) !== hex) p.hsv = hexToHsv(hex);
  }
  const hex = hsvToHex(p.hsv);
  el.querySelector(".cp-dot").style.background = hex;
  el.querySelector(".cp-input i").style.background = hex;
  const plane = el.querySelector(".cp-plane");
  plane.style.backgroundColor = `hsl(${p.hsv.h} 100% 50%)`;
  Object.assign(plane.querySelector(".cp-thumb").style, { left: `calc(${p.hsv.s} * (100% - 12px) + 6px)`, top: `calc(${1 - p.hsv.v} * (100% - 12px) + 6px)` });
  for (const axis of plane.querySelectorAll(".cp-axis")) {
    axis.value = p.hsv[axis.dataset.axis] * 100;
    axis.setAttribute("aria-valuetext", `${Math.round(p.hsv[axis.dataset.axis] * 100)}%`);
  }
  const hue = el.querySelector(".cp-hue");
  hue.setAttribute("aria-valuenow", Math.round(p.hsv.h));
  Object.assign(hue.querySelector(".cp-thumb").style, { left: `calc(${p.hsv.h / 360} * (100% - 16px) + 8px)`, background: `hsl(${p.hsv.h} 100% 50%)` });
  for (const input of el.querySelectorAll("[data-cp]")) if (document.activeElement !== input) input.value = input.dataset.cp === "hex" ? hex : rgbText(hex);
}
function commitPickerColor(color, now = false) {
  const p = colorPicker;
  p.pending = color;
  if (now) return flushColorPicker();
  if (!p.frame) p.frame = requestAnimationFrame(flushColorPicker);
}
function flushColorPicker() {
  const p = colorPicker;
  if (!p) return;
  cancelAnimationFrame(p.frame);
  p.frame = 0;
  if (p.pending === null || !themeEditor) return;
  const color = p.pending;
  p.pending = null;
  setThemeEditorColor(p.role, color);
  const field = document.querySelector(`#theme-editor [data-theme-color-role="${p.role}"]`);
  if (!field) return;
  const value = themeEditor.colors[themeEditor.appearance][p.role];
  field.querySelector(".te-swatch span").style.background = value;
  field.querySelector(".te-value").value = colorHex(value) ?? value;
}
function setPickerHsv(hsv) {
  colorPicker.hsv = hsv;
  syncColorPicker();
  commitPickerColor(hsvToHex(hsv) + colorPicker.alpha);
}
function bindColorPicker(el) {
  const clamp = (n) => Math.min(1, Math.max(0, n));
  const drag = (target, update) => {
    target.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      (target.querySelector(".cp-axis") ?? target).focus({ preventScroll: true });
      target.setPointerCapture(e.pointerId);
      update(e);
      const move = (ev) => update(ev);
      const end = () => {
        target.removeEventListener("pointermove", move);
        target.removeEventListener("lostpointercapture", end);
        flushColorPicker();
      };
      target.addEventListener("pointermove", move);
      target.addEventListener("lostpointercapture", end);
    });
  };
  const plane = el.querySelector(".cp-plane");
  drag(plane, (e) => {
    const r = plane.getBoundingClientRect();
    setPickerHsv({ ...colorPicker.hsv, s: clamp((e.clientX - r.left) / r.width), v: 1 - clamp((e.clientY - r.top) / r.height) });
  });
  const hue = el.querySelector(".cp-hue");
  drag(hue, (e) => {
    const r = hue.getBoundingClientRect();
    setPickerHsv({ ...colorPicker.hsv, h: clamp((e.clientX - r.left) / r.width) * 360 });
  });
  for (const axis of plane.querySelectorAll(".cp-axis")) {
    axis.addEventListener("keydown", (e) => {
      const k = axis.dataset.axis;
      const step = e.shiftKey ? 0.1 : 0.02;
      const now = colorPicker.hsv[k];
      const next = { ArrowRight: now + step, ArrowUp: now + step, ArrowLeft: now - step, ArrowDown: now - step, Home: 0, End: 1 }[e.key];
      if (next === undefined) return;
      e.preventDefault();
      setPickerHsv({ ...colorPicker.hsv, [k]: clamp(next) });
    });
  }
  hue.addEventListener("keydown", (e) => {
    if (!["ArrowDown", "ArrowLeft", "ArrowRight", "ArrowUp"].includes(e.key)) return;
    e.preventDefault();
    const dir = e.key === "ArrowRight" || e.key === "ArrowUp" ? 1 : -1;
    setPickerHsv({ ...colorPicker.hsv, h: (colorPicker.hsv.h + dir * (e.shiftKey ? 10 : 1) + 360) % 360 });
  });
  el.addEventListener("input", (e) => {
    const kind = e.target.dataset.cp;
    if (!kind) return;
    const hex = kind === "hex" ? (/^#[0-9a-f]{6}$/i.test(e.target.value) ? e.target.value.toLowerCase() : null) : rgbTextToHex(e.target.value);
    if (!hex) return;
    colorPicker.hsv = hexToHsv(hex);
    syncColorPicker();
    commitPickerColor(kind === "hex" ? hex : hex + colorPicker.alpha, true);
  });
  el.addEventListener("focusout", (e) => e.target.dataset.cp && syncColorPicker());
}
document.addEventListener("pointerdown", (e) => {
  if (!colorPicker || !(e.target instanceof Element)) return;
  if (e.target.closest("#theme-color-picker") || e.target.closest("[data-te-pick]") === colorPicker.anchor) return;
  closeColorPicker();
}, true);
document.addEventListener("keydown", (e) => {
  if (!colorPicker || e.key !== "Escape") return;
  e.preventDefault();
  e.stopPropagation();
  const anchor = colorPicker.anchor;
  closeColorPicker();
  anchor.focus();
}, true);
addEventListener("resize", () => placeColorPicker());
addEventListener("scroll", () => placeColorPicker(), true);

/** Using a field's swatch or hex value selects it, as T3's onSelect: Advanced colors only show there. */
function selectThemeField(role) {
  const ed = themeEditor;
  if (ed.selected === role) return;
  ed.selected = role;
  const panel = document.getElementById("theme-editor");
  for (const f of panel.querySelectorAll(".te-field")) {
    const on = f.dataset.themeColorRole === role;
    f.classList.toggle("selected", on);
    const label = f.querySelector(".te-label");
    label.setAttribute("aria-pressed", on);
    label.setAttribute("aria-label", `${on ? "Hide" : "Show"} ${label.textContent} usage`);
    label.title = `${on ? "Hide" : "Show"} where ${label.textContent} is used`;
  }
  watchUsage();
}

/** themeInspector.ts's highlightThemeRoleUsage: every element whose paint changes when the selected
 * color's roles take the probe color, cut out of a dimmed overlay. In Advanced a field stands for its
 * family's roles; Background and Accent stand for every role that holds the same color. */
let usageWatch = null;
function usageRoles() {
  const ed = themeEditor;
  if (!ed?.selected || ed.inspecting) return [];
  if (ed.advanced) return FAMILY_PROBES[ed.selected] ?? [ed.selected];
  const colors = ed.colors[ed.appearance];
  const value = colors[ed.selected]?.trim().toLowerCase();
  return THEME_PALETTES.roles.filter((r) => colors[r]?.trim().toLowerCase() === value);
}
function watchUsage() {
  const roles = usageRoles();
  const key = roles.join(",");
  if (usageWatch?.key === key) return showUsageCount();
  stopUsageWatch();
  if (!roles.length) return;
  const watch = { key, count: 0, timer: 0, frame: 0, last: 0 };
  usageWatch = watch;
  const refresh = () => {
    watch.timer = 0;
    watch.last = performance.now();
    watch.count = highlightUsage(roles);
    showUsageCount();
  };
  refresh();
  watch.observer = new MutationObserver((mutations) => {
    if (mutations.every((m) => m.target instanceof Element && m.target.closest("#theme-inspector-spotlight, [data-theme-editor-panel]"))) return;
    if (!watch.timer) watch.timer = setTimeout(refresh, Math.max(0, 500 - (performance.now() - watch.last)));
  });
  watch.observer.observe(document.body, { childList: true, subtree: true });
  watch.reflow = () => {
    if (!watch.frame) watch.frame = requestAnimationFrame(() => ((watch.frame = 0), drawSpotlight([...document.querySelectorAll("[data-theme-inspector-match]")])));
  };
  addEventListener("resize", watch.reflow);
  addEventListener("scroll", watch.reflow, true);
}
function stopUsageWatch() {
  if (!usageWatch) return;
  usageWatch.observer?.disconnect();
  clearTimeout(usageWatch.timer);
  cancelAnimationFrame(usageWatch.frame);
  removeEventListener("resize", usageWatch.reflow);
  removeEventListener("scroll", usageWatch.reflow, true);
  usageWatch = null;
  for (const el of document.querySelectorAll("[data-theme-inspector-match]")) el.removeAttribute("data-theme-inspector-match");
  document.getElementById("theme-inspector-spotlight")?.remove();
}
function showUsageCount() {
  const sub = document.querySelector("#theme-editor .te-title span");
  if (sub) sub.textContent = editorSubtitle();
}

function paintSnapshot(el) {
  const s = getComputedStyle(el);
  if (s.display === "none" || s.visibility === "hidden" || Number(s.opacity) === 0) return null;
  const borders = [["Top"], ["Right"], ["Bottom"], ["Left"]].filter(([side]) => s[`border${side}Style`] !== "none" && parseFloat(s[`border${side}Width`]) > 0).map(([side]) => s[`border${side}Color`]);
  if (s.outlineStyle !== "none" && parseFloat(s.outlineWidth) > 0) borders.push(s.outlineColor);
  if (s.boxShadow !== "none") borders.push(s.boxShadow);
  const text = el.matches("input, textarea, select, option") || [...el.childNodes].some((n) => n.nodeType === Node.TEXT_NODE && n.textContent.trim());
  const fg = [text ? s.color : "", el instanceof SVGElement ? `${s.fill}|${s.stroke}` : "", el.matches("input, textarea") ? s.caretColor : "", s.textDecorationLine !== "none" ? s.textDecorationColor : ""];
  return `${s.backgroundColor}|${s.backgroundImage}\n${borders.join("|")}\n${fg.join("|")}`;
}
function highlightUsage(roles) {
  for (const el of document.querySelectorAll("[data-theme-inspector-match]")) el.removeAttribute("data-theme-inspector-match");
  const candidates = [document.body, ...document.body.querySelectorAll("*")].filter((el) => !el.closest("[data-theme-editor-panel], #theme-inspector-spotlight, #theme-inspector-hover"));
  const changed = withProbeSession(() => {
    const before = new Map();
    for (const el of candidates) {
      const p = paintSnapshot(el);
      if (p) before.set(el, p);
    }
    const restore = probeRoles(roles);
    try {
      return [...before].filter(([el, p]) => paintSnapshot(el) !== p).map(([el]) => el);
    } finally {
      restore();
    }
  });
  const marked = new Set(changed.map((el) => (el instanceof SVGElement ? el.closest("svg") ?? el : el)));
  for (const el of marked) el.setAttribute("data-theme-inspector-match", "");
  drawSpotlight([...marked]);
  return marked.size;
}
function spotlightRect(el) {
  const r = el.getBoundingClientRect();
  if (r.width <= 0 || r.height <= 0 || r.right < 0 || r.bottom < 0 || r.left > innerWidth || r.top > innerHeight) return null;
  const pad = 5;
  return { x: r.left - pad, y: r.top - pad, width: r.width + pad * 2, height: r.height + pad * 2, radius: Math.min(18, Math.max(7, (parseFloat(getComputedStyle(el).borderTopLeftRadius) || 0) + pad)) };
}
function drawSpotlight(els) {
  const rects = new Map();
  for (const el of els) {
    const r = spotlightRect(el);
    if (r) rects.set([r.x, r.y, r.width, r.height].map(Math.round).join(":"), r);
  }
  let svg = document.getElementById("theme-inspector-spotlight");
  if (!rects.size) return svg?.remove();
  if (!svg) {
    svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.id = "theme-inspector-spotlight";
    svg.setAttribute("aria-hidden", "true");
    document.body.append(svg);
  }
  svg.setAttribute("viewBox", `0 0 ${innerWidth} ${innerHeight}`);
  const box = (r) => `x="${r.x}" y="${r.y}" width="${r.width}" height="${r.height}" rx="${r.radius}"`;
  const all = [...rects.values()];
  svg.innerHTML = `<defs><mask id="theme-inspector-spotlight-mask" maskUnits="userSpaceOnUse"><rect width="${innerWidth}" height="${innerHeight}" fill="white"/>${all.map((r) => `<rect ${box(r)} fill="black"/>`).join("")}</mask>
    <filter id="theme-inspector-spotlight-glow" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="5" result="blur"/><feMerge><feMergeNode in="blur"/><feMergeNode in="SourceGraphic"/></feMerge></filter></defs>
    <rect class="dimmer" width="${innerWidth}" height="${innerHeight}" mask="url(#theme-inspector-spotlight-mask)"/>
    <g>${all.map((r) => `<rect class="glow" ${box(r)} filter="url(#theme-inspector-spotlight-glow)"/>`).join("")}</g>`;
}

applyTheme();
applyContrast();
