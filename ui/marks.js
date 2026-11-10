/** The account's hub keeps how the user filed each conversation (crates/core/src/marks.rs), one row
 * per Claude session. These turn the sidebar's marks, seen times and archive into those rows and
 * back, and `marksSync` keeps the page and the hub in step. */

const MARK_FIELDS = ["title", "pin", "order", "settled", "unsettled", "snoozed", "seen", "no_auto_settle", "archived"];
/** Row fields `sideMarks` keeps by thread key, and those it keeps as ordered lists of thread keys. */
const KEYED_MARKS = { title: "titles", settled: "settled", unsettled: "unsettled", snoozed: "snoozed", no_auto_settle: "noAutoSettle" };
const LISTED_MARKS = { pin: "pins", order: "order" };
const MARKS_PER_CALL = 2000;

const markUnset = (value) => value == null || value === false;
const hasMarks = (row) => MARK_FIELDS.some((field) => !markUnset(row[field]));
/** The hub refuses a call with text over 2000 bytes; 500 characters never reach it. */
const markText = (text) => (text.length > 500 ? Array.from(text).slice(0, 500).join("") : text);
/** A field's value compared by content, whatever order the hub wrote its keys in. */
const markJson = (value) => JSON.stringify(markUnset(value) ? null : typeof value === "object" ? Object.fromEntries(Object.entries(value).sort(([a], [b]) => (a < b ? -1 : 1))) : value);

function markValue(field, value) {
  if (markUnset(value) || value === "") return null;
  if (field === "title") return markText(value);
  if (field === "seen") return value || null;
  if (field === "no_auto_settle") return true;
  if (field === "snoozed") return { until: value.until, at: value.at, ...(Number.isInteger(value.limit) && value.limit >= 0 && { limit: value.limit }) };
  if (field !== "archived") return value;
  const { machine = "", cwd = "", name = "", project = "", at, created, title, pinned } = value;
  return { machine: markText(machine), cwd: markText(cwd), name: markText(name), project: markText(project), at: Number(at) || 0, ...(created && { created }), ...(title && { title: markText(title) }), ...(pinned && { pinned: true }) };
}

/** The page's marks as the hub's rows, by session id. Marks of a thread with no session yet have no row. */
function markRows({ sideMarks, seenAt, archived }) {
  const rows = new Map();
  const set = (key, field, value) => {
    value = markValue(field, value);
    if (!key?.startsWith("session:") || value == null) return;
    const session = key.slice("session:".length);
    if (!rows.has(session)) rows.set(session, { session });
    rows.get(session)[field] = value;
  };
  for (const [field, kept] of Object.entries(KEYED_MARKS)) for (const [key, value] of Object.entries(sideMarks[kept])) set(key, field, value);
  for (const [field, kept] of Object.entries(LISTED_MARKS)) sideMarks[kept].filter((key) => key.startsWith("session:")).forEach((key, place) => set(key, field, place));
  for (const [key, at] of Object.entries(seenAt)) set(key, "seen", at);
  for (const { session, ...entry } of archived) if (session) set(`session:${session}`, "archived", entry);
  return rows;
}

/** The page's marks from the hub's rows: `markRows` turned around. */
function markState(rows) {
  const sideMarks = { pins: [], order: [], settled: {}, unsettled: {}, snoozed: {}, titles: {}, noAutoSettle: {} };
  const seenAt = {};
  const archived = [];
  const placed = { pin: [], order: [] };
  for (const row of rows.values()) {
    const key = `session:${row.session}`;
    for (const [field, kept] of Object.entries(KEYED_MARKS)) if (!markUnset(row[field])) sideMarks[kept][key] = row[field];
    for (const field in placed) if (row[field] != null) placed[field].push(row);
    if (row.seen) seenAt[key] = row.seen;
    if (row.archived) archived.push({ ...row.archived, session: row.session });
  }
  // Two devices that pin at once can take one place; both then order the tie the same way.
  for (const [field, kept] of Object.entries(LISTED_MARKS)) sideMarks[kept] = placed[field].sort((a, b) => a[field] - b[field] || (a.session < b.session ? -1 : 1)).map((row) => `session:${row.session}`);
  archived.sort((a, b) => a.at - b.at);
  return { sideMarks, seenAt, archived };
}

/** The `mark_sessions` changes that turn rows `before` into rows `after`: one per field that differs,
 * and `"clear"` for a session that has no row left. */
function markChanges(before, after) {
  const changes = [];
  for (const session of before.keys()) if (!after.has(session)) changes.push({ session, mark: "clear" });
  for (const [session, row] of after) {
    const was = before.get(session) ?? {};
    for (const field of MARK_FIELDS) {
      if (markJson(was[field]) !== markJson(row[field])) changes.push({ session, mark: { [field]: markUnset(row[field]) ? (field === "no_auto_settle" ? false : null) : row[field] } });
    }
  }
  return changes;
}

/** Rows with `changes` made, as the hub makes them. */
function applyMarkChanges(rows, changes) {
  const next = new Map(rows);
  for (const { session, mark } of changes) {
    const row = mark === "clear" ? { session } : { ...next.get(session), ...mark, session };
    for (const field of MARK_FIELDS) if (markUnset(row[field])) delete row[field];
    if (hasMarks(row)) next.set(session, row);
    else next.delete(session);
  }
  return next;
}

/** The changes that move what this browser kept up to a hub that may know some of it already: the
 * hub keeps each field it has, gets each one it lacks, and pins and orders brought up go after its own. */
function marksMigration(local, hub) {
  const changes = [];
  const placed = { pin: [], order: [] };
  for (const [session, row] of local) {
    const theirs = hub.get(session) ?? {};
    for (const field of MARK_FIELDS) {
      if (markUnset(row[field]) || !markUnset(theirs[field])) continue;
      if (field in placed) placed[field].push(row);
      else changes.push({ session, mark: { [field]: row[field] } });
    }
  }
  for (const field in placed) {
    const first = [...hub.values()].reduce((top, row) => Math.max(top, row[field] ?? -1), -1) + 1;
    placed[field].sort((a, b) => a[field] - b[field]).forEach((row, i) => changes.push({ session: row.session, mark: { [field]: first + i } }));
  }
  return changes;
}

/** The fields the page changed that the hub has not taken yet, as `"<session> <field>"`: those in
 * `mine` that differ from `known`, and those on their way in `flight`. */
function heldMarks(known, mine, flight = []) {
  const held = new Set();
  for (const { session, mark } of [...markChanges(known, mine), ...flight]) for (const field of mark === "clear" ? MARK_FIELDS : Object.keys(mark)) held.add(`${session} ${field}`);
  return held;
}

/** `mine` with the hub's rows in `theirs` laid over it, but for `held` fields. With `every`, a session
 * the hub has no row for loses its marks too. */
function mergedMarks(mine, theirs, held, every = false) {
  const next = new Map(mine);
  for (const session of new Set([...theirs.keys(), ...(every ? mine.keys() : [])])) {
    const ours = mine.get(session) ?? {};
    const hubs = theirs.get(session) ?? {};
    const row = { session };
    for (const field of MARK_FIELDS) {
      const value = held.has(`${session} ${field}`) ? ours[field] : hubs[field];
      if (!markUnset(value)) row[field] = value;
    }
    if (hasMarks(row)) next.set(session, row);
    else next.delete(session);
  }
  return next;
}

/** `current` with every session's marks taken from `sessions`, keeping those of threads with no session yet. */
function withSessionMarks(current, sessions) {
  const ofSession = (key) => key.startsWith("session:");
  const unsessioned = (kept) => Object.fromEntries(Object.entries(kept).filter(([key]) => !ofSession(key)));
  const sideMarks = {};
  for (const [name, kept] of Object.entries(current.sideMarks)) {
    sideMarks[name] = Array.isArray(kept) ? [...sessions.sideMarks[name], ...kept.filter((key) => !ofSession(key))] : { ...unsessioned(kept), ...sessions.sideMarks[name] };
  }
  return { sideMarks, seenAt: { ...unsessioned(current.seenAt), ...sessions.seenAt }, archived: [...current.archived.filter((entry) => !entry.session), ...sessions.archived] };
}

/** Keeps the page's marks on the account's hub once it answers `session_marks`, and in this browser's
 * storage before that, with a hub too old to answer, and signed out. `io` reads and writes the page's
 * marks (`read`, `write`), its storage (`stored`, `persist`, `unstore`), calls the hub (`call`), and
 * redraws (`changed`). */
function marksSync(io) {
  let onHub = false;
  /** The hub's rows as last heard. */
  let known = new Map();
  /** The changes on their way to the hub. */
  let flight = [];
  let draining = null;
  /** Each connect and sign-out starts a new one, so an answer to an older one is dropped. */
  let epoch = 0;
  /** Pushes that came while a connect waited for the rows, laid over them once they are in. */
  let joining = null;
  const mine = () => markRows(io.read());

  function show(next, before) {
    if (!markChanges(before, next).length) return;
    io.write(withSessionMarks(io.read(), markState(next)));
    io.changed();
  }
  function adopt(rows) {
    const before = mine();
    const held = heldMarks(known, before, flight);
    for (const row of rows) hasMarks(row) ? known.set(row.session, row) : known.delete(row.session);
    show(mergedMarks(before, new Map(rows.map((row) => [row.session, row])), held), before);
  }
  async function drain() {
    try {
      await null;
      while (onHub) {
        flight = markChanges(known, mine()).slice(0, MARKS_PER_CALL);
        if (!flight.length) return true;
        const at = epoch;
        const rows = await io.call({ cmd: "mark_sessions", changes: flight }).catch(() => null);
        if (!rows || at !== epoch) return false;
        adopt(rows);
      }
      return false;
    } finally {
      flight = [];
      draining = null;
    }
  }
  /** Sends what the hub lacks; true once it has everything. A failed send waits for the next connect. */
  const sync = () => (onHub ? (draining ??= drain()) : Promise.resolve(false));

  /** Back to storage, on sign-out and when the hub cannot keep marks. `keep` writes the hub's marks
   * there; otherwise the page forgets them. */
  function leave(keep) {
    if (onHub) keep ? io.persist() : io.write(withSessionMarks(io.read(), markState(new Map())));
    onHub = false;
    known = new Map();
    joining = null;
    epoch++;
  }

  /** On every connect: sends what waited, takes the hub's rows, and the first time brings this
   * browser's stored marks up, forgetting them once the hub has them. */
  async function hello() {
    const at = ++epoch;
    const wasOnHub = onHub;
    const start = mine();
    joining = [];
    if (wasOnHub) await sync();
    let rows;
    try {
      rows = await io.call({ cmd: "session_marks" });
    } catch (e) {
      if (at !== epoch) return;
      joining = null;
      if (/unknown variant|can't answer/.test(String(e))) leave(true);
      return;
    }
    if (at !== epoch) return;
    const now = mine();
    const held = heldMarks(wasOnHub ? known : start, now, flight);
    known = new Map(rows.filter(hasMarks).map((row) => [row.session, row]));
    let next = mergedMarks(now, known, held, true);
    if (io.stored()) next = applyMarkChanges(next, marksMigration(now, known));
    onHub = true;
    show(next, now);
    for (const pushed of joining) adopt(pushed);
    joining = null;
    if ((await sync()) && at === epoch && io.stored()) io.unstore();
  }

  function push(rows) {
    if (joining) joining.push(rows);
    else if (onHub) {
      adopt(rows);
      sync();
    }
  }

  /** In hub mode, sends the page's marks on and answers truthy; otherwise the caller stores them. */
  const saved = () => onHub && sync();
  return { hello, push, leave, saved };
}
