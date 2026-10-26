/** T3 Code's settings search (settings/settingsSearch.ts): each setting by its own title under
 * "Settings · Section", opened where it sits on its page, and every keybinding after them. */
const SETTINGS_SEARCH = [
  ["Keep this Mac awake", "general", ["sleep", "display", "caffeinate"], { desktop: true }],
  ["Color scheme", "general", ["theme", "light", "dark", "system", "appearance", "mode"]],
  ["Themes", "general", ["theme", "palette", "colors", "import", "editor"]],
  ["Contrast", "general", ["accessibility", "text", "borders"]],
  ["Chat width", "general", ["messages", "composer", "wide", "full"]],
  ["Diff colors", "general", ["additions", "deletions", "colorblind"]],
  ["Typography", "general", ["fonts", "font size"]],
  ["Interface font", "general", ["typography", "font", "size"]],
  ["Prompt font", "general", ["typography", "font", "composer"]],
  ["Monospace font", "general", ["typography", "code font", "terminal"]],
  ["Working section (beta)", "general", ["sidebar", "organization", "monitoring"]],
  ["Auto-settle merged threads", "general", ["sidebar", "settle", "pull request"]],
  ["Auto-settle inactive threads", "general", ["sidebar", "settle", "inactivity"]],
  ["Panel animation", "general", ["right panel", "motion", "duration"]],
  ["Send shortcut", "general", ["enter", "composer", "newline"]],
  ["Linear", "integrations", ["connectors", "issues", "api key"]],
  ["Slack", "integrations", ["connectors", "messages", "bot"]],
  ["GitHub", "integrations", ["connectors", "token", "repositories", "source control"]],
  ["Projects", "projects", ["history", "changes", "turns"]],
  ["Keybindings", "keybindings", ["shortcuts", "keyboard", "hotkeys"]],
  ["Workflows", "connectors", ["automation", "triggers", "agents"]],
  ["Account", "account", ["sign in", "sign out", "hub", "email", "password"]],
  ["Machines", "machines", ["servers", "remote", "computers", "environments"]],
  ["Archive", "archive", ["archived threads", "unarchive", "delete"]],
];
const settingsPageLabel = (page) => document.querySelector(`#settings-nav [data-page="${page}"]`)?.textContent.trim() || page;

/** searchSettings: nothing before a query; keybindings rank after the settings. */
function settingsSearchItems() {
  const settings = SETTINGS_SEARCH.filter(([, , , o]) => !o?.desktop || TAURI).map(([title, page, terms]) => ({
    icon: "settings",
    title: esc(title),
    sub: `Settings · ${settingsPageLabel(page)}`,
    terms: [title, settingsPageLabel(page), ...terms],
    run: () => openSetting(page, title),
  }));
  const keys = [...new Set(DEFAULT_KEYBINDINGS.map((r) => r.command))].sort((a, b) => commandTitle(a).localeCompare(commandTitle(b))).map((command) => ({
    icon: "settings",
    title: esc(commandTitle(command)),
    sub: `Settings · Keybindings`,
    secondary: true,
    terms: [commandTitle(command), "Keybindings", command, ...DEFAULT_KEYBINDINGS.filter((r) => r.command === command).map((r) => r.key)],
    run: () => openSetting("keybindings", "Keybindings", `keybinding-${command}`),
  }));
  return [...settings, ...keys];
}
/** Opens `page` and brings the row titled `title`, or the element `id`, into view, with a brief highlight. */
async function openSetting(page, title, id = null) {
  await openSettings(page);
  if (id && kb.query) {
    kb.query = "";
    renderKeybindings();
    const search = document.querySelector("#keybindings .kb-search input");
    if (search) search.value = "";
  }
  const find = () => (id && document.getElementById(id)) || [...settingsPage.querySelectorAll("h2, h3, label, b, .side-setting label, .kb-title")].find((el) => el.textContent.trim() === title || el.firstChild?.textContent?.trim() === title);
  const until = Date.now() + 2000;
  let el = find();
  while (!el && Date.now() < until) {
    await new Promise((r) => setTimeout(r, 50));
    el = find();
  }
  const row = el?.closest(".kb-row, .look, .side-setting, section, p, div") ?? el;
  if (!row) return;
  row.scrollIntoView({ block: "center" });
  row.classList.add("setting-found");
  setTimeout(() => row.classList.remove("setting-found"), 1600);
}
