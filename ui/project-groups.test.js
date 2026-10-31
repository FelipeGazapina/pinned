import assert from "node:assert/strict";
import test from "node:test";
import { groupProjects, sessionOn } from "./project-groups.js";

const directory = { localId: "local", localName: "This Mac", names: { vps: "studio" }, online: { local: true, vps: true } };

test("a local checkout is named with this machine's name", () => {
  const localOnly = sessionOn([{ machine: "local", path: "/Users/me/app" }], directory);
  assert.equal(localOnly.machine, "local");
  assert.equal(localOnly.label, "This Mac");
  assert.deepEqual(localOnly.path, "/Users/me/app");
  assert.deepEqual(localOnly.offline, false);
  assert.deepEqual(localOnly.also, []);
});

test("a remote checkout uses that machine's name", () => {
  const remoteOnly = sessionOn([{ machine: "vps", path: "/srv/app" }], directory);
  assert.equal(remoteOnly.machine, "vps");
  assert.equal(remoteOnly.label, "studio");
});

test("the open thread stays the start and the other machine is counted", () => {
  const two = sessionOn(
    [
      { machine: "local", path: "/Users/me/app", thread: true },
      { machine: "vps", path: "/srv/app", thread: false },
    ],
    directory,
  );
  assert.equal(two.machine, "local");
  assert.equal(two.label, "This Mac +1");
  assert.deepEqual(two.path, "/Users/me/app");
  assert.deepEqual(two.also, ["studio"]);
});

test("an offline machine keeps its checkout and says so", () => {
  const offline = sessionOn(
    [{ machine: "vps", path: "/srv/app", thread: true }],
    { localId: "local", localName: "This Mac", names: { vps: "studio" }, online: { vps: false } },
  );
  assert.equal(offline.machine, "vps");
  assert.equal(offline.label, "studio (offline)");
  assert.deepEqual(offline.offline, true);
});

test("an offline open thread stays ahead of an online copy", () => {
  const offlineTab = sessionOn(
    [
      { machine: "vps", path: "/srv/app", thread: true },
      { machine: "local", path: "/Users/me/app", thread: false },
    ],
    { localId: "local", localName: "This Mac", names: { vps: "studio" }, online: { vps: false, local: true } },
  );
  assert.equal(offlineTab.machine, "vps");
  assert.equal(offlineTab.label, "studio (offline) +1");
  assert.deepEqual(offlineTab.path, "/srv/app");
  assert.deepEqual(offlineTab.also, ["This Mac"]);
});

test("the first open thread wins when an online checkout is listed ahead of it", () => {
  const here = sessionOn(
    [
      { machine: "local", path: "/Users/me/app", thread: false },
      { machine: "vps", path: "/srv/app", thread: true },
    ],
    { localId: "local", localName: "This Mac", names: { vps: "studio" }, online: { vps: false, local: true } },
  );
  assert.equal(here.machine, "vps");
  assert.equal(here.label, "studio (offline) +1");
  assert.deepEqual(here.path, "/srv/app");
  assert.deepEqual(here.offline, true);
  assert.deepEqual(here.also, ["This Mac"]);
});

test("a second folder on the start machine is not another machine", () => {
  const here = sessionOn(
    [
      { machine: "local", path: "/Users/me/app", thread: true },
      { machine: "local", path: "/Users/me/other", thread: false },
    ],
    directory,
  );
  assert.equal(here.machine, "local");
  assert.equal(here.label, "This Mac");
  assert.deepEqual(here.path, "/Users/me/app");
  assert.deepEqual(here.also, []);
});

test("another machine is counted once when it has two folders", () => {
  const here = sessionOn(
    [
      { machine: "local", path: "/Users/me/app", thread: true },
      { machine: "vps", path: "/srv/app", thread: false },
      { machine: "vps", path: "/srv/other", thread: false },
    ],
    directory,
  );
  assert.equal(here.machine, "local");
  assert.equal(here.label, "This Mac +1");
  assert.deepEqual(here.also, ["studio"]);
});

test("a machine with no name is called machine", () => {
  const here = sessionOn(
    [{ machine: "vps", path: "/srv/app" }],
    { localId: "local", localName: "This Mac", names: {}, online: { vps: true } },
  );
  assert.equal(here.machine, "vps");
  assert.equal(here.label, "machine");
});

test("a remote project with no thread is on the list the main machine shows", () => {
  const named = { localId: "local", localName: "This Mac", names: { mac: "desk", vps: "studio" }, online: { mac: true, vps: true } };
  const groups = groupProjects({
    threads: [{ machine: "mac", path: "/home/app", name: "app", repo: "github.com/acme/app", thread: { id: "tab-1" } }],
    listed: {
      vps: [
        { path: "/srv/app", name: "app", repo: "github.com/acme/app" },
        { path: "/srv/notes", name: "notes", repo: null },
        { path: "/srv/other-notes", name: "notes", repo: "github.com/acme/other" },
      ],
    },
    grouping: true,
  });
  const notes = groups.find((g) => sessionOn(g.checkouts, named).path === "/srv/notes");
  assert.ok(notes, `notes is absent from the project list: ${groups.map((g) => sessionOn(g.checkouts, named).path).join(", ")}`);
  const here = sessionOn(notes.checkouts, named);
  assert.equal(here.machine, "vps");
  assert.equal(here.label, "studio");
  assert.deepEqual(here.path, "/srv/notes");
  assert.deepEqual(here.offline, false);
  assert.deepEqual(here.also, []);
  assert.equal(notes.tabs.length, 0);
  assert.equal(Object.hasOwn(notes, "machine"), false);
  assert.equal(Object.hasOwn(notes, "path"), false);
  assert.equal(
    groups.filter((g) => g.name === "notes").length,
    2,
    "two projects with the same folder name collapsed into one",
  );
  assert.equal(groups.filter((g) => g.key === "repo:github.com/acme/app").length, 1);
  const app = groups.find((g) => g.key === "repo:github.com/acme/app");
  const start = sessionOn(app.checkouts, named);
  assert.equal(start.machine, "mac");
  assert.equal(start.label, "desk +1");
  assert.deepEqual(start.path, "/home/app");
  assert.deepEqual(start.also, ["studio"]);
  assert.equal(app.checkouts.length, 2);
  assert.equal(app.tab.id, "tab-1");
});

test("grouping off keeps each machine's checkout of one repository", () => {
  const named = { localId: "local", localName: "This Mac", names: { mac: "desk", vps: "studio" }, online: { mac: true, vps: true } };
  const groups = groupProjects({
    threads: [],
    listed: {
      mac: [{ path: "/home/app", name: "app", repo: "github.com/acme/app" }],
      vps: [{ path: "/srv/app", name: "app", repo: "github.com/acme/app" }],
    },
    grouping: false,
  });
  assert.deepEqual(
    groups.map((g) => {
      const here = sessionOn(g.checkouts, named);
      return `${here.machine}:${here.path}`;
    }).sort(),
    ["mac:/home/app", "vps:/srv/app"],
  );
  assert.deepEqual(
    groups.map((g) => sessionOn(g.checkouts, named).label).sort(),
    ["desk", "studio"],
  );
});
