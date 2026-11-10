// Auto-archive's settings, as pure logic: the grace periods the page offers, the account's one
// view out of every machine's answer, and the writes a change sends. gui-sidebar.js renders and
// wires it on the Archive page; node --test drives this file alone.

/** The grace periods, in minutes. `null` is never, and any other number a machine holds is shown as it is. */
const AA_CHOICES = [
  [null, "Never"],
  [10, "After 10 minutes"],
  [30, "After 30 minutes"],
  [120, "After 2 hours"],
  [1440, "After 24 hours"],
];

/** What a machine that never answered stands for: workers after half an hour, other threads never. */
const AA_DEFAULT_SETTINGS = { workers: 30, sessions: null };

/** The words for one grace period. */
function autoArchiveLabel(minutes) {
  const known = AA_CHOICES.find(([value]) => value === minutes);
  if (known) return known[1];
  if (minutes % 60 === 0) return `After ${minutes / 60} hours`;
  return `After ${minutes} ${minutes === 1 ? "minute" : "minutes"}`;
}

/** The select's options, with the machine's own value added when it is not one of the choices. */
function autoArchiveOptions(minutes) {
  const choices = AA_CHOICES.some(([value]) => value === minutes) ? AA_CHOICES : [...AA_CHOICES, [minutes, autoArchiveLabel(minutes)]];
  return choices.map(([value, label]) => `<option value="${value ?? "never"}"${value === minutes ? " selected" : ""}>${label}</option>`).join("");
}

/** A select's value back to minutes: "never" is null. */
const autoArchiveMinutes = (value) => (value === "never" ? null : Number(value));

/** One view out of the machines' answers, `{ id, online, settings }` each: the first machine that
 * answered speaks for the account, since a save writes the same settings to all of them. */
function mergeAutoArchive(machines) {
  const answered = machines.find((m) => m.settings);
  return { ...AA_DEFAULT_SETTINGS, ...(answered?.settings ?? {}) };
}

/** The writes a change sends: the settings to every online machine that can take them. */
const autoArchiveWrites = (view, machines) => machines.filter((m) => m.online && !m.update).map((m) => ({ machine: m.id, settings: { workers: view.workers, sessions: view.sessions } }));
