// The global orchestrator's account-wide settings, as pure logic: the merge of every machine's
// answer into the one view Felipe edits, the host choice, and the writes a save sends. Kept here
// so node --test can drive it without the page; gui-global-orchestrator.js renders and wires it.

/** The names the settings offer. Anything else is custom text, which the node cleans. */
const GO_NAME_PRESETS = ["Orquestrador", "Almirante", "Comandante"];

/** CONTRACTS §2's defaults, for machines that never answered. */
const GO_DEFAULT_SETTINGS = { name: "Orquestrador", host: false, busy: "queue", reports: "auto", group_reports: true, read_aloud: false };

/** The busy policy and the report policy, each value with the words the section shows. */
const GO_BUSY_CHOICES = [
  ["queue", "Pôr na fila"],
  ["refuse", "Recusar e tentar depois"],
];
const GO_REPORT_CHOICES = [
  ["auto", "Automático no fim do turno"],
  ["tool_only", "Só quando o Capitão chamar"],
];

/** An older node answers an unknown command and an older hub refuses the call; both wordings
 * appear in the error text, so either one means "this machine needs an update". */
const isOlderPinned = (error) => /unknown variant|can't answer/.test(String(error ?? ""));

/** One account-wide view out of every machine's answer. `machines` holds one entry per machine:
 * `{ id, name, online, settings, update, error }`, with `settings` present only when the machine
 * answered, `update` when it is online but too old, and `error` for any other read failure. The
 * shared fields come from the host when it answered, else from the first machine that did; `host`
 * becomes that machine's id, or null when no machine claims it. */
function mergeGlobalSettings(machines) {
  const answered = machines.filter((m) => m.settings);
  const host = machines.find((m) => m.settings?.host === true) ?? null;
  const base = host?.settings ? host : answered[0] ?? null;
  return { ...GO_DEFAULT_SETTINGS, ...(base?.settings ?? {}), host: host?.id ?? null };
}

/** The writes a save sends: the shared fields to every online machine that can take them, with
 * `host` true only on the chosen machine. Offline and too-old machines are skipped; the section
 * tells Felipe about them instead of failing the save. */
function globalSettingsWrites(view, machines) {
  return machines
    .filter((m) => m.online && !m.update)
    .map((m) => ({
      machine: m.id,
      settings: {
        name: view.name,
        host: m.id === view.host,
        busy: view.busy,
        reports: view.reports,
        group_reports: view.group_reports !== false,
        read_aloud: !!view.read_aloud,
      },
    }));
}

/** One line per machine the save could not reach, in the machine's own words: too old, offline, or
 * failed. Machines that took the settings stay quiet. */
function globalSettingsNotes(machines) {
  return machines
    .filter((m) => m.update || !m.online || m.error)
    .map((m) =>
      m.update ? `${m.name} precisa atualizar o pinned`
      : !m.online ? `${m.name} está offline; atualiza quando reconectar`
      : `${m.name}: ${m.error}`,
    );
}
