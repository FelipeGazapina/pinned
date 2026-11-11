#!/usr/bin/env node
// Drives #project / @machine in the global orchestrator's composer on the real page, by keyboard.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/composer-tags.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes composer-tags-{popover,machines,route,plain}.png into SWEEP_DIR. The sweep stack has one
// node, so the page is handed two more machines and a project list, and one tab's view is marked
// orchestrator_global (the hub does not mark any yet). Each step re-applies that, since a tabs
// event from the node replaces the view.
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const WORK = path.join(DIR, "home", "work");

const check = (ok, what) => {
  if (!ok) throw new Error(what);
};
const waitFor = async (what, fn, timeout = 20000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    if (await fn()) return;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timed out waiting for ${what}`);
};

// Runs in the page: the account's other machines and every machine's projects, the global
// orchestrator tab, and a working Captain on site (a tab opened in site's folder).
function seed({ node, keys }) {
  const work = tabs.find((t) => t.key === keys.orch).view.cwd.replace(/\/[^/]+$/, "");
  const fake = (id, name, online, projects) => {
    if (!machines.has(id)) machines.set(id, { view: { id, name, online }, tabs: [], transport: { call: () => Promise.resolve([]) } });
    noteProjects(id, projects);
  };
  fake("macbook", "MacBook", true, [
    { path: "/Users/felipe/site", name: "site", repo: "github.com/felipe/site" },
    { path: "/Users/felipe/api", name: "api", repo: "github.com/felipe/api" },
  ]);
  fake("vps", "Linux VPS", false, [{ path: "/srv/infra", name: "infra", repo: "github.com/felipe/infra" }]);
  noteProjects(node, [
    { path: `${work}/pinned`, name: "pinned", repo: "github.com/felipe/pinned" },
    { path: `${work}/site`, name: "site", repo: "github.com/felipe/site" },
    { path: `${work}/sistema-notas`, name: "sistema-notas", repo: null },
  ]);
  tabs.find((t) => t.key === keys.orch).view.orchestrator_global = true;
  Object.assign(tabs.find((t) => t.key === keys.captain).view, { role: "orchestrator", status: "working" });
}

const state = () => ({
  text: prompt.value,
  popover: document.getElementById("tag-popover").hidden ? null : document.getElementById("tag-popover").innerText,
  highlight: document.querySelector("#tag-popover .highlight .name")?.textContent ?? null,
  route: document.getElementById("tag-route").hidden ? null : [...document.querySelectorAll("#tag-route .route")].map((row) => `${row.className.replace("route ", "")}: ${row.querySelector(".where").textContent}${[...row.querySelectorAll("[data-machine]")].map((b) => ` [${b.textContent}]`).join("")}`),
  mirror: [...document.querySelectorAll("#prompt-mirror .tag")].map((el) => `${el.className}:${el.textContent}`),
  fileOrPrMenu: !document.getElementById("composer-menu").hidden && ["mention", "pr"].includes(document.getElementById("composer-menu").dataset.kind),
});

(async () => {
  const browser = await playwright.chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : { channel: process.env.SWEEP_CHANNEL || "chrome" }) });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, colorScheme: "dark" });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(HUB);
  await page.fill("#login [name=email]", EMAIL);
  await page.fill("#login [name=password]", PASSWORD);
  await page.click("#login .primary");
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  await waitFor("the sweep node", () => page.evaluate(() => [...machines.values()].some((m) => m.view.online && m.view.name === "sweep-node")));
  const node = await page.evaluate(() => [...machines.keys()].find((k) => machines.get(k).view.name === "sweep-node"));
  await page.evaluate(() => setMode("gui"));
  const keys = {};
  for (const name of ["orch", "captain", "plain"]) {
    const folder = { orch: "alpha", captain: "site", plain: "plain" }[name];
    fs.mkdirSync(path.join(WORK, folder), { recursive: true });
    keys[name] = await page.evaluate(async ({ node, dir }) => {
      const open = tabs.find((t) => t.machine === node && t.view.cwd === dir);
      return (open ?? (await openRemote(node, dir, false))).key;
    }, { node, dir: fs.realpathSync(path.join(WORK, folder)) });
  }
  // The page asks the node for its projects and each folder's repo on its own; seeding before
  // those answers land would be overwritten by them.
  await waitFor("the node's own project and repo answers", () =>
    page.evaluate(({ node, keys }) => {
      const captain = tabs.find((t) => t.key === keys.captain);
      return machines.get(node).projects !== undefined && projectRepos.has(`${node}:${captain.view.cwd}`);
    }, { node, keys }));
  const open = async (key) => {
    await page.evaluate((args) => {
      seed(args);
      activate(tabs.find((t) => t.key === args.key));
    }, { node, keys, key });
    await page.click("#prompt");
    await page.keyboard.press("ControlOrMeta+A");
    await page.keyboard.press("Backspace");
  };
  await page.addScriptTag({ content: `${seed};` });
  const read = () => page.evaluate(state);

  await open(keys.orch);
  await page.keyboard.type("no #si");
  let s = await read();
  console.log("typed 'no #si':", JSON.stringify(s));
  check(s.popover?.startsWith("Projects"), "# did not open the project list");
  check(/site\ngithub.com\/felipe\/site · sweep-node · MacBook\nworking/.test(s.popover), `site row: ${s.popover}`);
  check(/sistema-notas\nno remote · sweep-node\nnone/.test(s.popover), `sistema-notas row: ${s.popover}`);
  check(!s.fileOrPrMenu, "the pull request menu opened too");
  await page.screenshot({ path: path.join(DIR, "composer-tags-popover.png") });
  const first = s.highlight;
  await page.keyboard.press("ArrowDown");
  s = await read();
  check(s.highlight !== first, "ArrowDown did not move the highlight");
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Enter");
  s = await read();
  check(s.text === `no #${first} ` && s.popover === null, `Enter inserted ${JSON.stringify(s.text)}`);

  await page.keyboard.type("@");
  s = await read();
  check(s.popover?.startsWith("Machines") && /Linux VPS\noffline\n@linux-vps/.test(s.popover), `@ list: ${s.popover}`);
  check(!s.fileOrPrMenu, "the file menu opened too");
  await page.screenshot({ path: path.join(DIR, "composer-tags-machines.png") });
  await page.keyboard.press("Escape");
  await page.keyboard.press("ArrowLeft");
  s = await read();
  check(s.popover === null, "Esc did not keep the popover closed");
  await page.keyboard.press("End");
  await page.keyboard.type("mac");
  await page.keyboard.press("Tab");
  s = await read();
  check(/^no #site @mac-?\w+ $/.test(s.text), `Tab inserted ${JSON.stringify(s.text)}`);

  await open(keys.orch);
  // Enter sends, so a blank line is two Shift+Enters, as a person types it.
  const paragraphs = ["no #site, corrige o login.", "no #pinned, sobe a versão.", "olha os logs de ontem.", "no #fantasma, nada."];
  for (const [i, text] of paragraphs.entries()) {
    if (i) for (const _ of [0, 1]) await page.keyboard.press("Shift+Enter");
    await page.keyboard.type(text);
  }
  await page.keyboard.press("Escape");
  s = await read();
  console.log("four paragraphs:", JSON.stringify(s));
  check(
    JSON.stringify(s.route.map((r) => r.replace(/ It has no Captain yet, so one starts\.$/, ""))) === JSON.stringify([
      "ambiguous: site is on 2 machines [@sweep-node] [@macbook]",
      "routed: Goes to Captain pinned on sweep-node.",
      "untagged: no project: Orchestrator decides",
      "unknown: unknown project #fantasma",
    ]),
    `route: ${JSON.stringify(s.route)}`,
  );
  check(JSON.stringify(s.mirror) === JSON.stringify(["tag project:#site", "tag project:#pinned", "tag project unknown:#fantasma"]), `mirror: ${s.mirror}`);
  await page.screenshot({ path: path.join(DIR, "composer-tags-route.png") });
  await page.click('#tag-route [data-machine="macbook"]');
  s = await read();
  check(s.text.startsWith("no #site @macbook, corrige o login.\n\nno #pinned, sobe a versão."), `button wrote ${JSON.stringify(s.text)}`);
  check(s.route[0].startsWith("routed: Goes to Captain site on MacBook.") && s.route[1].startsWith("routed: Goes to Captain pinned on sweep-node."), `route after the button: ${s.route}`);
  check(s.mirror.includes("tag machine:@macbook"), `mirror after the button: ${s.mirror}`);
  await page.screenshot({ path: path.join(DIR, "composer-tags-route-picked.png") });

  await open(keys.plain);
  await page.keyboard.type("no #si");
  s = await read();
  console.log("ordinary tab:", JSON.stringify(s));
  check(s.popover === null && s.route === null && s.mirror.length === 0, "the ordinary tab changed");
  check(await page.evaluate(() => document.getElementById("composer").dataset.chips === "false"), "the ordinary tab painted a mirror");
  await page.keyboard.type(" @");
  await waitFor("the file menu on the ordinary tab", () => page.evaluate(() => !document.getElementById("composer-menu").hidden && document.getElementById("composer-menu").dataset.kind === "mention"));
  check((await read()).popover === null, "the ordinary tab opened the machine list");
  await page.screenshot({ path: path.join(DIR, "composer-tags-plain.png") });

  // Last, since the turn it starts makes the node resend the tab's view without the flag.
  await open(keys.orch);
  await page.keyboard.type("olha o #pinned");
  check((await read()).highlight === "pinned", "the typed tag is not highlighted");
  await page.keyboard.press("Enter");
  await waitFor("Enter to send a message that ends in a typed tag", () => page.evaluate(() => prompt.value === ""));

  await browser.close();
  check(!errors.length, `page errors:\n${errors.slice(0, 5).join("\n")}`);
  console.log("composer tags: every check passed");
})().catch((error) => {
  console.error(error.stack || String(error));
  process.exit(1);
});
