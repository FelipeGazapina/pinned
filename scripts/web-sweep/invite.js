#!/usr/bin/env node
// Two people on one invite hub. The stack's account (Ana) makes invite codes in Settings, Bruno
// creates his own account with one on the sign-in screen and pairs his own node, and each
// account's orchestrator, sessions, projects page and memory show nothing of the other.
//
//   SWEEP_SIGNUP=invite SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... CHROME_BIN=... node scripts/web-sweep/invite.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes invite-*.png and invite-*-sessions.json into scripts/web-sweep/screens/. Bruno's node runs
// from $SWEEP_DIR/home-b and its pid goes to $SWEEP_DIR/node-b.pid, which stack.sh stop kills.
const fs = require("fs");
const path = require("path");
const { execFileSync, spawn } = require("child_process");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const REPO = path.join(__dirname, "../..");
const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const SERVER = path.join(process.env.CARGO_TARGET_DIR || path.join(REPO, "target"), "debug/pinned-server");
const SCREENS = path.join(__dirname, "screens");
const INVITE_NEEDED = "this hub needs a valid invite code to create an account";
const DESKTOP = { width: 1440, height: 900 };
const PHONE = { width: 390, height: 844 };

const ANA = {
  who: "ana",
  email: "sweep@example.test",
  password: "sweep password 1",
  node: "sweep-node",
  home: path.join(DIR, "home"),
  folders: ["alpha", "beta", "gamma"],
  memory: ["Ana deploys in the morning", "Ana's deploys go out before noon, never at night."],
};
const BRUNO = {
  who: "bruno",
  email: "bruno@example.test",
  password: "bruno password 1",
  node: "bruno-node",
  home: path.join(DIR, "home-b"),
  folders: ["delta", "epsilon"],
  memory: ["Bruno bills on Fridays", "Bruno sends every invoice on Friday afternoon."],
};

const check = (ok, what) => {
  if (!ok) throw new Error(what);
};
const waitFor = async (what, fn, timeout = 20000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const found = await fn();
    if (found) return found;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timed out waiting for ${what}`);
};
const shot = (page, name) => page.screenshot({ path: path.join(SCREENS, name) });

async function signupStatus(body) {
  const res = await fetch(`${HUB}/api/signup`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  return [res.status, (await res.json()).error ?? "ok"];
}

async function openAccountPage(page) {
  await page.evaluate(() => {
    openSettings();
    showPage("account");
  });
  await page.waitForSelector(".card.invites", { timeout: 15000 });
}

function startBrunosNode() {
  for (const folder of BRUNO.folders) {
    const dir = path.join(BRUNO.home, "work", folder);
    fs.mkdirSync(dir, { recursive: true });
    execFileSync("git", ["-C", dir, "init", "-q"]);
    execFileSync("git", ["-C", dir, "remote", "add", "origin", `https://github.com/bruno/${folder}.git`]);
  }
  const config = path.join(DIR, "node-b");
  execFileSync(SERVER, ["login", HUB, BRUNO.email, "--name", BRUNO.node, "--config-dir", config], {
    env: { ...process.env, HOME: BRUNO.home, PINNED_PASSWORD: BRUNO.password },
    stdio: "ignore",
  });
  const log = fs.openSync(path.join(DIR, "node-b.log"), "a");
  const node = spawn(SERVER, ["node", "--config-dir", config], {
    env: { ...process.env, HOME: BRUNO.home, SHELL: path.join(DIR, "bin/shell"), PATH: `${path.join(DIR, "bin")}:${process.env.PATH}`, PINNED_PR_WATCH_SECS: "2" },
    stdio: ["ignore", log, log],
    detached: true,
  });
  node.unref();
  fs.writeFileSync(path.join(DIR, "node-b.pid"), `${node.pid}\n`);
}

// One account's view: its node's threads, its orchestrator on its own node, its global memory,
// its projects page and the hub's overview of its projects and sessions.
async function tour(page, person) {
  const node = await waitFor(`${person.node} online`, () =>
    page.evaluate((name) => [...machines.entries()].find(([, m]) => m.view.online && m.view.name === name)?.[0], person.node));
  await page.evaluate(() => setMode("gui"));
  const folders = person.folders.map((f) => fs.realpathSync(path.join(person.home, "work", f)));
  await page.evaluate(async ({ node, folders }) => {
    for (const dir of folders) if (!tabs.some((t) => t.machine === node && t.view.cwd === dir)) await openRemote(node, dir, false);
  }, { node, folders });

  await page.evaluate(() => {
    openSettings();
    showPage("general");
  });
  await page.waitForSelector(`#go-host .go-host[data-machine="${node}"]`, { timeout: 20000 });
  await page.click(`#go-host .go-host[data-machine="${node}"]`);
  await page.click("#go-save");
  await waitFor("the host saved", () => page.evaluate(() => document.getElementById("go-status").textContent === "Saved"));

  await page.click("#go-memory");
  await waitFor("the global memory panel", () => page.evaluate(() => document.querySelector('#panel [data-panel="memory"]') && document.querySelector("#panel .memory-add")));
  await page.fill("#panel .memory-add [name=title]", person.memory[0]);
  await page.fill("#panel .memory-add [name=text]", person.memory[1]);
  await page.click("#panel .memory-add button[type=submit]");
  const memories = await waitFor("the saved memory", () =>
    page.evaluate((title) => {
      const titles = [...document.querySelectorAll("#panel .memory-list .title")].map((t) => t.textContent);
      return titles.includes(title) && titles;
    }, person.memory[0]));
  // The memory panel opens beside the orchestrator's own chat, so one picture shows both.
  await page.evaluate(() => openGlobalChat());
  await waitFor("the orchestrator chat", () => page.evaluate(() => typeof active !== "undefined" && isGlobalTab(active)));
  await page.waitForTimeout(800);
  await shot(page, `invite-${person.who}-orchestrator.png`);

  await page.evaluate(() => openGlobalProjects());
  const projects = await waitFor("the projects page", () =>
    page.evaluate((count) => {
      renderGlobalProjectsPage();
      const names = [...document.querySelectorAll(".gp-row .gp-name")].map((n) => n.textContent);
      return names.length >= count && names;
    }, person.folders.length));
  await shot(page, `invite-${person.who}-projects.png`);

  const sessions = await page.evaluate(async () => ({
    machines: [...machines.values()].map((m) => m.view?.name ?? m.hubView?.name),
    tabs: tabs.map((t) => ({ machine: machines.get(t.machine)?.view.name, cwd: t.view.cwd, title: t.view.title })),
    overview: await hub.hub({ cmd: "overview" }),
  }));
  fs.writeFileSync(path.join(SCREENS, `invite-${person.who}-sessions.json`), `${JSON.stringify(sessions, null, 2).replaceAll(DIR, "$SWEEP_DIR")}\n`);
  const text = await page.evaluate(() => document.body.innerText);
  return { node, memories, projects, sessions, seen: `${text}\n${JSON.stringify(sessions)}\n${memories.join("\n")}` };
}

function sawNothingOf(view, other) {
  const marks = [other.node, other.email, other.memory[0], ...other.folders.map((f) => `/work/${f}`)];
  const leaked = marks.filter((mark) => view.seen.includes(mark));
  check(leaked.length === 0, `saw the other account's ${leaked.join(", ")}`);
  check(view.projects.every((name) => !other.folders.includes(name)), `projects page lists ${view.projects}`);
}

(async () => {
  fs.mkdirSync(SCREENS, { recursive: true });
  const browser = await playwright.chromium.launch({ headless: true, executablePath: process.env.CHROME_BIN || undefined });
  const errors = [];
  const newPage = async () => {
    const context = await browser.newContext({ viewport: DESKTOP, colorScheme: "dark" });
    await context.grantPermissions(["clipboard-read", "clipboard-write"], { origin: HUB });
    const page = await context.newPage();
    page.on("pageerror", (e) => errors.push(String(e)));
    return page;
  };

  const ana = await newPage();
  await ana.goto(HUB);
  await ana.fill("#login [name=email]", ANA.email);
  await ana.fill("#login [name=password]", ANA.password);
  await ana.click("#login .primary");
  await waitFor("Ana online", () => ana.evaluate(() => document.body.dataset.hub === "online"));
  await openAccountPage(ana);
  check(/Create an invite code/.test(await ana.textContent(".card.invites")), "no invite button");
  await ana.click('[data-action="create-invite"]');
  await ana.waitForSelector(".card.invites .row.command code");
  const revokedCode = await ana.textContent(".card.invites .row.command code");
  await ana.click('[data-action="create-invite"]');
  await waitFor("a second code", async () => (await ana.textContent(".card.invites .row.command code")) !== revokedCode);
  const code = await ana.textContent(".card.invites .row.command code");
  check(/^[A-Z2-9]{4}(-[A-Z2-9]{4}){3}$/.test(code), `code reads ${code}`);
  await ana.click('[data-action="copy-invite"]');
  check((await ana.evaluate(() => navigator.clipboard.readText())) === code, "Copy put something else on the clipboard");
  const pending = await ana.evaluate(() => invitesUi.pending.map((i) => ({ id: i.id, hint: i.hint })));
  check(pending.length === 2 && pending.some((p) => p.hint === code.slice(0, 4)) && pending.some((p) => p.hint === revokedCode.slice(0, 4)), `pending ${JSON.stringify(pending)}`);
  await ana.evaluate(() => (document.querySelector('[data-action="copy-invite"]').textContent = "Copy"));
  await shot(ana, "invite-settings.png");
  await ana.setViewportSize(PHONE);
  await openAccountPage(ana);
  await ana.evaluate(() => document.querySelector(".card.invites").scrollIntoView({ block: "end" }));
  await shot(ana, "invite-settings-phone.png");
  await ana.setViewportSize(DESKTOP);
  const revoked = pending.find((p) => p.hint === revokedCode.slice(0, 4)).id;
  await ana.click(`[data-invite="${revoked}"] [data-action="revoke-invite"]`);
  await waitFor("the revoked row gone", () => ana.evaluate((id) => !document.querySelector(`[data-invite="${id}"]`), revoked));
  console.log("ana made", code, "and revoked", revokedCode);

  const bruno = await newPage();
  await bruno.goto(HUB);
  await bruno.waitForSelector("#login [name=invite]", { state: "visible" });
  await shot(bruno, "invite-signup.png");
  await bruno.fill("#login [name=email]", BRUNO.email);
  await bruno.fill("#login [name=password]", BRUNO.password);
  await bruno.click("#login [name=signup]");
  await waitFor("the refusal", async () => (await bruno.textContent("#login .error")) === INVITE_NEEDED);
  await shot(bruno, "invite-signup-refused.png");
  await bruno.fill("#login [name=invite]", revokedCode);
  await bruno.evaluate(() => (document.querySelector("#login .error").textContent = ""));
  await bruno.click("#login [name=signup]");
  await waitFor("the revoked code refused", async () => (await bruno.textContent("#login .error")) === INVITE_NEEDED);
  await bruno.evaluate(() => (document.querySelector("#login .error").textContent = ""));
  await bruno.fill("#login [name=invite]", code.toLowerCase());
  await bruno.setViewportSize(PHONE);
  await shot(bruno, "invite-signup-phone.png");
  await bruno.setViewportSize(DESKTOP);
  await bruno.click("#login [name=signup]");
  await waitFor("Bruno online", () => bruno.evaluate(() => document.body.dataset.hub === "online"));
  const accounts = { ana: await ana.evaluate(() => hub.hub({ cmd: "invites" })), bruno: await bruno.evaluate(() => hub.hub({ cmd: "invites" })) };
  check(accounts.ana.pending.length === 0 && accounts.bruno.pending.length === 0, `pending after sign up ${JSON.stringify(accounts)}`);
  check((await signupStatus({ email: "carla@example.test", password: "carla password 1", invite: code })).join(" ") === `403 ${INVITE_NEEDED}`, "the used code worked twice");
  console.log("bruno signed up with the code; it fails a second time");

  startBrunosNode();
  const anaView = await tour(ana, ANA);
  const brunoView = await tour(bruno, BRUNO);
  check(anaView.node !== brunoView.node, "both accounts toured one node");
  sawNothingOf(anaView, BRUNO);
  sawNothingOf(brunoView, ANA);
  check(JSON.stringify(anaView.memories) === JSON.stringify([ANA.memory[0]]), `Ana's memory lists ${anaView.memories}`);
  check(JSON.stringify(brunoView.memories) === JSON.stringify([BRUNO.memory[0]]), `Bruno's memory lists ${brunoView.memories}`);
  check(await bruno.evaluate((id) => !machines.has(id), anaView.node), "Bruno's page knows Ana's node");
  const refused = await bruno.evaluate((id) => hub.call(id, { cmd: "tabs" }).then(() => "answered", (e) => String(e)), anaView.node);
  check(refused === `this account has no machine ${anaView.node}`, `Bruno reached Ana's node: ${refused}`);
  for (const [who, view] of [["ana", anaView], ["bruno", brunoView]]) {
    console.log(`${who}: machines ${JSON.stringify(view.sessions.machines)} projects ${JSON.stringify(view.projects)} memory ${JSON.stringify(view.memories)}`);
    console.log(`${who}: sessions ${JSON.stringify(view.sessions.tabs.map((t) => `${t.machine}:${path.basename(t.cwd ?? "")}`))}`);
  }
  console.log("bruno calling ana's node:", refused);
  check(errors.length === 0, `page errors: ${errors.join("\n")}`);
  console.log("ok");
  await browser.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
