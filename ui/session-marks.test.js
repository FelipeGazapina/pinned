import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const context = {};
vm.createContext(context);
vm.runInContext(readFileSync(new URL("./marks.js", import.meta.url), "utf8"), context);

// Objects built inside the vm carry its prototypes; a JSON round trip makes them comparable here.
const here = (value) => (value === undefined ? value : JSON.parse(JSON.stringify(value)));
const rowsOf = (state) => here(Object.fromEntries(context.markRows(state)));
const byRow = (list) => new Map(list.map((row) => [row.session, row]));
const changes = (before, after) => here(context.markChanges(context.markRows(before), context.markRows(after)));

const filed = () => ({
  sideMarks: {
    pins: ["session:s2", "session:s1"],
    order: ["session:s3"],
    settled: { "session:s4": 1700000100.5 },
    unsettled: { "session:s3": 1700000200 },
    snoozed: { "session:s5": { until: 1700090000, at: 1700000300, limit: 1700090000 } },
    titles: { "session:s1": "Fix login" },
    noAutoSettle: { "session:s2": true },
  },
  seenAt: { "session:s1": 1700000400, "session:s3": 1700000500 },
  archived: [{ machine: "m1", cwd: "/w/site", name: "Old work", project: "site", at: 1700000600, created: 1699990000, title: "Old work", pinned: true, session: "s6" }],
});
const blank = () => ({ sideMarks: { pins: [], order: [], settled: {}, unsettled: {}, snoozed: {}, titles: {}, noAutoSettle: {} }, seenAt: {}, archived: [] });

test("the sidebar's marks become one hub row per session and come back the same", () => {
  assert.deepEqual(rowsOf(filed()), {
    s1: { session: "s1", title: "Fix login", pin: 1, seen: 1700000400 },
    s2: { session: "s2", pin: 0, no_auto_settle: true },
    s3: { session: "s3", order: 0, unsettled: 1700000200, seen: 1700000500 },
    s4: { session: "s4", settled: 1700000100.5 },
    s5: { session: "s5", snoozed: { until: 1700090000, at: 1700000300, limit: 1700090000 } },
    s6: { session: "s6", archived: { machine: "m1", cwd: "/w/site", name: "Old work", project: "site", at: 1700000600, created: 1699990000, title: "Old work", pinned: true } },
  });
  assert.deepEqual(here(context.markState(context.markRows(filed()))), filed());
});

test("a rename is one title change", () => {
  const renamed = filed();
  renamed.sideMarks.titles["session:s1"] = "x";
  assert.deepEqual(changes(filed(), renamed), [{ session: "s1", mark: { title: "x" } }]);
});

test("reordering the pins moves each pin that changed place", () => {
  const reordered = filed();
  reordered.sideMarks.pins = ["session:s1", "session:s2"];
  assert.deepEqual(changes(filed(), reordered), [
    { session: "s1", mark: { pin: 0 } },
    { session: "s2", mark: { pin: 1 } },
  ]);
});

test("closing a thread clears its row, and the pins after it move up", () => {
  const closed = filed();
  for (const m of ["settled", "unsettled", "snoozed", "titles", "noAutoSettle"]) delete closed.sideMarks[m]["session:s2"];
  for (const m of ["pins", "order"]) closed.sideMarks[m] = closed.sideMarks[m].filter((k) => k !== "session:s2");
  assert.deepEqual(changes(filed(), closed), [
    { session: "s2", mark: "clear" },
    { session: "s1", mark: { pin: 0 } },
  ]);
});

test("unsetting a mark sends it as null, and auto-settle back on as false", () => {
  const woken = filed();
  delete woken.sideMarks.snoozed["session:s5"];
  woken.sideMarks.titles["session:s5"] = "Waiting on CI";
  delete woken.sideMarks.noAutoSettle["session:s2"];
  woken.seenAt["session:s1"] = 0;
  assert.deepEqual(changes(filed(), woken), [
    { session: "s1", mark: { seen: null } },
    { session: "s5", mark: { title: "Waiting on CI" } },
    { session: "s5", mark: { snoozed: null } },
    { session: "s2", mark: { no_auto_settle: false } },
  ]);
});

test("a thread with no session yet has no row: its archive entry is dropped and it takes no pin place", () => {
  const early = blank();
  early.sideMarks.pins = ["local:3", "session:s1"];
  early.sideMarks.titles = { "local:3": "Draft" };
  early.archived = [
    { machine: "local", cwd: "/w/a", session: null, title: undefined, pinned: false, name: "No session", project: "a", at: 10 },
    { machine: "local", cwd: "/w/b", session: "s7", title: undefined, pinned: false, name: "Has one", project: "b", at: 20, created: undefined },
  ];
  assert.deepEqual(rowsOf(early), {
    s1: { session: "s1", pin: 0 },
    s7: { session: "s7", archived: { machine: "local", cwd: "/w/b", name: "Has one", project: "b", at: 20 } },
  });
});

test("text longer than the hub keeps is cut to 500 characters", () => {
  const long = blank();
  long.sideMarks.titles = { "session:s1": "é".repeat(1200) };
  assert.equal(rowsOf(long).s1.title, "é".repeat(500));
});

test("bringing stored marks up keeps the hub's own and adds what it lacks, pins after its pins", () => {
  const local = byRow([
    { session: "s1", title: "Local name", pin: 0, seen: 5 },
    { session: "s7", settled: 100, no_auto_settle: true },
  ]);
  const hub = byRow([
    { session: "s1", title: "Hub name", seen: 9 },
    { session: "s9", pin: 0 },
  ]);
  assert.deepEqual(here(context.marksMigration(local, hub)), [
    { session: "s7", mark: { settled: 100 } },
    { session: "s7", mark: { no_auto_settle: true } },
    { session: "s1", mark: { pin: 1 } },
  ]);
});

/** A hub that keeps rows as crates/core/src/marks.rs does, answers each call when told to, and pushes
 * the touched rows to every device right before it answers the caller. */
function fakeHub() {
  const hub = { rows: new Map(), sent: [], inbox: [], devices: [], auto: true, down: false, old: false, refuse: null };
  const apply = (changes) => {
    const touched = [];
    for (const { session, mark } of changes) {
      if (!touched.includes(session)) touched.push(session);
      const row = mark === "clear" ? { session } : { ...(hub.rows.get(session) ?? { session }) };
      if (mark !== "clear") for (const [field, value] of Object.entries(mark)) value === null || value === false ? delete row[field] : (row[field] = value);
      hub.rows.set(session, row);
    }
    const rows = touched.map((session) => hub.rows.get(session));
    for (const row of rows) if (Object.keys(row).length === 1) hub.rows.delete(row.session);
    return here(rows);
  };
  hub.answer = () => {
    for (const { call, resolve, reject } of hub.inbox.splice(0)) {
      if (hub.down) reject("lost the connection to the hub");
      else if ((hub.old && call.cmd === "session_marks") || hub.refuse === call.cmd) reject(`this hub runs pinned 0.17.20, which can't answer that: unknown variant \`${call.cmd}\``);
      else if (call.cmd === "session_marks") resolve(here([...hub.rows.values()]));
      else {
        hub.sent.push(here(call.changes));
        const touched = apply(call.changes);
        for (const device of hub.devices) device.marks.push(here(touched));
        resolve(touched);
      }
    }
  };
  hub.call = (call) =>
    new Promise((resolve, reject) => {
      hub.inbox.push({ call, resolve, reject });
      if (hub.auto) Promise.resolve().then(hub.answer);
    });
  return hub;
}

const ACCOUNT = "acct-1";
/** A device of the account, its page marks read from `storage` as the sidebar reads them on load. Each
 * `edit` changes the page's marks and stores them as the sidebar's `storeMarks` does. */
function device(hub, storage = {}) {
  const stored = (key, empty) => (key in storage ? JSON.parse(storage[key]) : empty);
  const d = { storage, renders: 0 };
  d.state = here({ sideMarks: context.storedSideMarks(stored("gui-marks", null)), seenAt: stored("gui-seen", {}), archived: stored("gui-archived", []) });
  const persist = () => Object.assign(d.storage, { "gui-marks": JSON.stringify(d.state.sideMarks), "gui-archived": JSON.stringify(d.state.archived), "gui-seen": JSON.stringify(d.state.seenAt) });
  d.marks = context.marksSync({
    read: () => d.state,
    write: (next) => (d.state = here(next)),
    persist,
    load: (key) => d.storage[key] ?? null,
    save: (key, value) => (value === null ? delete d.storage[key] : (d.storage[key] = value)),
    call: (call) => hub.call(call),
    changed: () => d.renders++,
  });
  d.edit = (change) => {
    change(d.state);
    persist();
    return d.marks.saved();
  };
  /** The app quits: the page is gone and hears nothing more, and its storage stays. */
  d.quit = () => hub.devices.splice(hub.devices.indexOf(d), 1);
  hub.devices.push(d);
  return d;
}
const stored = (storage) => ({ sideMarks: JSON.parse(storage["gui-marks"]), seenAt: JSON.parse(storage["gui-seen"]), archived: JSON.parse(storage["gui-archived"]) });
const quiet = async () => {
  for (let i = 0; i < 20; i++) await new Promise((resolve) => setTimeout(resolve, 0));
};

test("a connect shows the hub's marks, and a push from another device lands without being sent back", async () => {
  const hub = fakeHub();
  hub.rows = byRow([{ session: "s1", title: "A" }]);
  const mine = device(hub);
  const other = device(hub);
  await mine.marks.hello(ACCOUNT);
  await other.marks.hello(ACCOUNT);
  assert.deepEqual(mine.state.sideMarks.titles, { "session:s1": "A" });

  assert.equal(await other.edit((state) => (state.sideMarks.titles["session:s1"] = "B")), true);
  await quiet();
  assert.deepEqual(mine.state.sideMarks.titles, { "session:s1": "B" });
  assert.equal(mine.renders, 2);
  assert.deepEqual(hub.sent, [[{ session: "s1", mark: { title: "B" } }]]);
  assert.deepEqual(stored(mine.storage).sideMarks.titles, { "session:s1": "B" });
});

test("a push that comes while a connect waits for the rows is not lost to the rows it read before", async () => {
  const hub = fakeHub();
  hub.rows = byRow([{ session: "s1", title: "A" }]);
  const mine = device(hub);
  const other = device(hub);
  await other.marks.hello(ACCOUNT);
  hub.auto = false;
  const joined = mine.marks.hello(ACCOUNT);
  await quiet();
  const [ask] = hub.inbox.splice(0);
  const read = here([...hub.rows.values()]);
  other.edit((state) => (state.sideMarks.titles["session:s1"] = "B"));
  await quiet();
  hub.answer();
  ask.resolve(read);
  await joined;
  assert.deepEqual(mine.state.sideMarks.titles, { "session:s1": "B" });
  assert.deepEqual(hub.inbox, []);
});

test("a change made while the hub is unreachable goes up on the next connect", async () => {
  const hub = fakeHub();
  const mine = device(hub);
  await mine.marks.hello(ACCOUNT);
  hub.down = true;
  assert.equal(await mine.edit((state) => (state.sideMarks.titles["session:s1"] = "Offline rename")), false);
  assert.deepEqual(hub.sent, []);

  hub.down = false;
  await mine.marks.hello(ACCOUNT);
  assert.deepEqual(hub.sent, [[{ session: "s1", mark: { title: "Offline rename" } }]]);
  assert.deepEqual(here(hub.rows.get("s1")), { session: "s1", title: "Offline rename" });
  assert.deepEqual(mine.state.sideMarks.titles, { "session:s1": "Offline rename" });
});

test("a rename made while the hub is unreachable reaches the hub after the app restarts and reconnects", async () => {
  const hub = fakeHub();
  hub.rows = byRow([{ session: "s1", title: "Before", pin: 0 }]);
  const mine = device(hub);
  const other = device(hub);
  await mine.marks.hello(ACCOUNT);
  await other.marks.hello(ACCOUNT);
  hub.down = true;
  assert.equal(await mine.edit((state) => (state.sideMarks.titles["session:s1"] = "Offline rename")), false);
  mine.quit();

  hub.down = false;
  const again = device(hub, mine.storage);
  assert.deepEqual(again.state.sideMarks.titles, { "session:s1": "Offline rename" });
  await again.marks.hello(ACCOUNT);
  await quiet();
  assert.deepEqual(hub.sent, [[{ session: "s1", mark: { title: "Offline rename" } }]]);
  assert.deepEqual(here(hub.rows.get("s1")), { session: "s1", title: "Offline rename", pin: 0 });
  assert.deepEqual(other.state.sideMarks.titles, { "session:s1": "Offline rename" });
  assert.deepEqual(again.state.sideMarks.pins, ["session:s1"]);
});

test("a mark cleared on another device does not come back from this device's stored copy", async () => {
  const hub = fakeHub();
  hub.rows = byRow([{ session: "s1", title: "Kept", pin: 0 }, { session: "s2", settled: 100 }]);
  const one = device(hub);
  const two = device(hub);
  await one.marks.hello(ACCOUNT);
  await two.marks.hello(ACCOUNT);
  one.quit();

  await two.edit((state) => {
    state.sideMarks.pins = [];
    delete state.sideMarks.settled["session:s2"];
  });
  assert.deepEqual(here([...hub.rows.values()]), [{ session: "s1", title: "Kept" }]);

  const again = device(hub, one.storage);
  assert.deepEqual(again.state.sideMarks.pins, ["session:s1"]);
  await again.marks.hello(ACCOUNT);
  await quiet();
  assert.deepEqual(again.state.sideMarks.pins, []);
  assert.deepEqual(again.state.sideMarks.settled, {});
  assert.deepEqual(again.state.sideMarks.titles, { "session:s1": "Kept" });
  assert.deepEqual(stored(again.storage).sideMarks.pins, []);
  assert.deepEqual(hub.sent, [[{ session: "s2", mark: "clear" }, { session: "s1", mark: { pin: null } }]]);
  assert.deepEqual(here([...hub.rows.values()]), [{ session: "s1", title: "Kept" }]);
});

test("a pin and a title on a thread with no session yet survive a reload on the hub, and stay off it", async () => {
  const hub = fakeHub();
  const mine = device(hub);
  await mine.marks.hello(ACCOUNT);
  await mine.edit((state) => {
    state.sideMarks.pins = ["local:tab-1a2b-3", "local:tab-7"];
    state.sideMarks.titles = { "local:tab-1a2b-3": "Shell" };
  });
  mine.quit();

  const again = device(hub, mine.storage);
  await again.marks.hello(ACCOUNT);
  await quiet();
  assert.deepEqual(again.state.sideMarks.pins, ["local:tab-1a2b-3"]);
  assert.deepEqual(again.state.sideMarks.titles, { "local:tab-1a2b-3": "Shell" });
  assert.deepEqual(hub.sent, []);
  assert.deepEqual(here([...hub.rows.values()]), []);
});

test("stored marks go up on the first connect, and storage keeps its copy", async () => {
  const hub = fakeHub();
  hub.rows = byRow([{ session: "s1", title: "Hub name" }]);
  const local = blank();
  local.sideMarks.titles = { "session:s1": "Local name" };
  local.sideMarks.pins = ["session:s2"];
  const mine = device(hub, { "gui-marks": JSON.stringify(local.sideMarks), "gui-archived": "[]", "gui-seen": "{}", "gui-folded": "[]" });

  hub.refuse = "mark_sessions";
  await mine.marks.hello(ACCOUNT);
  assert.deepEqual(hub.sent, []);

  hub.refuse = null;
  await mine.marks.hello(ACCOUNT);
  assert.deepEqual(hub.sent, [[{ session: "s2", mark: { pin: 0 } }]]);
  assert.deepEqual(mine.state.sideMarks.titles, { "session:s1": "Hub name" });
  assert.deepEqual(mine.state.sideMarks.pins, ["session:s2"]);
  assert.deepEqual(stored(mine.storage).sideMarks.titles, { "session:s1": "Hub name" });
  assert.deepEqual(stored(mine.storage).sideMarks.pins, ["session:s2"]);
  assert.equal(mine.storage["gui-folded"], "[]");
  assert.equal(mine.storage["gui-marks-migrated"], ACCOUNT);
});

test("the move up runs once: neither a mark cleared since nor another account gets this browser's marks", async () => {
  const hub = fakeHub();
  const local = blank();
  local.sideMarks.pins = ["session:s2"];
  const mine = device(hub, { "gui-marks": JSON.stringify(local.sideMarks) });
  await mine.marks.hello(ACCOUNT);
  assert.deepEqual(hub.sent, [[{ session: "s2", mark: { pin: 0 } }]]);
  mine.quit();

  const other = device(hub);
  await other.marks.hello(ACCOUNT);
  await other.edit((state) => (state.sideMarks.pins = []));
  const again = device(hub, mine.storage);
  await again.marks.hello(ACCOUNT);
  await quiet();
  assert.deepEqual(again.state.sideMarks.pins, []);
  assert.deepEqual(hub.sent, [[{ session: "s2", mark: { pin: 0 } }], [{ session: "s2", mark: "clear" }]]);

  const elsewhere = fakeHub();
  elsewhere.rows = byRow([{ session: "s9", title: "Theirs" }]);
  const stale = blank();
  stale.sideMarks.pins = ["session:s3"];
  const switched = device(elsewhere, { ...again.storage, "gui-marks": JSON.stringify(stale.sideMarks) });
  await switched.marks.hello("acct-2");
  await quiet();
  assert.deepEqual(elsewhere.sent, []);
  assert.deepEqual(switched.state.sideMarks.pins, []);
  assert.deepEqual(switched.state.sideMarks.titles, { "session:s9": "Theirs" });
  assert.equal(switched.storage["gui-marks-migrated"], ACCOUNT);
});

test("a hub too old to keep marks leaves them in storage", async () => {
  const hub = fakeHub();
  hub.old = true;
  const mine = device(hub, { "gui-marks": "{}" });
  await mine.marks.hello(ACCOUNT);
  assert.equal(mine.edit((state) => (state.sideMarks.titles["session:s1"] = "Stays here")), false);
  await quiet();
  assert.deepEqual(hub.inbox, []);
  assert.deepEqual(hub.sent, []);
  assert.deepEqual(stored(mine.storage).sideMarks.titles, { "session:s1": "Stays here" });
  assert.equal(mine.storage["gui-marks-migrated"], undefined);
});

test("a hub that loses the calls on a reconnect sends the page back to storage, marks and all", async () => {
  const hub = fakeHub();
  hub.rows = byRow([{ session: "s1", pin: 0 }]);
  const mine = device(hub);
  await mine.marks.hello(ACCOUNT);
  hub.old = true;
  await mine.marks.hello(ACCOUNT);
  assert.equal(mine.marks.saved(), false);
  assert.deepEqual(stored(mine.storage).sideMarks, { pins: ["session:s1"], order: [], settled: {}, unsettled: {}, snoozed: {}, titles: {}, noAutoSettle: {} });
});

test("an undo made while the change is on its way survives the hub's echo of it", async () => {
  const hub = fakeHub();
  const mine = device(hub);
  await mine.marks.hello(ACCOUNT);
  hub.auto = false;

  const sent = mine.edit((state) => (state.sideMarks.pins = ["session:s1"]));
  await quiet();
  mine.edit((state) => (state.sideMarks.pins = []));
  hub.answer();
  assert.deepEqual(mine.state.sideMarks.pins, []);
  await quiet();
  hub.answer();
  assert.equal(await sent, true);

  assert.deepEqual(hub.sent, [[{ session: "s1", mark: { pin: 0 } }], [{ session: "s1", mark: "clear" }]]);
  assert.deepEqual([...hub.rows.keys()], []);
  assert.deepEqual(mine.state.sideMarks.pins, []);
});

test("two devices that pin at once end with the same order", async () => {
  const hub = fakeHub();
  const one = device(hub);
  const two = device(hub);
  await one.marks.hello(ACCOUNT);
  await two.marks.hello(ACCOUNT);
  one.edit((state) => (state.sideMarks.pins = ["session:s1"]));
  two.edit((state) => (state.sideMarks.pins = ["session:s2"]));
  await quiet();
  assert.deepEqual(one.state.sideMarks.pins, ["session:s1", "session:s2"]);
  assert.deepEqual(two.state.sideMarks.pins, ["session:s1", "session:s2"]);
  assert.deepEqual(here([...hub.rows.values()]), [{ session: "s1", pin: 0 }, { session: "s2", pin: 1 }]);
});

test("signing out of the app keeps the hub's marks in storage, and of the web page keeps none", async () => {
  const hub = fakeHub();
  hub.rows = byRow([{ session: "s1", title: "Kept", seen: 7 }]);
  const web = device(hub);
  await web.marks.hello(ACCOUNT);
  web.marks.leave(false);
  assert.deepEqual(web.state, blank());
  assert.deepEqual(stored(web.storage), blank());
  assert.equal(web.storage["gui-marks-heard"], undefined);
  assert.equal(web.marks.saved(), false);

  const mine = device(hub);
  await mine.marks.hello(ACCOUNT);
  mine.marks.leave(true);
  assert.deepEqual(stored(mine.storage), {
    sideMarks: { pins: [], order: [], settled: {}, unsettled: {}, snoozed: {}, titles: { "session:s1": "Kept" }, noAutoSettle: {} },
    seenAt: { "session:s1": 7 },
    archived: [],
  });
  assert.equal(mine.marks.saved(), false);
});
