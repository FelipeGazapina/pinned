#!/usr/bin/env node
// Drives the Placement block of Settings › Way of working on the real page, with two machines
// (sweep-node, and sweep-mini that this script pairs): the empty policy, a policy built through the
// pickers (Prefer sweep-mini, a retired machine struck out, Avoid sweep-node), the refusal to put a
// machine in both lists, a save that reaches the hub, and a reload that shows the saved policy.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... PLAYWRIGHT=... SWEEP_CHANNEL=chrome node scripts/web-sweep/placement.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop   # stops sweep-mini too: placement.js writes node-b.pid
//
// Writes placement-{empty,filled}-{desktop,phone}.png into scripts/web-sweep/screens/placement/.
const fs = require("fs");
const path = require("path");
const { execFileSync, spawn } = require("child_process");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const SERVER = path.join(process.env.CARGO_TARGET_DIR || path.join(__dirname, "../../target"), "debug/pinned-server");
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const SCREENS = path.join(__dirname, "screens", "placement");
const SIZES = [["desktop", { width: 1440, height: 900 }], ["phone", { width: 390, height: 844 }]];
const RETIRED = "m-retired-runner";

const check = (ok, what) => {
  if (!ok) throw new Error(what);
  console.log(`ok: ${what}`);
};
const waitFor = async (what, fn, timeout = 30000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const found = await fn();
    if (found) return found;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`timed out waiting for ${what}`);
};

/** A second node on the same hub, in its own HOME and config. Its pid goes to node-b.pid. */
function pairMini() {
  const home = path.join(DIR, "home2");
  fs.mkdirSync(path.join(home, "work"), { recursive: true });
  const env = { ...process.env, HOME: home, PINNED_PASSWORD: PASSWORD };
  for (const key of ["PINNED_SOCK", "PINNED_TASK", "PINNED_ACCOUNT", "CLAUDE_CONFIG_DIR"]) delete env[key];
  execFileSync(SERVER, ["login", HUB, EMAIL, "--name", "sweep-mini", "--config-dir", path.join(DIR, "node2")], { env, stdio: "ignore" });
  const node = spawn(SERVER, ["node", "--config-dir", path.join(DIR, "node2")], {
    env: { ...env, SHELL: path.join(DIR, "bin", "shell"), PATH: `${path.join(DIR, "bin")}:${process.env.PATH}` },
    stdio: ["ignore", fs.openSync(path.join(DIR, "node2.log"), "w"), fs.openSync(path.join(DIR, "node2.log"), "a")],
    detached: true,
  });
  node.unref();
  fs.writeFileSync(path.join(DIR, "node-b.pid"), String(node.pid));
}

async function signIn(page) {
  await page.goto(HUB);
  if (await page.isVisible("#login [name=email]")) {
    await page.fill("#login [name=email]", EMAIL);
    await page.fill("#login [name=password]", PASSWORD);
    await page.click("#login .primary");
  }
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  await waitFor("both nodes online", () => page.evaluate(() => ["sweep-node", "sweep-mini"].every((n) => [...machines.values()].some((m) => m.view.online && m.view.name === n))));
}

const placementOf = (page) => page.evaluate(async () => (await connectorCall("get_tastes")).placement);
const lists = (page) =>
  page.evaluate(() => Object.fromEntries(["Prefer", "Avoid"].map((l) => [l, [...document.querySelectorAll(`.taste-placement [aria-label="${l}"] li`)].map((li) => li.querySelector(".name").textContent.trim() + (li.classList.contains("gone") ? " (removed)" : ""))])));
const idOf = (page, name) => page.evaluate((n) => [...machines.entries()].find(([, m]) => m.view.name === n)[0], name);
const toPlacement = async (page) => {
  await page.click('.taste-scopes button:text-is("Captains (all)")');
  await page.locator(".taste-placement").scrollIntoViewIfNeeded();
};

(async () => {
  fs.mkdirSync(SCREENS, { recursive: true });
  if (!fs.existsSync(path.join(DIR, "node-b.pid"))) pairMini();
  const browser = await playwright.chromium.launch({ headless: true, channel: process.env.SWEEP_CHANNEL || undefined });
  const shoot = async (state, open) => {
    for (const [size, viewport] of SIZES) {
      const ctx = await browser.newContext({ viewport, colorScheme: "dark", deviceScaleFactor: size === "phone" ? 2 : 1 });
      const page = await ctx.newPage();
      await signIn(page);
      await page.evaluate(() => openPage("taste"));
      await page.waitForSelector(".taste-placement .taste-ladder, .taste-placement select");
      await open(page);
      await page.waitForTimeout(400);
      await page.screenshot({ path: path.join(SCREENS, `placement-${state}-${size}.png`) });
      await ctx.close();
    }
  };

  const ctx = await browser.newContext({ viewport: SIZES[0][1], colorScheme: "dark" });
  const page = await ctx.newPage();
  await signIn(page);
  const [node, mini] = [await idOf(page, "sweep-node"), await idOf(page, "sweep-mini")];
  await page.evaluate(() => openPage("taste"));
  await page.waitForSelector(".taste-placement select[data-place-add]");
  await toPlacement(page);

  // The empty policy.
  check(JSON.stringify(await placementOf(page)) === JSON.stringify({ prefer: [], avoid: [], heavy: "worktree" }), "a fresh account has the empty policy");
  check(/Nothing is preferred or avoided, so every worker runs on the machine it was asked on\./.test(await page.locator(".taste-policy").innerText()), "the empty policy says every worker runs where it was asked");
  check(/If no machine fits, the worker runs on the machine that asked, and the reason is shown\./.test(await page.locator(".taste-policy").innerText()), "the fallback line shows, with no switch for it");
  await shoot("empty", toPlacement);

  // Build a policy through the pickers: Prefer sweep-mini, then Avoid sweep-node.
  await page.selectOption('select[data-place-add="prefer"]', mini);
  await page.selectOption('select[data-place-add="avoid"]', node);
  check(JSON.stringify(await lists(page)) === JSON.stringify({ Prefer: ["sweep-mini"], Avoid: ["sweep-node"] }), "the pickers fill Prefer and Avoid by machine name");
  const locked = await page.$$eval('select[data-place-add="avoid"] option[disabled]:not([value=""])', (o) => o.map((x) => x.textContent));
  check(locked.join() === "sweep-mini (in Prefer)", `Avoid's picker locks the preferred machine (${locked.join()})`);
  check(!(await page.$$eval('select[data-place-add="prefer"] option', (o) => o.map((x) => x.value))).includes(node), "a machine already in Avoid is not offered for Prefer");
  check(/Unsaved changes/.test(await page.locator(".taste-foot").innerText()), "an edit shows the unsaved footer");
  await page.click('.taste-foot button:text-is("Save")');
  await waitFor("the save", async () => (await placementOf(page)).prefer.length === 1);
  const saved = await placementOf(page);
  check(saved.prefer[0] === mini && saved.avoid[0] === node && saved.heavy === "worktree", "the hub stores the saved policy by machine id");

  // A machine the account lost: it reaches the policy through the hub, as one removed later would.
  await page.evaluate(async (id) => {
    const got = await connectorCall("get_tastes");
    await connectorCall("set_taste", { scope: { kind: "captains" }, patch: { placement: { ...got.placement, prefer: [...got.placement.prefer, id] } } });
  }, RETIRED);
  await page.reload();
  await waitFor("the page", () => page.evaluate(() => document.body.dataset.hub === "online"));
  await page.evaluate(() => openPage("taste"));
  await page.waitForSelector(".taste-placement li");
  const reloaded = await lists(page);
  check(JSON.stringify(reloaded) === JSON.stringify({ Prefer: ["sweep-mini", `${RETIRED} (removed)`], Avoid: ["sweep-node"] }), `a reload shows the saved policy, the lost machine struck out (${JSON.stringify(reloaded)})`);
  check(await page.$eval(".taste-placement li.gone .name s", (s) => getComputedStyle(s).textDecorationLine === "line-through"), "the lost machine's name is struck through");
  check(await page.locator('.taste-placement li.gone [data-act="pdrop"]').count() === 1, "the lost machine has a remove button");
  await shoot("filled", toPlacement);

  // Removing the lost machine saves a policy without it.
  await toPlacement(page);
  await page.click('.taste-placement li.gone [data-act="pdrop"]');
  await page.click('.taste-foot button:text-is("Save")');
  await waitFor("the removal", async () => (await placementOf(page)).prefer.length === 1);
  check(JSON.stringify((await placementOf(page)).prefer) === JSON.stringify([mini]), "removing the lost machine saves a policy without it");
  await ctx.close();
  await browser.close();
  console.log("placement sweep done");
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
