/** T3 Code's settings search and rows (SettingsSidebarNav.tsx, settingsSearch.ts, settingsLayout.tsx), in
 * the settings overlay's nav and pages. */
const settingsNav = document.getElementById("settings-nav");
const settingsSearch = settingsNav.querySelector(".settings-search input");
const settingsResults = settingsNav.querySelector(".settings-results");
new MutationObserver(() => settingsOpen() || clearSettingsSearch()).observe(document.body, { attributes: true, attributeFilter: ["data-page"] });

/** settingsSearch.ts's searchSettings: every token in the title, the section or a term; ranked by how
 * the title matches, then in the order the sections list them. */
function searchSettingsItems(query) {
  const q = normalize(query);
  if (!q) return [];
  const tokens = q.split(" ");
  const items = settingsSections.flatMap((s) => (s.search ?? []).filter((item) => !item.when || item.when()).map((item) => ({ ...item, section: s })));
  return items
    .flatMap((item, index) => {
      const title = normalize(item.title);
      const fields = [title, normalize(item.section.label), ...(item.terms ?? []).map(normalize)];
      if (!tokens.every((t) => fields.some((f) => f.includes(t)))) return [];
      const rank = title === q ? 5 : title.startsWith(q) ? 4 : title.includes(q) ? 3 : tokens.every((t) => title.includes(t)) ? 2 : fields.some((f) => f.includes(q)) ? 1 : 0;
      return [{ item, index, rank }];
    })
    .sort((a, b) => b.rank - a.rank || a.index - b.index)
    .map(({ item }) => item);
}

const settingsFind = { results: [], hi: 0 };
function renderSettingsSearch() {
  const query = settingsSearch.value;
  const searching = !!query.trim();
  settingsFind.results = searchSettingsItems(query);
  settingsFind.hi = Math.min(settingsFind.hi, Math.max(settingsFind.results.length - 1, 0));
  const has = searching && settingsFind.results.length > 0;
  settingsNav.querySelector(".settings-search kbd").hidden = searching;
  settingsNav.querySelector(".settings-search .clear").hidden = !searching;
  settingsNav.querySelector(".settings-none").hidden = !searching || has;
  settingsNav.querySelector(".links").hidden = searching;
  settingsResults.hidden = !has;
  settingsSearch.setAttribute("aria-expanded", has);
  if (has) settingsSearch.setAttribute("aria-controls", settingsResults.id);
  else settingsSearch.removeAttribute("aria-controls");
  settingsResults.innerHTML = has
    ? settingsFind.results.map((item, i) => `<button type="button" role="option" tabindex="-1" id="settings-search-result-${i}" data-result="${i}" aria-selected="${i === settingsFind.hi}"${i === settingsFind.hi ? ' class="on"' : ""}>
        ${icon(item.section.icon)}<span class="text"><span class="title">${esc(item.title)}</span><span class="sub">${esc(item.section.label)}</span></span></button>`).join("")
    : "";
  if (has) settingsSearch.setAttribute("aria-activedescendant", `settings-search-result-${settingsFind.hi}`);
  else settingsSearch.removeAttribute("aria-activedescendant");
  settingsResults.querySelector(".on")?.scrollIntoView({ block: "nearest" });
}
function clearSettingsSearch() {
  if (!settingsSearch.value) return;
  settingsSearch.value = "";
  settingsFind.hi = 0;
  renderSettingsSearch();
}
function openSettingsResult(item) {
  clearSettingsSearch();
  item.prepare?.();
  if (page !== item.section.page || item.prepare) showPage(item.section.page);
  requestAnimationFrame(() => revealSetting(item));
}
/** settingsLayout.tsx's scrollAndFocusSettingsTarget: the setting scrolls to the middle, takes focus and
 * pulses once in the primary color. */
function revealSetting(item) {
  const titled = [...settingsPage.querySelectorAll("h2, h3, label, b")].find((el) => el.textContent.trim() === item.title);
  const target = (item.id && document.getElementById(item.id)) || titled?.closest(".settings-row, .settings-section, .card, .look, .side-setting, section") || titled;
  if (!target) return;
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  target.scrollIntoView({ block: "center", behavior: reduced ? "auto" : "smooth" });
  if (!target.hasAttribute("tabindex")) target.tabIndex = -1;
  target.focus({ preventScroll: true });
  target.classList.remove("settings-pulse");
  if (reduced) return;
  void target.offsetWidth;
  target.classList.add("settings-pulse");
  target.addEventListener("blur", () => target.classList.remove("settings-pulse"), { once: true });
}

settingsSearch.addEventListener("input", () => {
  settingsFind.hi = 0;
  renderSettingsSearch();
});
settingsSearch.addEventListener("keydown", (e) => {
  if (e.isComposing) return;
  const n = settingsFind.results.length;
  if (e.key === "Escape" && settingsSearch.value.trim()) {
    e.preventDefault();
    e.stopPropagation();
    return clearSettingsSearch();
  }
  if (!n || !settingsSearch.value.trim()) return;
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    settingsFind.hi = (settingsFind.hi + (e.key === "ArrowDown" ? 1 : -1) + n) % n;
    renderSettingsSearch();
  } else if (e.key === "Enter") {
    e.preventDefault();
    openSettingsResult(settingsFind.results[settingsFind.hi]);
  }
});
settingsNav.querySelector(".settings-search .clear").onclick = () => {
  clearSettingsSearch();
  settingsSearch.focus();
};
settingsResults.addEventListener("mousemove", (e) => {
  const i = Number(e.target.closest("[data-result]")?.dataset.result ?? -1);
  if (i < 0 || i === settingsFind.hi) return;
  settingsFind.hi = i;
  for (const b of settingsResults.querySelectorAll("[data-result]")) {
    const on = Number(b.dataset.result) === i;
    b.classList.toggle("on", on);
    b.setAttribute("aria-selected", on);
  }
  settingsSearch.setAttribute("aria-activedescendant", `settings-search-result-${i}`);
});
settingsResults.addEventListener("click", (e) => {
  const b = e.target.closest("[data-result]");
  if (b) openSettingsResult(settingsFind.results[Number(b.dataset.result)]);
});

function focusSettingsSearch() {
  settingsSearch.focus();
  settingsSearch.select();
}
const settingsInDialog = (el) => el?.closest?.('[role="dialog"], [aria-modal="true"], #g-select-popup, #palette:not([hidden]), #shell-dialog:not([hidden])');
// T3 focuses Search settings on / (outside a text field) or ⌘F, and goes back to the app on Escape.
window.addEventListener("keydown", (e) => {
  if (!guiOn() || !settingsOpen() || e.defaultPrevented || e.isComposing || settingsInDialog(e.target) || !paletteEl.hidden || !shellDialog.hidden) return;
  const mod = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  const typing = e.target.matches?.("input, textarea, select, [contenteditable]");
  if ((e.key.toLowerCase() === "f" && mod && !e.altKey && !e.shiftKey) || (e.key === "/" && !mod && !e.altKey && !typing)) {
    e.preventDefault();
    return focusSettingsSearch();
  }
  if (e.key !== "Escape" || e.repeat || (typing && e.target.closest("#settings-page"))) return;
  e.preventDefault();
  document.activeElement?.blur();
  toggleSettings();
});

/** settingsLayout.tsx's SettingsSection and SettingsRow: a titled group of rows, each a title with its
 * reset button, a description, and a control at the end. */
const settingsSectionHtml = ({ id, title, rows, action = "", plain = false }) =>
  `<section class="settings-section${plain ? " plain" : ""}" id="${id}" aria-label="${esc(title)}">${plain ? "" : `<div class="section-head"><h3>${esc(title)}</h3>${action}</div>`}<div class="settings-group">${rows}</div></section>`;
const settingsRowHtml = ({ id, title, about = "", control = "", reset = "", body = "" }) =>
  `<div class="settings-row" id="${id}"><div class="row-line"><div class="row-text"><div class="row-title"><h4>${esc(title)}</h4>${reset ? resetButtonHtml(reset, title) : ""}</div>${about ? `<p>${about}</p>` : ""}</div>${control ? `<div class="row-control">${control}</div>` : ""}</div>${body}</div>`;

/** SettingResetButton: shown while the row's setting differs from its default. `resettable` names
 * what a row's button checks and puts back. */
const settingResets = new Map();
const resettable = (key, how) => settingResets.set(key, how);
const resetButtonHtml = (key, title) =>
  `<button type="button" class="row-reset" data-reset="${key}" aria-label="Reset ${esc(title.toLowerCase())} to default" title="Reset to default"${settingResets.get(key)?.changed() ? "" : " hidden"}>${icon("undo-2")}</button>`;
function refreshResets() {
  for (const b of settingsPage.querySelectorAll("[data-reset]")) b.hidden = !settingResets.get(b.dataset.reset)?.changed();
}
function redrawSettingsPage() {
  const top = settingsPage.scrollTop;
  settingsSectionOf(page)?.show();
  settingsPage.scrollTop = top;
}
settingsPage.addEventListener("click", (e) => {
  const b = e.target.closest("[data-reset]");
  if (!b) return;
  settingResets.get(b.dataset.reset)?.reset();
  redrawSettingsPage();
  settingsPage.querySelector(`[data-reset="${b.dataset.reset}"]`)?.closest(".settings-row")?.querySelector(".row-control :is(button, input)")?.focus();
});
for (const type of ["input", "change"]) settingsPage.addEventListener(type, () => queueMicrotask(refreshResets));

/** T3's Select (components/ui/select.tsx): a trigger that shows the value and opens a popup of options,
 * aligned to its end. The trigger is a button whose `value` and change event stand in for a select's;
 * each option is [value, label, label in the popup]. */
const selectHtml = ({ id, value, options, label, cls = "" }) => {
  const current = options.find(([v]) => v === value) ?? options[0];
  return `<button type="button" class="g-select ${cls}" id="${id}" value="${esc(current[0])}" aria-haspopup="listbox" aria-expanded="false" aria-label="${esc(label)}" data-options="${esc(JSON.stringify(options))}"><span class="value">${current[1]}</span>${icon("chevron-down")}</button>`;
};
let selectOpen = null;
function openSelect(trigger) {
  closeSelect();
  const options = JSON.parse(trigger.dataset.options);
  const popup = Object.assign(document.createElement("div"), { id: "g-select-popup" });
  popup.setAttribute("role", "listbox");
  popup.setAttribute("aria-label", trigger.getAttribute("aria-label"));
  popup.innerHTML = options.map(([v, l, item], i) => `<div role="option" id="g-select-${i}" data-value="${esc(v)}" aria-selected="${v === trigger.value}"${v === trigger.value ? ' class="on"' : ""}>${item ?? l}</div>`).join("");
  document.body.append(popup);
  selectOpen = { trigger, popup, hi: Math.max(0, options.findIndex(([v]) => v === trigger.value)) };
  placeSelect();
  trigger.setAttribute("aria-expanded", "true");
  highlightSelect(selectOpen.hi);
}
/** Under the trigger with their right edges lined up, or above it when there is no room below. */
function placeSelect() {
  const { trigger, popup } = selectOpen;
  const r = trigger.getBoundingClientRect();
  if (!r.width || r.bottom < 0 || r.top > innerHeight) return closeSelect();
  popup.style.maxHeight = "";
  popup.style.minWidth = `${r.width}px`;
  const w = popup.offsetWidth;
  const h = popup.offsetHeight;
  const below = r.bottom + 4 + h <= innerHeight - 8 || r.top - 4 - h < 8;
  Object.assign(popup.style, { left: `${Math.max(8, Math.min(r.right - w, innerWidth - w - 8))}px`, top: `${below ? r.bottom + 4 : Math.max(8, r.top - 4 - h)}px`, maxHeight: `${Math.max(120, below ? innerHeight - r.bottom - 12 : r.top - 12)}px` });
}
function highlightSelect(i) {
  const items = selectOpen.popup.querySelectorAll("[role=option]");
  selectOpen.hi = (i + items.length) % items.length;
  items.forEach((el, n) => el.classList.toggle("hi", n === selectOpen.hi));
  const item = items[selectOpen.hi];
  const box = selectOpen.popup;
  if (item.offsetTop < box.scrollTop) box.scrollTop = item.offsetTop;
  else if (item.offsetTop + item.offsetHeight > box.scrollTop + box.clientHeight) box.scrollTop = item.offsetTop + item.offsetHeight - box.clientHeight;
  selectOpen.trigger.setAttribute("aria-activedescendant", `g-select-${selectOpen.hi}`);
}
function pickSelect(value) {
  const { trigger } = selectOpen;
  closeSelect();
  trigger.focus();
  if (value === trigger.value) return;
  const [, label] = JSON.parse(trigger.dataset.options).find(([v]) => v === value);
  trigger.value = value;
  trigger.querySelector(".value").innerHTML = label;
  trigger.dispatchEvent(new Event("change", { bubbles: true }));
}
function closeSelect() {
  if (!selectOpen) return;
  selectOpen.popup.remove();
  selectOpen.trigger.setAttribute("aria-expanded", "false");
  selectOpen.trigger.removeAttribute("aria-activedescendant");
  selectOpen = null;
}
document.addEventListener("click", (e) => {
  const trigger = e.target.closest?.(".g-select[data-options]");
  if (trigger) return selectOpen?.trigger === trigger ? closeSelect() : openSelect(trigger);
  const option = e.target.closest?.("#g-select-popup [role=option]");
  if (option) return pickSelect(option.dataset.value);
  if (selectOpen && !e.target.closest?.("#g-select-popup")) closeSelect();
});
document.addEventListener("mousemove", (e) => {
  const option = selectOpen && e.target.closest?.("#g-select-popup [role=option]");
  if (option) highlightSelect([...selectOpen.popup.children].indexOf(option));
});
document.addEventListener("keydown", (e) => {
  const trigger = e.target.closest?.(".g-select[data-options]");
  if (!selectOpen) {
    if (trigger && ["ArrowDown", "ArrowUp"].includes(e.key)) {
      e.preventDefault();
      openSelect(trigger);
    }
    return;
  }
  const keys = { ArrowDown: () => highlightSelect(selectOpen.hi + 1), ArrowUp: () => highlightSelect(selectOpen.hi - 1), Home: () => highlightSelect(0), End: () => highlightSelect(-1),
    Enter: () => pickSelect(selectOpen.popup.children[selectOpen.hi].dataset.value), " ": () => pickSelect(selectOpen.popup.children[selectOpen.hi].dataset.value),
    Escape: () => { const t = selectOpen.trigger; closeSelect(); t.focus(); }, Tab: () => closeSelect() };
  if (!keys[e.key]) return;
  if (e.key !== "Tab") e.preventDefault();
  e.stopPropagation();
  keys[e.key]();
}, true);
addEventListener("resize", () => selectOpen && placeSelect());
addEventListener("scroll", (e) => selectOpen && !selectOpen.popup.contains(e.target) && placeSelect(), true);
