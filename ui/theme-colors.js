/** Color math for themes, ported from T3 Code's themePalette.ts: parsing, OKLCH, contrast, and the
 * two generators its theme editor uses, `vividThemeColors` (a whole palette from a background and
 * an accent) and `updateColorFamily` (one edited role and the roles derived from it). */

const LIGHT_FG = { r: 255, g: 250, b: 255 };
const DARK_FG = { r: 36, g: 21, b: 35 };
const WHITE = { r: 255, g: 255, b: 255 };
const BLACK = { r: 0, g: 0, b: 0 };

const toLinear = (c) => ((c /= 255) <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const fromLinear = (c) => Math.round(Math.min(1, Math.max(0, c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055)) * 255);

function rgbToOklch({ r, g, b }) {
  [r, g, b] = [r, g, b].map(toLinear);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return { L, C: Math.hypot(a, bb), h: (Math.atan2(bb, a) * 180) / Math.PI };
}

function oklchToLinear({ L, C, h }) {
  const a = C * Math.cos((h * Math.PI) / 180);
  const bb = C * Math.sin((h * Math.PI) / 180);
  const l = (L + 0.3963377774 * a + 0.2158037573 * bb) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * bb) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * bb) ** 3;
  return [4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s, -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s, -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s];
}

function inGamut(color) {
  const fits = (C) => oklchToLinear({ ...color, C }).every((c) => c >= -0.0001 && c <= 1.0001);
  if (fits(color.C)) return color;
  let low = 0;
  let high = color.C;
  for (let i = 0; i < 20; i++) {
    const mid = (low + high) / 2;
    if (fits(mid)) low = mid;
    else high = mid;
  }
  return { ...color, C: low };
}

function oklchToRgb(color) {
  const [r, g, b] = oklchToLinear(inGamut(color)).map(fromLinear);
  return { r, g, b };
}

/** `#rgb`, `#rrggbb(aa)`, `rgb()` and `oklch()`, as { color: OKLCH, alpha }, or null. */
function parseColor(value) {
  if (typeof value !== "string") return null;
  const v = value.trim().toLowerCase();
  let m = v.match(/^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/);
  if (m) {
    let h = m[1];
    if (h.length <= 4) h = [...h].map((c) => c + c).join("");
    const n = (i) => parseInt(h.slice(i, i + 2), 16);
    return { color: rgbToOklch({ r: n(0), g: n(2), b: n(4) }), alpha: h.length === 8 ? n(6) / 255 : 1 };
  }
  m = v.match(/^(rgba?|oklch)\(([^)]*)\)$/);
  if (!m) return null;
  const parts = m[2].split(/[\s,/]+/).filter(Boolean);
  if (parts.length < 3) return null;
  const num = (p, scale = 1) => (p.endsWith("%") ? (parseFloat(p) / 100) * scale : parseFloat(p));
  const alpha = parts[3] === undefined ? 1 : Math.min(1, Math.max(0, num(parts[3])));
  let color;
  if (m[1] === "oklch") color = { L: Math.min(1, Math.max(0, num(parts[0]))), C: Math.max(0, num(parts[1], 0.4)), h: parseFloat(parts[2]) || 0 };
  else color = rgbToOklch({ r: num(parts[0], 255), g: num(parts[1], 255), b: num(parts[2], 255) });
  return [color.L, color.C, color.h, alpha].every(Number.isFinite) ? { color, alpha } : null;
}

const fmt = (n, p) => (Math.abs(n) < 10 ** -p / 2 ? 0 : n).toFixed(p).replace(/(?:\.0+|(\.\d*?)0+)$/, "$1");
function oklchString(color, alpha = 1) {
  const h = color.C < 0.0000005 ? 0 : ((color.h % 360) + 360) % 360;
  const body = `${fmt(color.L, 6)} ${fmt(color.C, 6)} ${fmt(h, 3)}`;
  return alpha < 1 ? `oklch(${body} / ${fmt(alpha, 4)})` : `oklch(${body})`;
}
const canonicalColor = (value) => {
  const p = parseColor(value);
  return p ? oklchString(p.color, p.alpha) : null;
};
const rgbHex = ({ r, g, b }) => `#${[r, g, b].map((c) => Math.round(Math.min(255, Math.max(0, c))).toString(16).padStart(2, "0")).join("")}`;
function colorHex(value) {
  const p = parseColor(value);
  if (!p) return null;
  const hex = rgbHex(oklchToRgb(p.color));
  return p.alpha >= 1 ? hex : hex + Math.round(p.alpha * 255).toString(16).padStart(2, "0");
}
const rgbOf = (value, fallback) => {
  const p = parseColor(value);
  return p ? oklchToRgb(p.color) : fallback;
};
const rgbColor = (rgb) => oklchString(rgbToOklch(rgb));
const oklchColor = (color) => oklchString(inGamut(color));
const mixRgb = (base, over, t) => ({ r: base.r + (over.r - base.r) * t, g: base.g + (over.g - base.g) * t, b: base.b + (over.b - base.b) * t });

function luminance({ r, g, b }) {
  const lin = (c) => ((c /= 255) <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}
function contrast(a, b) {
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}
const isDarkRgb = (rgb) => luminance(rgb) < 0.179;

/** The lightness, moved only lighter or darker, that reaches `min` contrast against `against`. */
function solveLightness(base, against, min, lighter) {
  let low = lighter ? base.L : 0;
  let high = lighter ? 1 : base.L;
  if (contrast(oklchToRgb(base), against) >= min) return base;
  for (let i = 0; i < 18; i++) {
    const mid = (low + high) / 2;
    const ok = contrast(oklchToRgb({ ...base, L: mid }), against) >= min;
    if (ok === lighter) high = mid;
    else low = mid;
  }
  return { ...base, L: lighter ? high : low };
}

function readableForeground(bg) {
  const light = contrast(bg, LIGHT_FG);
  const dark = contrast(bg, DARK_FG);
  if (Math.max(light, dark) >= 4.5) return light >= dark ? LIGHT_FG : DARK_FG;
  return contrast(bg, WHITE) >= contrast(bg, BLACK) ? WHITE : BLACK;
}

function readableText(bg, fg, amount, min) {
  const softened = mixRgb(fg, bg, amount);
  if (contrast(softened, bg) >= min) return softened;
  let readable = fg;
  let low = 0;
  let high = amount;
  for (let i = 0; i < 12; i++) {
    const t = (low + high) / 2;
    const candidate = mixRgb(fg, bg, t);
    if (contrast(candidate, bg) >= min) [readable, low] = [candidate, t];
    else high = t;
  }
  return readable;
}
const mutedText = (bg, fg) => readableText(bg, fg, 1, isDarkRgb(bg) ? 5.082 : 4.705);

const STATUS_COLORS = {
  light: { error: "#fb2c36", errorForeground: "#c10007", warning: "#fe9a00", warningForeground: "#bb4d00" },
  dark: { error: "#fb414a", errorForeground: "#ff6467", warning: "#fe9a00", warningForeground: "#ffb900" },
};
function statusColors(canvas) {
  const dark = isDarkRgb(canvas);
  const standard = STATUS_COLORS[dark ? "dark" : "light"];
  const surfaceOf = (v) => mixRgb(canvas, rgbOf(v, canvas), dark ? 0.16 : 0.08);
  const on = (fg, surface) => oklchColor(solveLightness(rgbToOklch(rgbOf(fg, canvas)), surface, 4.6, dark));
  const errorSurface = surfaceOf(standard.error);
  const warningSurface = surfaceOf(standard.warning);
  return {
    error: canonicalColor(standard.error),
    errorForeground: on(standard.errorForeground, errorSurface),
    errorSurface: rgbColor(errorSurface),
    warning: canonicalColor(standard.warning),
    warningForeground: on(standard.warningForeground, warningSurface),
    warningSurface: rgbColor(warningSurface),
  };
}

/** T3's createVividThemeColors: every role from a background and an accent. `defaults` fills any role
 * the generator doesn't set (T3 uses the T3 Chat palette). */
function vividThemeColors(defaults, backgroundValue, accentValue, appearance) {
  const canvasRgb = rgbOf(backgroundValue, appearance === "dark" ? { r: 24, g: 15, b: 27 } : { r: 250, g: 245, b: 250 });
  const accentRgb = rgbOf(accentValue, { r: 168, g: 67, b: 112 });
  const canvas = rgbToOklch(canvasRgb);
  const accent = rgbToOklch(accentRgb);
  const dark = isDarkRgb(canvasRgb);
  const hue = accent.C < 0.02 ? canvas.h : accent.h;
  const tintC = Math.min(0.045, Math.max(0.008, accent.C * 0.22));
  const step = dark ? 1 : -1;
  const at = (dL, C = tintC) => ({ L: Math.min(0.98, Math.max(0.05, canvas.L + step * dL)), C, h: hue });
  const dl = (d, l) => (dark ? d : l);
  const aC = (k, max) => Math.min(max, accent.C * k);

  const textBase = { L: dark ? 0.95 : 0.2, C: Math.min(0.035, accent.C * 0.25), h: hue };
  const textRgb = oklchToRgb(solveLightness(textBase, canvasRgb, 7, dark));
  const textMutedRgb = mutedText(canvasRgb, textRgb);
  const action = { L: Math.min(0.85, Math.max(0.35, accent.L + (dark ? 0.06 : -0.02))), C: Math.max(accent.C * 0.9, 0.06), h: (hue + 50) % 360 };
  const actionRgb = oklchToRgb(action);
  const sidebar = at(0.045, tintC * 1.4);
  const surfaceRaised = at(0.05);
  const secondary = at(dl(0.1, 0.06), aC(0.5, 0.09));
  const muted = at(dl(0.06, 0.04), aC(0.35, 0.06));
  const accentSurface = at(dl(0.13, 0.08), aC(0.55, 0.11));
  const messageSurface = at(dl(0.16, 0.1), aC(0.6, 0.13));
  const updateSurface = at(dl(0.14, 0.09), aC(0.55, 0.12));
  const foregroundOn = (surface) => oklchColor(solveLightness(textBase, oklchToRgb(surface), 4.6, dark));
  const searchMatch = oklchToRgb(at(0.22, aC(1, 0.12)));
  const searchMatchActive = oklchToRgb(at(0.4, aC(1, 0.18)));
  const text = rgbColor(textRgb);
  const textMuted = rgbColor(textMutedRgb);
  const canvasColor = rgbColor(canvasRgb);
  const accentColor = rgbColor(accentRgb);
  return {
    ...defaults,
    ...statusColors(canvasRgb),
    canvas: canvasColor,
    chrome: canvasColor,
    toolbar: canvasColor,
    toolbarForeground: text,
    toolbarBorder: oklchColor(at(dl(0.14, 0.1), aC(0.4, 0.08))),
    toolbarControl: oklchColor(at(dl(0.09, 0.05), tintC * 1.3)),
    toolbarControlForeground: text,
    toolbarControlHover: oklchColor(at(dl(0.14, 0.09), tintC * 1.6)),
    surface: oklchColor(at(0.015)),
    surfaceRaised: oklchColor(surfaceRaised),
    surfaceOverlay: oklchColor(at(0.075)),
    text,
    textMuted,
    border: oklchColor(at(dl(0.16, 0.12), aC(0.35, 0.07))),
    input: oklchColor(at(dl(0.21, 0.16), aC(0.4, 0.08))),
    focus: accentColor,
    accent: accentColor,
    accentForeground: rgbColor(readableForeground(accentRgb)),
    secondary: oklchColor(secondary),
    secondaryForeground: foregroundOn(secondary),
    muted: oklchColor(muted),
    mutedForeground: rgbColor(readableText(oklchToRgb(muted), textRgb, 1, 4.6)),
    placeholder: rgbColor(readableText(oklchToRgb(surfaceRaised), textRgb, 1, 4.6)),
    secondaryLabel: textMuted,
    iconMuted: textMuted,
    update: accentColor,
    updateForeground: foregroundOn(updateSurface),
    updateSurface: oklchColor(updateSurface),
    accentSurface: oklchColor(accentSurface),
    accentSurfaceForeground: foregroundOn(accentSurface),
    messageSurface: oklchColor(messageSurface),
    messageForeground: foregroundOn(messageSurface),
    messageAction: rgbColor(actionRgb),
    messageActionForeground: rgbColor(readableForeground(actionRgb)),
    messageActionHover: oklchColor({ ...action, L: action.L + (dark ? 0.06 : -0.06) }),
    codeBackground: oklchColor(at(0.035, tintC * 0.8)),
    codeForeground: text,
    searchMatchBackground: rgbColor(searchMatch),
    searchMatchForeground: rgbColor(readableForeground(searchMatch)),
    searchMatchActiveBackground: rgbColor(searchMatchActive),
    searchMatchActiveForeground: rgbColor(readableForeground(searchMatchActive)),
    sidebar: oklchColor(sidebar),
    sidebarForeground: foregroundOn(sidebar),
    sidebarMutedForeground: rgbColor(mutedText(oklchToRgb(sidebar), textRgb)),
    sidebarControlSurface: oklchColor(at(dl(0.1, 0.07), tintC * 1.5)),
    sidebarRowHover: oklchColor(at(dl(0.08, 0.06), aC(0.45, 0.08))),
    sidebarRowActive: oklchColor(at(dl(0.12, 0.09), aC(0.55, 0.1))),
    sidebarRowSelected: oklchColor(at(dl(0.14, 0.1), aC(0.6, 0.11))),
    sidebarBorder: oklchColor(at(dl(0.17, 0.12), aC(0.4, 0.08))),
    terminalBackground: canvasColor,
    terminalForeground: text,
    terminalCursor: accentColor,
    terminalSelection: oklchColor(at(dl(0.18, 0.12), aC(0.55, 0.12))),
    terminalScrollbar: oklchColor(at(dl(0.22, 0.16))),
    terminalScrollbarHover: oklchColor(at(dl(0.3, 0.22))),
  };
}

/** T3's updateThemeColorFamily: sets `role` and recomputes the roles that follow from it. */
function updateColorFamily(colors, role, value, appearance) {
  const parsed = parseColor(value);
  if (!parsed) return { ...colors, [role]: value };
  const v = oklchString(parsed.color, parsed.alpha);
  const canvas = rgbOf(colors.canvas, appearance === "dark" ? { r: 24, g: 15, b: 27 } : { r: 250, g: 245, b: 250 });
  const selected = oklchToRgb(parsed.color);
  const on = (bg) => mixRgb(bg, selected, parsed.alpha);
  const onCanvas = on(canvas);
  const accent = rgbOf(colors.accent, { r: 168, g: 67, b: 112 });
  const canvasDark = isDarkRgb(canvas);
  const fgOn = (bg) => rgbColor(readableForeground(bg));
  const toneOn = (bg) => oklchColor(solveLightness(parsed.color, bg, 4.6, isDarkRgb(bg)));
  const status = () => {
    const surface = mixRgb(canvas, onCanvas, canvasDark ? 0.16 : 0.08);
    return { foreground: toneOn(surface), surface: rgbColor(surface) };
  };
  switch (role) {
    case "canvas":
      return { ...colors, canvas: v, chrome: v, toolbar: v };
    case "text":
      return { ...colors, text: v, toolbarForeground: v, toolbarControlForeground: v };
    case "mutedForeground":
      return { ...colors, textMuted: v, mutedForeground: v, placeholder: v, secondaryLabel: v, iconMuted: v, sidebarMutedForeground: v };
    case "border":
      return { ...colors, border: v, toolbarBorder: v, sidebarBorder: v };
    case "secondary":
      return { ...colors, secondary: v, secondaryForeground: fgOn(onCanvas), muted: v, toolbarControl: v };
    case "accentSurface":
      return { ...colors, accentSurface: v, accentSurfaceForeground: fgOn(onCanvas), toolbarControlHover: v };
    case "accent": {
      const updateSurface = mixRgb(canvas, onCanvas, canvasDark ? 0.32 : 0.16);
      return { ...colors, accent: v, accentForeground: fgOn(onCanvas), focus: v, update: v, updateForeground: toneOn(updateSurface), updateSurface: rgbColor(updateSurface), terminalCursor: v };
    }
    case "messageAction": {
      const fg = readableForeground(onCanvas);
      const toward = fg === LIGHT_FG || fg === WHITE ? BLACK : WHITE;
      return { ...colors, messageAction: v, messageActionForeground: rgbColor(fg), messageActionHover: oklchString(rgbToOklch(mixRgb(selected, toward, 0.12)), parsed.alpha) };
    }
    case "messageSurface":
      return { ...colors, messageSurface: v, messageForeground: fgOn(onCanvas) };
    case "searchMatchBackground":
      return { ...colors, searchMatchBackground: rgbColor(onCanvas), searchMatchForeground: fgOn(onCanvas) };
    case "searchMatchActiveBackground":
      return { ...colors, searchMatchActiveBackground: rgbColor(onCanvas), searchMatchActiveForeground: fgOn(onCanvas) };
    case "codeBackground":
      return { ...colors, codeBackground: v, codeForeground: fgOn(onCanvas) };
    case "sidebar":
      return { ...colors, sidebar: v, sidebarForeground: fgOn(onCanvas) };
    case "sidebarRowSelected": {
      const sidebar = rgbOf(colors.sidebar, canvas);
      const onSidebar = on(sidebar);
      return { ...colors, sidebarRowHover: rgbColor(mixRgb(sidebar, onSidebar, 0.5)), sidebarRowActive: rgbColor(mixRgb(sidebar, onSidebar, 0.8)), sidebarRowSelected: v };
    }
    case "terminalBackground": {
      const fg = readableForeground(onCanvas);
      const dark = isDarkRgb(onCanvas);
      return {
        ...colors,
        terminalBackground: v,
        terminalForeground: rgbColor(fg),
        terminalSelection: rgbColor(mixRgb(onCanvas, accent, dark ? 0.35 : 0.18)),
        terminalScrollbar: rgbColor(mixRgb(onCanvas, fg, dark ? 0.42 : 0.22)),
        terminalScrollbarHover: rgbColor(mixRgb(onCanvas, fg, dark ? 0.55 : 0.32)),
      };
    }
    case "error":
    case "warning": {
      const s = status();
      return { ...colors, [role]: v, [`${role}Foreground`]: s.foreground, [`${role}Surface`]: s.surface };
    }
    default:
      return { ...colors, [role]: v };
  }
}
