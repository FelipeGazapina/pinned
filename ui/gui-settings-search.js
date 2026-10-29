/** T3 Code's settings search in the command palette (settings/settingsSearch.ts): each section's own
 * entries under "Settings · Section", opened where they sit on their page, and every keybinding after them. */
function settingsSearchItems() {
  const entry = (section, item, extra = {}) => ({
    icon: "settings",
    title: esc(item.title),
    sub: `Settings · ${section.label}`,
    terms: [item.title, section.label, ...(item.terms ?? [])],
    run: async () => {
      await openSettings();
      openSettingsResult({ ...item, section });
    },
    ...extra,
  });
  const settings = settingsSections.flatMap((s) => (s.search ?? []).filter((item) => !item.when || item.when()).map((item) => entry(s, item)));
  const keybindings = settingsSectionOf("keybindings");
  const clearQuery = () => {
    kb.query = "";
    const search = document.querySelector("#keybindings .kb-search input");
    if (search) search.value = "";
  };
  const keys = [...new Set(DEFAULT_KEYBINDINGS.map((r) => r.command))].sort((a, b) => commandTitle(a).localeCompare(commandTitle(b))).map((command) => entry(keybindings, {
    id: `keybinding-${command}`,
    title: commandTitle(command),
    terms: [command, ...DEFAULT_KEYBINDINGS.filter((r) => r.command === command).map((r) => r.key)],
    prepare: clearQuery,
  }, { secondary: true }));
  return [...settings, ...keys];
}
