// node --test scripts/teams-dev/hub-client.test.mjs
// TEAMS_DEV_HUB must name this machine over ws/http with no userinfo; anything else is refused with a
// message that names the variable and never its value (a value could hold a credential).
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { checkHub } from "./hub-client.mjs";

const CLIENT = path.join(path.dirname(fileURLToPath(import.meta.url)), "hub-client.mjs");

test("a loopback address is accepted, as the http address the client talks to", () => {
  assert.equal(checkHub("http://127.0.0.1:8811"), "http://127.0.0.1:8811");
  assert.equal(checkHub("http://localhost:8811/"), "http://localhost:8811");
  assert.equal(checkHub("http://LOCALHOST:8811"), "http://localhost:8811");
  assert.equal(checkHub("http://[::1]:8811"), "http://[::1]:8811");
  assert.equal(checkHub("ws://127.0.0.1:8811"), "http://127.0.0.1:8811");
  assert.equal(checkHub("ws://[::1]:8811"), "http://[::1]:8811");
  assert.equal(checkHub("ws://localhost"), "http://localhost");
});

test("a public host is refused", () => {
  for (const address of ["http://example.com", "http://8.8.8.8:8811", "ws://hub.example.com:8811", "http://127.0.0.1.example.com", "http://localhost.example.com", "http://0.0.0.0:8811"]) {
    assert.throws(() => checkHub(address), /TEAMS_DEV_HUB .*loopback/, address);
  }
});

test("an address with userinfo is refused, even on loopback", () => {
  for (const address of ["http://ana:s3cret@127.0.0.1:8811", "http://token@localhost:8811", "ws://:s3cret@[::1]:8811", "http://ana:s3cret@example.com"]) {
    assert.throws(() => checkHub(address), /TEAMS_DEV_HUB .*user/, address);
  }
});

test("a scheme other than ws or http is refused, a wss remote included", () => {
  for (const address of ["wss://hub.example.com", "wss://hub.example.com:443/ws/client", "wss://127.0.0.1:8811", "https://localhost:8811", "ftp://127.0.0.1", "file:///etc/passwd"]) {
    assert.throws(() => checkHub(address), /TEAMS_DEV_HUB .*(ws|http)/, address);
  }
});

test("something that is not an address is refused", () => {
  for (const address of ["not a url", "127.0.0.1:8811", "//localhost"]) {
    assert.throws(() => checkHub(address), /TEAMS_DEV_HUB/, address);
  }
});

test("a refusal never contains the value", () => {
  for (const address of ["http://ana:s3cret@hub.example.com:8811", "wss://tok-s3cret.example.com", "http://hub-s3cret.example.com", "s3cret nonsense"]) {
    try {
      checkHub(address);
      assert.fail(`${address} was accepted`);
    } catch (e) {
      assert.match(e.message, /TEAMS_DEV_HUB/);
      assert.ok(!e.message.includes("s3cret") && !e.message.includes("example.com"), e.message);
    }
  }
});

/** Loads hub-client.mjs in a child with TEAMS_DEV_HUB set (or unset), as seed.mjs and smoke.mjs do. */
function load(hub) {
  const env = { ...process.env };
  delete env.TEAMS_DEV_HUB;
  if (hub !== undefined) env.TEAMS_DEV_HUB = hub;
  return spawnSync(process.execPath, ["--input-type=module", "-e", `import { HUB } from ${JSON.stringify(CLIENT)}; console.log(HUB);`], { env, encoding: "utf8" });
}

test("loading the client with a loopback address, or none, works", () => {
  assert.equal(load("ws://127.0.0.1:9").stdout.trim(), "http://127.0.0.1:9");
  const unset = load(undefined);
  assert.equal(unset.status, 0);
  assert.equal(unset.stdout.trim(), "http://127.0.0.1:8811");
  assert.equal(load("").stdout.trim(), "http://127.0.0.1:8811");
});

test("loading the client with a refused address exits non-zero, names the variable and not the value", () => {
  for (const address of ["http://hub-s3cret.example.com", "http://ana:s3cret@127.0.0.1:8811", "wss://hub-s3cret.example.com"]) {
    const run = load(address);
    assert.notEqual(run.status, 0, address);
    assert.match(run.stderr, /TEAMS_DEV_HUB/);
    assert.ok(!(run.stderr + run.stdout).includes("s3cret") && !(run.stderr + run.stdout).includes("example.com"), run.stderr);
    assert.equal(run.stdout, "");
  }
});
