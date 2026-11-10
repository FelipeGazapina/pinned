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

/** Thread keys whose marks outlive a reload: a session's, and a tab's whose id names the run that
 * opened it (machine.rs `add_tab`). A node before 0.17.0 numbered tabs from 1 each run, so marks under
 * such an id would land on whichever thread took the number next. */
const keptKey = (key) => key.startsWith("session:") || /:tab-[0-9a-f]+-\d+$/.test(key);

/** The sidebar's marks as this browser stored them in `gui-marks`. */
function storedSideMarks(saved) {
  const sideMarks = {};
  for (const kept of Object.values(LISTED_MARKS)) sideMarks[kept] = (saved?.[kept] ?? []).filter(keptKey);
  for (const kept of Object.values(KEYED_MARKS)) sideMarks[kept] = Object.fromEntries(Object.entries(saved?.[kept] ?? {}).filter(([key]) => keptKey(key)));
  return sideMarks;
}

/** The hub's rows as this browser last heard them, and from which account. */
const MARKS_HEARD = "gui-marks-heard";
/** The account this browser's marks from before the hub kept them went up to. */
const MARKS_MIGRATED = "gui-marks-migrated";

/** Keeps the page's marks on the account's hub once it answers `session_marks`. The page's own copy
 * stays in this browser's storage, which the caller writes on every change and `io.persist` writes for
 * a change from the hub, so marks of threads with no session yet, and changes the hub has not heard,
 * survive a reload. `io` reads and writes the page's marks (`read`, `write`, `persist`), reads and
 * writes storage (`load`, `save`), calls the hub (`call`), and redraws (`changed`). */
function marksSync(io) {
  let onHub = false;
  let account = null;
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

  /** Stores what the hub was last heard to hold. The page's marks are stored first: a field that
   * differs between the two then counts as unsent, and resending a change the hub has is harmless,
   * while a mark the hub dropped must not come back from a copy stored after the rows. */
  const remember = () => io.save(MARKS_HEARD, JSON.stringify({ account, rows: [...known.values()] }));
  /** The rows `remember` stored for `id`, or null when it stored none for that account. */
  function heard(id) {
    try {
      const last = JSON.parse(io.load(MARKS_HEARD));
      return last?.account === id ? new Map(last.rows.map((row) => [row.session, row])) : null;
    } catch {
      return null;
    }
  }

  function show(next, before) {
    if (!markChanges(before, next).length) return;
    io.write(withSessionMarks(io.read(), markState(next)));
    io.persist();
    io.changed();
  }
  function adopt(rows) {
    const before = mine();
    const held = heldMarks(known, before, flight);
    for (const row of rows) hasMarks(row) ? known.set(row.session, row) : known.delete(row.session);
    show(mergedMarks(before, new Map(rows.map((row) => [row.session, row])), held), before);
    remember();
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

  /** Off the hub, on sign-out and when the hub cannot keep marks. Without `keep` the page and its
   * storage forget the session marks, so a shared browser keeps no thread titles. */
  function leave(keep) {
    if (onHub && !keep) {
      io.write(withSessionMarks(io.read(), markState(new Map())));
      io.persist();
      io.save(MARKS_HEARD, null);
    }
    onHub = false;
    account = null;
    known = new Map();
    joining = null;
    epoch++;
  }

  /** On every connect to account `id`: sends what waited, and takes the hub's rows but for the fields
   * changed here since the hub was last heard, which then go up. The first connect ever also brings
   * up the marks this browser kept before the hub did. */
  async function hello(id) {
    const at = ++epoch;
    const wasOnHub = onHub;
    const start = mine();
    const last = wasOnHub ? null : heard(id);
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
    const held = heldMarks(wasOnHub ? known : (last ?? start), now, flight);
    account = id;
    known = new Map(rows.filter(hasMarks).map((row) => [row.session, row]));
    let next = mergedMarks(now, known, held, true);
    const migrating = io.load(MARKS_MIGRATED) == null;
    if (migrating) next = applyMarkChanges(next, marksMigration(now, known));
    onHub = true;
    show(next, now);
    remember();
    if (migrating) io.save(MARKS_MIGRATED, id);
    for (const pushed of joining) adopt(pushed);
    joining = null;
    await sync();
  }

  function push(rows) {
    if (joining) joining.push(rows);
    else if (onHub) {
      adopt(rows);
      sync();
    }
  }

  /** Sends the page's marks on once the hub keeps them, answering whether the hub took them all. */
  const saved = () => onHub && sync();
  return { hello, push, leave, saved };
}
