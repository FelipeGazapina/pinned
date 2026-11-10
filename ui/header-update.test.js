import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

function load() {
  const source = readFileSync(new URL("./main.js", import.meta.url), "utf8");
  const start = source.indexOf("async function checkUpdate");
  const end = source.indexOf('for (const button of document.querySelectorAll(".update"))');
  if (start < 0 || end < 0) throw new Error("installUpdate is missing");
  return new Function("invoke", "document", "active", "guiOn", "tell", "esc", "setNewestVersion", `${source.slice(start, end)}\nreturn { installUpdate, headerUpdateResult };`);
}

async function press(status, { gui = false, throws = null } = {}) {
  const button = { disabled: false, textContent: "Update to v0.16.0", closest: () => null };
  const term = { text: "", write(chunk) { this.text += chunk; } };
  const told = [];
  const newest = [];
  const document = { body: { dataset: {} }, querySelectorAll: () => [button] };
  const invoke = async (cmd) => {
    if (cmd === "install_update") {
      if (throws) throw throws;
      return status;
    }
    if (cmd === "check_update") return "0.17.0";
    throw new Error(cmd);
  };
  const esc = (text) => String(text).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  const api = load()(invoke, document, { term }, gui ? () => true : undefined, gui ? (html, ms) => told.push([html, ms]) : undefined, esc, (v) => newest.push(v));
  await api.installUpdate();
  return { button, term: term.text, told, newest };
}

test("a failed install shows the reason and can be pressed again", async () => {
  const { button, term } = await press({ state: "failed", reason: "checksum mismatch" });
  assert.equal(button.disabled, false);
  assert.equal(button.textContent, "Update to v0.17.0");
  assert.equal(term, "\r\n\x1b[31m[update failed: checksum mismatch]\x1b[0m\r\n");
});

test("a refused install shows the reason and can be pressed again", async () => {
  const { button, term } = await press({ state: "cannot", reason: "update by hand" });
  assert.equal(button.disabled, false);
  assert.equal(button.textContent, "Update to v0.17.0");
  assert.equal(term, "\r\n\x1b[31m[update failed: update by hand]\x1b[0m\r\n");
});

test("an up to date install refreshes the button and writes nothing", async () => {
  const { button, term } = await press({ state: "up_to_date", version: "0.16.0" });
  assert.equal(button.disabled, false);
  assert.equal(button.textContent, "Update to v0.17.0");
  assert.equal(term, "");
});

test("an empty install result refreshes the button and writes nothing", async () => {
  const { button, term } = await press(undefined);
  assert.equal(button.disabled, false);
  assert.equal(button.textContent, "Update to v0.17.0");
  assert.equal(term, "");
});

test("in the app view a failed install says why in a notice, since the terminal is hidden", async () => {
  const { button, term, told } = await press({ state: "failed", reason: "checksum <mismatch>" }, { gui: true });
  assert.deepEqual(told, [["Update failed: checksum &lt;mismatch&gt;", 8000]]);
  assert.equal(term, "");
  assert.equal(button.textContent, "Update to v0.17.0");
});

test("in the app view an install that throws says why in a notice", async () => {
  const { told, term } = await press(undefined, { gui: true, throws: "no network" });
  assert.deepEqual(told, [["Update failed: no network", 8000]]);
  assert.equal(term, "");
});

test("an update check that finds a version tells the machines page what is newest", async () => {
  const { newest } = await press({ state: "up_to_date", version: "0.16.0" });
  assert.deepEqual(newest, ["0.17.0"]);
});
