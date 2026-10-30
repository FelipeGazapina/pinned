import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const route = new Function(`${readFileSync(new URL("./link-route.js", import.meta.url), "utf8")}; return routeAnchorClick;`)();

const link = (over) => ({
  href: "https://example.com/notes",
  protocol: "https:",
  download: false,
  pr: "",
  cardPr: "",
  inPanel: false,
  openUrl: "",
  meta: false,
  ctrl: false,
  ...over,
});

test("an http link outside the panel opens through the panel", () => {
  assert.deepEqual(route(link()), { url: "https://example.com/notes", outside: false });
});

test("a meta or ctrl click still asks for the system browser", () => {
  assert.deepEqual(route(link({ meta: true })), { url: "https://example.com/notes", outside: true });
  assert.deepEqual(route(link({ href: "http://127.0.0.1:9/a", protocol: "http:", ctrl: true })), { url: "http://127.0.0.1:9/a", outside: true });
});

test("the panel, downloads, pull request links, and the open-in-browser control stay on their own path", () => {
  assert.equal(route(link({ inPanel: true })), null);
  assert.equal(route(link({ download: true })), null);
  assert.equal(route(link({ pr: "https://github.com/example/repo/pull/7" })), null);
  assert.equal(route(link({ cardPr: "https://github.com/example/repo/pull/7" })), null);
  assert.deepEqual(route(link({ openUrl: "https://example.com/out" })), { openOutside: "https://example.com/out" });
});

test("in-app, blob, and data urls are not sent to the panel", () => {
  assert.equal(route(link({ href: "tauri://localhost/index.html", protocol: "tauri:" })), null);
  assert.equal(route(link({ href: "blob:tauri://localhost/uuid", protocol: "blob:" })), null);
  assert.equal(route(link({ href: "data:text/plain,hi", protocol: "data:" })), null);
});
