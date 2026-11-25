// Compaction's settings, as pure logic: the thresholds the page offers, the account's one view out
// of every machine's answer, the writes a change sends, and the rule for when the composer offers
// to compact. gui-composer.js renders and wires it; node --test drives this file alone.

/** What a machine that never answered stands for, in tokens. */
const COMPACT_DEFAULT_TOKENS = 200_000;

/** How long claude's prompt cache lasts: an older context costs its whole history again. */
const CACHE_COLD_MS = 70 * 60_000;

/** The thresholds, in tokens. `null` is never, and any other number a machine holds is shown as it is. */
const COMPACT_CHOICES = [
  [null, "Never"],
  [50_000, "From 50k tokens"],
  [100_000, "From 100k tokens"],
  [150_000, "From 150k tokens"],
  [200_000, "From 200k tokens"],
  [300_000, "From 300k tokens"],
  [500_000, "From 500k tokens"],
];

/** The words for one threshold. */
function compactionLabel(tokens) {
  const known = COMPACT_CHOICES.find(([value]) => value === tokens);
  if (known) return known[1];
  return `From ${tokens % 1000 ? tokens : `${tokens / 1000}k`} tokens`;
}

/** The select's options, with the machine's own value added when it is not one of the choices. */
function compactionOptions(tokens) {
  const choices = COMPACT_CHOICES.some(([value]) => value === tokens) ? COMPACT_CHOICES : [...COMPACT_CHOICES, [tokens, compactionLabel(tokens)]];
  return choices.map(([value, label]) => `<option value="${value ?? "never"}"${value === tokens ? " selected" : ""}>${label}</option>`).join("");
}

/** A select's value back to tokens: "never" is null. */
const compactionChoice = (value) => (value === "never" ? null : Number(value));

/** One view out of the machines' answers, `{ id, online, settings }` each: the first machine that
 * answered speaks for the account, since a save writes the same settings to all of them. */
function mergeCompaction(machines) {
  const answered = machines.find((m) => m.settings);
  return { offer_tokens: COMPACT_DEFAULT_TOKENS, ...(answered?.settings ?? {}) };
}

/** The writes a change sends: the settings to every online machine that can take them. */
const compactionWrites = (view, machines) => machines.filter((m) => m.online && !m.update).map((m) => ({ machine: m.id, settings: { offer_tokens: view.offer_tokens } }));

/** The tokens to offer to compact, or null: a context at least `threshold` tokens long whose cache
 * has gone cold by `now`. An undefined threshold is the default, and null is never. */
function compactOfferTokens(context, threshold = COMPACT_DEFAULT_TOKENS, now) {
  if (threshold === null || context.tokens < threshold) return null;
  return now - Date.parse(context.at) >= CACHE_COLD_MS ? context.tokens : null;
}
