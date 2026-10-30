import assert from "node:assert/strict";
import test from "node:test";
import { groupProjects } from "./project-groups.js";

test("a remote project with no thread is on the list the main machine shows", () => {
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
  const notes = groups.find((g) => g.path === "/srv/notes");
  assert.ok(notes, `notes is absent from the project list: ${groups.map((g) => g.path).join(", ")}`);
  assert.equal(notes.machine, "vps");
  assert.equal(notes.tabs.length, 0);
  assert.equal(
    groups.filter((g) => g.name === "notes").length,
    2,
    "two projects with the same folder name collapsed into one",
  );
  assert.equal(groups.filter((g) => g.key === "repo:github.com/acme/app").length, 1);
});

test("grouping off keeps each machine's checkout of one repository", () => {
  const groups = groupProjects({
    threads: [],
    listed: {
      mac: [{ path: "/home/app", name: "app", repo: "github.com/acme/app" }],
      vps: [{ path: "/srv/app", name: "app", repo: "github.com/acme/app" }],
    },
    grouping: false,
  });
  assert.deepEqual(
    groups.map((g) => `${g.machine}:${g.path}`).sort(),
    ["mac:/home/app", "vps:/srv/app"],
  );
});
