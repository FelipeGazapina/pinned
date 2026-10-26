/** T3 Code's Appearance settings (AppearanceSettingsPanel.tsx, TypographySettings.tsx): the Interface
 * rows (contrast, glass opacity, diff colors, chat width), Motion, and typography: font families and
 * sizes for the interface, the prompt, code and the terminal. Each row resets to its default. */
const SANS = '-apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif';
const MONO = '"SF Mono", "SFMono-Regular", Menlo, Consolas, "Liberation Mono", monospace';
const range = (from, to) => Array.from({ length: to - from + 1 }, (_, i) => from + i);
const diffDots = (scheme) => `<span class="diff-dots${scheme === "blue-orange" ? " flip" : ""}" aria-hidden="true"><i class="del"></i><i class="add"></i></span>`;
const LOOKS = {
  "diff-colors": { id: "diff-color-scheme", label: "Diff colors", about: "Choose colors for additions and deletions, including change counts.",
    options: [["", `${diffDots("")}Red &amp; green`, "Red &amp; green (default)"], ["blue-orange", `${diffDots("blue-orange")}Blue &amp; orange`, "Blue &amp; orange"]] },
  "chat-width": { id: "chat-width", label: "Chat width", about: "Set how wide messages and the composer can grow on large screens.",
    options: [["", "Comfortable", "Comfortable (default)"], ["wide", "Wide"], ["full", "Full"]] },
};
/** Each font row: its family and size settings, the sizes it offers and its default size. */
const FONTS = [
  { name: "interface", label: "Interface font", about: "Everything outside code blocks and the terminal.", family: "sans-font", size: "ui-size", sizes: range(12, 20), base: 16 },
  { name: "prompt", label: "Prompt font", about: "Only the box you write prompts in. Mono works well here.", family: "prompt-font", size: "prompt-size", sizes: range(12, 20), base: 14, advanced: true },
  { name: "code", label: "Code font", simpleLabel: "Monospace font", about: "Code blocks, diffs, and file previews.", simpleAbout: "Code blocks, diffs, file previews, and the terminal.", family: "mono-font", size: "code-size", sizes: range(10, 18), base: 13, mono: true },
  { name: "terminal", label: "Terminal font", about: "Terminal output, independent from code blocks and diffs.", family: "terminal-font", size: "terminal-size", sizes: range(8, 20), base: 12, advanced: true, mono: true },
];
const look = (name) => load(`look-${name}`) ?? "";
const setLook = (name, value) => (value ? save(`look-${name}`, value) : localStorage.removeItem(`look-${name}`));
const advancedType = () => look("advanced") === "true";
const sizeOf = (font) => Number.parseInt(look(font.size)) || font.base;
const fontByName = (name) => FONTS.find((f) => f.name === name);

/** A typed font list as CSS: each name quoted unless it is a plain word. */
function fontStack(custom, fallback) {
  const names = custom.split(",").map((n) => n.trim().replace(/^(['"])(.*)\1$/, "$2").replaceAll('"', "")).filter(Boolean);
  return names.length ? `${names.map((n) => (/^[a-zA-Z][a-zA-Z0-9-]*$/.test(n) ? n : `"${n}"`)).join(", ")}, ${fallback}` : null;
}

/** The terminal's font: the code font unless Advanced gives it its own, as in T3 Code. Until one is set
 * it keeps pinned's own 12px terminal. */
function terminalLook() {
  const font = fontByName(advancedType() ? "terminal" : "code");
  return { fontFamily: fontStack(look(font.family), MONO) ?? "ui-monospace, Menlo, monospace", fontSize: look(font.size) ? sizeOf(font) : 12 };
}

/** T3's glass opacity, 40 to 100%: how solid menus, dialogs and the composer are (gui-theme.css). */
const GLASS = { key: "gui-glass-opacity", min: 40, max: 100, standard: 80 };
function glassOpacity() {
  const n = Number(load(GLASS.key));
  return Number.isInteger(n) && n >= GLASS.min && n <= GLASS.max ? n : GLASS.standard;
}
/** How long panels take to open and close: T3's setting, 0 to 400 ms by 25, off by default. */
const MOTION = { key: "gui-panel-animation", min: 0, max: 400, standard: 0 };
const panelAnimation = () => Math.min(MOTION.max, Math.max(MOTION.min, Number(load(MOTION.key)) || 0));

function applyLooks() {
  const root = document.documentElement;
  for (const name of Object.keys(LOOKS)) {
    const key = name.replace(/-(\w)/g, (_, c) => c.toUpperCase());
    if (look(name)) root.dataset[key] = look(name);
    else delete root.dataset[key];
  }
  const vars = {
    "--g-sans": fontStack(look("sans-font"), SANS),
    "--g-mono": fontStack(look("mono-font"), MONO),
    "--prompt-font": fontStack(look("prompt-font"), "var(--g-sans)"),
    "--prompt-size": look("prompt-size") ? `${sizeOf(fontByName("prompt"))}px` : null,
    "--code-size": look("code-size") ? `${sizeOf(fontByName("code"))}px` : null,
    "--ui-zoom": sizeOf(fontByName("interface")) === 16 ? null : String(sizeOf(fontByName("interface")) / 16),
    "--g-glass-opacity": glassOpacity() === GLASS.standard ? null : `${glassOpacity()}%`,
  };
  for (const [name, value] of Object.entries(vars)) root.style.setProperty(name, value);
  root.dataset.smoothing = look("smoothing") !== "false";
  if (vars["--ui-zoom"]) root.dataset.zoom = "";
  else delete root.dataset.zoom;
  for (const tab of typeof tabs === "undefined" ? [] : tabs) {
    Object.assign(tab.term.options, terminalLook());
    refit(tab);
  }
  // The panel's Terminal tabs and the drawer's project scripts (gui-panel.js, gui-composer.js).
  const shells = [...(typeof shellTerms === "undefined" ? [] : shellTerms.values()), ...(typeof scriptShells === "undefined" ? [] : scriptShells)];
  for (const { term, fit } of shells) {
    Object.assign(term.options, terminalLook());
    if (term.element?.offsetParent) fit.fit();
  }
}
function looksChanged() {
  applyLooks();
  rendered.clear();
  if (guiOn()) rerender();
}

const sliderHtml = ({ id, label, min, max, step, value, unit }) =>
  `<div class="slider-control"><output for="${id}">${value}${unit}</output><input type="range" class="settings-slider" id="${id}" aria-label="${label}" min="${min}" max="${max}" step="${step}" value="${value}" style="${sliderFill(value, min, max)}"></div>`;
const sliderFill = (value, min, max) => {
  const ratio = (value - min) / (max - min);
  return `--settings-slider-progress: ${ratio * 100}%; --settings-slider-fill-offset: ${0.5 - ratio}rem`;
};
function paintSlider(input, text) {
  input.style.cssText = sliderFill(Number(input.value), Number(input.min), Number(input.max));
  input.closest(".slider-control").querySelector("output").textContent = text;
}

resettable("glass", {
  changed: () => glassOpacity() !== GLASS.standard,
  reset: () => {
    localStorage.removeItem(GLASS.key);
    applyLooks();
  },
});
resettable("motion", { changed: () => panelAnimation() !== MOTION.standard, reset: () => localStorage.removeItem(MOTION.key) });
const resetLooks = (...names) => () => {
  for (const name of names) setLook(name, "");
  looksChanged();
};
for (const name of Object.keys(LOOKS)) resettable(name, { changed: () => !!look(name), reset: resetLooks(name) });

/** AppearanceSettingsPanel.tsx's Interface section. */
function interfaceSettingsHtml() {
  const lookRow = (name) => {
    const l = LOOKS[name];
    return settingsRowHtml({ id: l.id, title: l.label, about: l.about, reset: name, control: selectHtml({ id: `look-${name}`, value: look(name), options: l.options, label: l.label, cls: "wide" }) });
  };
  return settingsSectionHtml({ id: "appearance-interface", title: "Interface", rows: `${contrastRowHtml()}
    ${settingsRowHtml({ id: "setting-glass-opacity", title: "Glass opacity", about: "Higher values make menus, dialogs, and the composer more solid.", reset: "glass", control: sliderHtml({ id: "glass-opacity", label: "Glass opacity", ...GLASS, step: 5, value: glassOpacity(), unit: "%" }) })}
    ${lookRow("diff-colors")}
    ${lookRow("chat-width")}` });
}
function bindInterfaceSettings() {
  showContrast();
  settingsPage.querySelector("#glass-opacity").oninput = (e) => {
    const n = Number(e.target.value);
    if (!Number.isInteger(n) || n < GLASS.min || n > GLASS.max) return;
    if (n === GLASS.standard) localStorage.removeItem(GLASS.key);
    else save(GLASS.key, String(n));
    applyLooks();
    paintSlider(e.target, `${n}%`);
  };
  for (const name of Object.keys(LOOKS)) {
    settingsPage.querySelector(`#look-${name}`).onchange = (e) => {
      setLook(name, e.target.value);
      looksChanged();
    };
  }
}

/** PanelAnimationsPreview.tsx: a small sidebar, thread and panel that fold and unfold at the set
 * duration when clicked. */
const motionPreviewHtml = (ms) =>
  `<button type="button" class="motion-preview" aria-label="Replay panel animation preview" style="--preview-duration: ${ms}ms"><span class="side"></span>
    <span class="main"><span class="lines"><i></i><i></i><i></i></span><span class="foot"><i></i></span></span><span class="panel"></span></button>`;
function motionSettingsHtml() {
  const ms = panelAnimation();
  return settingsSectionHtml({ id: "motion", title: "Motion", rows: settingsRowHtml({ id: "panel-animations", title: "Panel animations", about: "Set how fast panels open and close.", reset: "motion",
    control: `<div class="motion-control">${motionPreviewHtml(ms)}${sliderHtml({ id: "panel-animation-duration", label: "Panel animation duration", ...MOTION, step: 25, value: ms, unit: " ms" })}</div>` }) });
}
function bindMotionSettings() {
  const preview = settingsPage.querySelector(".motion-preview");
  preview.onclick = () => preview.classList.toggle("closed");
  settingsPage.querySelector("#panel-animation-duration").oninput = (e) => {
    const ms = Number(e.target.value);
    if (!Number.isInteger(ms) || ms < MOTION.min || ms > MOTION.max) return;
    if (ms === MOTION.standard) localStorage.removeItem(MOTION.key);
    else save(MOTION.key, String(ms));
    preview.style.setProperty("--preview-duration", `${ms}ms`);
    paintSlider(e.target, `${ms} ms`);
  };
}
/** T3 animates the sidebar's collapse over the same duration (components/ui/sidebar.tsx), except under
 * reduced motion; resizing never animates. */
let sidebarAnimationTimer = 0;
new MutationObserver(() => {
  const ms = panelAnimation();
  if (!guiOn() || !ms || matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  document.documentElement.style.setProperty("--panel-animation", `${ms}ms`);
  document.body.classList.add("sidebar-animating");
  clearTimeout(sidebarAnimationTimer);
  sidebarAnimationTimer = setTimeout(() => document.body.classList.remove("sidebar-animating"), ms + 50);
}).observe(document.body, { attributes: true, attributeFilter: ["data-sidebar"] });

for (const font of FONTS) resettable(font.name, { changed: () => !!look(font.family) || !!look(font.size), reset: resetLooks(font.family, font.size) });
resettable("smoothing", { changed: () => look("smoothing") === "false", reset: resetLooks("smoothing") });
resettable("word-wrap", { changed: () => !wrapsByDefault(), reset: resetLooks("word-wrap") });

const lookSwitchHtml = (name, label, on) => `<span class="switch"><input type="checkbox" id="look-${name}" data-look-switch="${name}" aria-label="${label}"${on ? " checked" : ""}><span></span></span>`;
/** TypographySection: Interface and Monospace fonts, or all four with Advanced, then Word wrap. */
function appearanceHtml() {
  const advanced = advancedType();
  const fonts = FONTS.filter((f) => advanced || !f.advanced).map((f) => {
    const title = advanced ? f.label : f.simpleLabel ?? f.label;
    const size = selectHtml({ id: `look-${f.size}`, value: look(f.size) ? `${sizeOf(f)}px` : "", label: `${title} size`, cls: "size",
      options: f.sizes.map((n) => [n === f.base ? "" : `${n}px`, `${n} px`]) });
    return settingsRowHtml({ id: `${f.name}-font`, title, about: advanced ? f.about : f.simpleAbout ?? f.about, reset: f.name,
      control: `<div class="font-control">${fontFamilyControl(f, title)}${size}</div>` });
  }).join("");
  const advancedSwitch = `<label class="advanced">Advanced ${lookSwitchHtml("advanced", "Show advanced typography settings", advanced)}</label>`;
  return settingsSectionHtml({ id: "typography", title: "Typography", action: advancedSwitch, rows: `${fonts}
    ${advanced && IS_MAC ? settingsRowHtml({ id: "font-smoothing", title: "Font smoothing", about: "Use thinner grayscale text smoothing instead of the macOS default.", reset: "smoothing", control: lookSwitchHtml("smoothing", "Font smoothing", look("smoothing") !== "false") }) : ""}
    ${settingsRowHtml({ id: "word-wrap", title: "Word wrap", about: "Wrap long lines in code blocks, tables, diffs, and file previews by default.", reset: "word-wrap", control: lookSwitchHtml("word-wrap", "Wrap code, tables, diffs, and file previews by default", wrapsByDefault()) })}` });
}

function bindAppearance() {
  for (const font of FONTS) {
    const select = settingsPage.querySelector(`#look-${font.size}`);
    if (!select) continue;
    select.onchange = () => {
      setLook(font.size, select.value);
      looksChanged();
    };
  }
  for (const input of settingsPage.querySelectorAll("[data-look-switch]")) {
    input.onchange = () => {
      save(`look-${input.dataset.lookSwitch}`, String(input.checked));
      looksChanged();
      if (input.dataset.lookSwitch === "advanced") redrawSettingsPage();
    };
  }
  for (const input of settingsPage.querySelectorAll("[data-look-font]")) {
    let timer = 0;
    const font = FONTS.find((f) => f.family === input.dataset.lookFont);
    // A typed family takes once it names an installed font, monospace for code and the terminal;
    // until then the field stays marked, as T3's draft.
    const commit = () => {
      clearTimeout(timer);
      const next = input.value.trim();
      const takes = !next || acceptsFamily(next, font.mono);
      if (takes || next === look(font.family)) input.removeAttribute("aria-invalid");
      else input.setAttribute("aria-invalid", "true");
      if (!takes) return;
      setLook(font.family, next);
      looksChanged();
      refreshResets();
    };
    input.onfocus = () => discoverInstalledFonts(true);
    input.oninput = () => {
      clearTimeout(timer);
      timer = setTimeout(commit, 400);
    };
    input.onchange = commit;
    input.onkeydown = (e) => {
      if (e.key === "Enter") commit();
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      clearTimeout(timer);
      input.value = look(input.dataset.lookFont);
      input.removeAttribute("aria-invalid");
    };
  }
  probeGrantedFonts();
}

/** appearanceFonts.ts: whether a family is installed, found by drawing text in it beside each generic
 * family, and whether it is monospace, by drawing glyphs of every width in four styles. */
let fontProbeContext;
function probeFontWidth(fonts) {
  fontProbeContext ??= document.createElement("canvas").getContext("2d");
  if (!fontProbeContext) return null;
  fontProbeContext.font = `16px ${fonts}`;
  return fontProbeContext.measureText("mmmmmmmmMMWli1O0@# fjord").width;
}
const cssFamilies = (family) => fontStack(family, "")?.replace(/, $/, "") || null;
function fontAvailable(family) {
  const families = cssFamilies(family);
  if (!families) return false;
  if (/^(system-ui|sans-serif|serif|monospace|ui-monospace)$/i.test(families)) return true;
  return ["monospace", "serif", "sans-serif"].some((generic) => {
    const base = probeFontWidth(generic);
    const candidate = probeFontWidth(`${families}, ${generic}`);
    return base !== null && candidate !== null && candidate !== base;
  });
}
function fontMonospace(family) {
  const families = cssFamilies(family);
  if (!families) return true;
  fontProbeContext ??= document.createElement("canvas").getContext("2d");
  if (!fontProbeContext) return true;
  return ["normal 400", "normal 700", "italic 400", "italic 700"].every((variant) => {
    fontProbeContext.font = `${variant} 32px ${families}, monospace`;
    const widths = ["i", "M", "W", "0", "@", "#", ".", " "].map((g) => fontProbeContext.measureText(g).width);
    return widths.every((w) => Math.abs(w - widths[0]) < 0.01);
  });
}
const acceptsFamily = (family, mono) => fontAvailable(family) && (!mono || fontMonospace(family));

/** resolveDefaultFamilyLabel: the face the default stack draws, named, for placeholders and the
 * picker's default row. */
const defaultFamilyNames = {};
function defaultFamilyName(mono) {
  const key = mono ? "code" : "sans";
  if (defaultFamilyNames[key]) return defaultFamilyNames[key];
  const domWidth = (family) => {
    const span = Object.assign(document.createElement("span"), { textContent: "RagIl10O@ fjord quiz" });
    span.style.cssText = "position:absolute;left:-9999px;top:0;visibility:hidden;white-space:pre;font-size:100px";
    span.style.fontFamily = family;
    document.body.append(span);
    const w = span.getBoundingClientRect().width;
    span.remove();
    return w;
  };
  const generic = (name) => {
    const monoLike = /monospace/i.test(name);
    const width = domWidth(name);
    const candidates = monoLike ? ["Menlo", "Consolas", "Cascadia Mono", "DejaVu Sans Mono", "Ubuntu Mono", "Liberation Mono", "Noto Sans Mono", "Roboto Mono", "Monaco", "Courier New"]
      : ["Segoe UI", "Roboto", "Noto Sans", "Ubuntu", "Cantarell", "DejaVu Sans", "Liberation Sans", "Helvetica Neue", "Arial"];
    const hit = candidates.find((c) => fontAvailable(c) && Math.abs(domWidth(`"${c}"`) - width) < 0.01);
    if (hit) return hit;
    if (/mac|iphone|ipad|ipod/i.test(navigator.platform)) return monoLike ? "SF Mono" : "SF Pro";
    return null;
  };
  for (const raw of (mono ? MONO : SANS).split(",")) {
    const family = raw.trim().replace(/^(['"])(.*)\1$/, "$2");
    const named = /^(system-ui|sans-serif|serif|monospace|ui-monospace|-apple-system|BlinkMacSystemFont)$/i.test(family)
      ? (/^serif$/i.test(family) ? null : generic(family)) : fontAvailable(family) ? family : null;
    if (named) return (defaultFamilyNames[key] = named);
  }
  return (defaultFamilyNames[key] = mono ? "System monospace" : "System default");
}
const fontDefaultOf = (font) => (font.name === "prompt" ? look("sans-font") || defaultFamilyName(false) : defaultFamilyName(!!font.mono));

/** FontFamilyPicker.tsx: once the browser lists installed fonts (queryLocalFonts, asked on a field's
 * focus or already granted), each family is a searchable list drawn in its own face. */
const installedFonts = { status: typeof window.queryLocalFonts === "function" ? "unknown" : "unavailable", families: [], loading: false };
function discoverInstalledFonts(gesture = false) {
  if (installedFonts.status !== "unknown" || installedFonts.loading) return;
  installedFonts.loading = true;
  window.queryLocalFonts().then((fonts) => {
    const families = [...new Set(fonts.map((f) => f.family))].filter((f) => !f.startsWith(".")).sort((a, b) => a.localeCompare(b));
    Object.assign(installedFonts, families.length ? { status: "granted", families } : { status: "unavailable" });
  }, () => (installedFonts.status = "unavailable")).finally(() => {
    installedFonts.loading = false;
    if (installedFonts.status !== "granted" || !settingsPage.querySelector("#typography")) return;
    const focused = gesture && document.activeElement?.dataset?.lookFont;
    redrawSettingsPage();
    if (focused) settingsPage.querySelector(`[data-font-pick="${focused}"]`)?.click();
  });
}
let grantedFontsProbed = false;
function probeGrantedFonts() {
  if (grantedFontsProbed || installedFonts.status !== "unknown") return;
  grantedFontsProbed = true;
  navigator.permissions?.query({ name: "local-fonts" }).then((p) => p.state === "granted" && discoverInstalledFonts(), () => {});
}

function fontFamilyControl(font, title) {
  const value = look(font.family);
  if (installedFonts.status === "granted") {
    return `<button type="button" class="g-select font-pick" id="look-${font.family}" data-font-pick="${font.family}" aria-haspopup="listbox" aria-expanded="false" aria-label="${esc(title)} family"><span class="value">${esc(value || fontDefaultOf(font))}</span>${icon("chevron-down")}</button>`;
  }
  return `<input id="look-${font.family}" data-look-font="${font.family}" value="${esc(value)}" placeholder="${esc(fontDefaultOf(font))}" aria-label="${esc(title)} family" maxlength="200" spellcheck="false" autocomplete="off" autocapitalize="off">`;
}

let fontPicker = null;
function openFontPicker(trigger) {
  closeFontPicker();
  const font = FONTS.find((f) => f.family === trigger.dataset.fontPick);
  const popup = Object.assign(document.createElement("div"), { id: "font-picker" });
  popup.innerHTML = `<input type="text" placeholder="Search fonts…" aria-label="Search fonts" spellcheck="false" autocomplete="off"><div class="list" role="listbox" aria-label="${esc(trigger.getAttribute("aria-label"))}"></div>`;
  document.body.append(popup);
  fontPicker = { trigger, popup, font, hi: 0, items: [] };
  trigger.setAttribute("aria-expanded", "true");
  const r = trigger.getBoundingClientRect();
  popup.style.left = `${Math.max(8, Math.min(r.right - popup.offsetWidth, innerWidth - popup.offsetWidth - 8))}px`;
  popup.style.top = `${r.bottom + 4 + popup.offsetHeight <= innerHeight - 8 ? r.bottom + 4 : Math.max(8, r.top - 4 - popup.offsetHeight)}px`;
  renderFontPicker();
  const search = popup.querySelector("input");
  search.focus();
  search.oninput = () => {
    fontPicker.hi = 0;
    renderFontPicker();
  };
  search.onkeydown = (e) => {
    const n = fontPicker.items.length;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeFontPicker();
      return trigger.focus();
    }
    if (!n) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      fontPicker.hi = (fontPicker.hi + (e.key === "ArrowDown" ? 1 : -1) + n) % n;
      renderFontPicker();
    } else if (e.key === "Enter") {
      e.preventDefault();
      pickFont(fontPicker.items[fontPicker.hi]);
    }
  };
  popup.onclick = (e) => {
    const item = e.target.closest("[data-font]");
    if (item) pickFont(item.dataset.font);
  };
  popup.onmousemove = (e) => {
    const i = Number(e.target.closest("[data-index]")?.dataset.index ?? -1);
    if (i >= 0 && i !== fontPicker.hi) {
      fontPicker.hi = i;
      renderFontPicker();
    }
  };
}
function renderFontPicker() {
  const { popup, font } = fontPicker;
  const query = popup.querySelector("input").value.trim().toLowerCase();
  fontPicker.items = [...(query ? [] : [""]), ...installedFonts.families.filter((f) => !query || f.toLowerCase().includes(query))];
  const selected = look(font.family);
  const list = popup.querySelector(".list");
  list.innerHTML = fontPicker.items.length
    ? fontPicker.items.map((item, i) => {
        const family = item || fontDefaultOf(font);
        return `<div role="option" data-font="${esc(item)}" data-index="${i}" aria-selected="${item === selected}"${i === fontPicker.hi ? ' class="hi"' : ""}><span class="name" style="font-family: ${esc(fontStack(family, "system-ui"))}">${esc(family)}</span>${item ? "" : '<span class="default">default</span>'}${item === selected ? icon("check") : ""}</div>`;
      }).join("")
    : `<p class="none">No fonts found.</p>`;
  list.querySelector(".hi")?.scrollIntoView({ block: "nearest" });
}
function pickFont(family) {
  const { font, trigger } = fontPicker;
  closeFontPicker();
  trigger.focus();
  if (family && font.mono && !fontMonospace(family)) {
    return themeToast({ type: "error", title: `"${family}" isn't monospace`, description: "Code and terminal need a fixed-width font, so the current font was kept." });
  }
  setLook(font.family, family);
  looksChanged();
  redrawSettingsPage();
  settingsPage.querySelector(`[data-font-pick="${font.family}"]`)?.focus();
}
function closeFontPicker() {
  if (!fontPicker) return;
  fontPicker.popup.remove();
  fontPicker.trigger.setAttribute("aria-expanded", "false");
  fontPicker = null;
}
document.addEventListener("click", (e) => {
  const trigger = e.target.closest?.("[data-font-pick]");
  if (trigger) return fontPicker?.trigger === trigger ? closeFontPicker() : openFontPicker(trigger);
  if (fontPicker && !e.target.closest?.("#font-picker")) closeFontPicker();
});
addEventListener("resize", closeFontPicker);
applyLooks();

const showAdvancedType = () => setLook("advanced", "true");
settingsSection({ page: "appearance", label: "Appearance", icon: "palette", show: () => showAppearance(), search: [
  { id: "appearance", title: "Color scheme", terms: ["appearance light dark system mode"] },
  { id: "appearance", title: "Themes", terms: ["appearance colors palette custom import"] },
  { id: "setting-appearance-contrast", title: "Contrast", terms: ["colors borders interface"] },
  { id: "setting-glass-opacity", title: "Glass opacity", terms: ["transparent transparency solid menus dialogs composer"] },
  { id: "diff-color-scheme", title: "Diff colors", terms: ["red green blue orange additions deletions changes counts palette colorblind"] },
  { id: "chat-width", title: "Chat width", terms: ["wide full width column layout messages composer monitor"] },
  { id: "panel-animations", title: "Panel animations" },
  { id: "interface-font", title: "Interface font", terms: ["typography family size system sans"] },
  { id: "prompt-font", title: "Prompt font", terms: ["typography family size composer input"], prepare: showAdvancedType },
  { id: "code-font", title: "Code font", terms: ["typography family size monospace code blocks diffs file previews"] },
  { id: "terminal-font", title: "Terminal font", terms: ["typography family size monospace output"], prepare: showAdvancedType },
  { id: "font-smoothing", title: "Font smoothing", terms: ["typography text grayscale anti aliasing macos thin"], when: () => IS_MAC, prepare: showAdvancedType },
  { id: "word-wrap", title: "Word wrap", terms: ["long lines code blocks tables diffs file previews"] },
] });
