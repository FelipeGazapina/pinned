import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const here = (value) => JSON.parse(JSON.stringify(value));

// The file's consts stay in its script scope, so a bridge hands its API out.
const pure = () => {
  const context = {};
  vm.createContext(context);
  vm.runInContext(`${read("auto-archive.js")}\nthis.api = { AA_CHOICES, AA_DEFAULT_SETTINGS, autoArchiveLabel, autoArchiveOptions, autoArchiveMinutes, mergeAutoArchive, autoArchiveWrites };`, context);
  return context.api;
};

test("the page offers never, ten minutes, half an hour, two hours and a day", () => {
  const { AA_CHOICES, autoArchiveOptions } = pure();
  assert.deepEqual(here(AA_CHOICES).map(([minutes]) => minutes), [null, 10, 30, 120, 1440]);
  assert.equal(
    autoArchiveOptions(30),
    '<option value="never">Never</option><option value="10">After 10 minutes</option><option value="30" selected>After 30 minutes</option><option value="120">After 2 hours</option><option value="1440">After 24 hours</option>',
  );
  assert.match(autoArchiveOptions(null), /<option value="never" selected>Never<\/option>/);
});

test("a value a machine holds that the page does not offer is listed as it is, and selected", () => {
  const { autoArchiveOptions, autoArchiveLabel } = pure();
  assert.match(autoArchiveOptions(45), /<option value="45" selected>After 45 minutes<\/option>/);
  assert.equal(autoArchiveLabel(180), "After 3 hours");
  assert.equal(autoArchiveLabel(1), "After 1 minute");
});

test("a select's never is null and the rest are minutes", () => {
  const { autoArchiveMinutes } = pure();
  assert.equal(autoArchiveMinutes("never"), null);
  assert.equal(autoArchiveMinutes("120"), 120);
});

test("the account's view is the first machine that answered, and the defaults when none did", () => {
  const { mergeAutoArchive } = pure();
  assert.deepEqual(here(mergeAutoArchive([])), { workers: 30, sessions: null });
  const view = mergeAutoArchive([
    { id: "mb", online: false, settings: null },
    { id: "mm", online: true, settings: { workers: 10, sessions: 120 } },
    { id: "vps", online: true, settings: { workers: null, sessions: null } },
  ]);
  assert.deepEqual(here(view), { workers: 10, sessions: 120 });
  assert.deepEqual(here(mergeAutoArchive([{ id: "mm", online: true, settings: { workers: null, sessions: null } }])), { workers: null, sessions: null }, "never is an answer, not a gap");
});

test("a change is written to every online machine that can take it", () => {
  const { autoArchiveWrites } = pure();
  const machines = [
    { id: "mb", online: true, update: false },
    { id: "mm", online: true, update: true },
    { id: "vps", online: false, update: false },
    { id: "mini", online: true, update: false },
  ];
  assert.deepEqual(here(autoArchiveWrites({ workers: 120, sessions: null }, machines)), [
    { machine: "mb", settings: { workers: 120, sessions: null } },
    { machine: "mini", settings: { workers: 120, sessions: null } },
  ]);
});
