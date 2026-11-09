#!/usr/bin/env node
// Drives the global orchestrator's settings, projects page and global memory on the real page.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/global-orchestrator.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes go-{settings,settings-phone,projects,memory}.png into scripts/web-sweep/screens/. It needs
// two nodes: stack.sh pairs one, and a second one must be paired to the same hub (see the PR). Both
// open the same three work folders. The page is also handed an online machine running an older
// pinned and an offline one, so the section's notes and the skipped writes are exercised too.
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const WORK = path.join(DIR, "home", "work");
const SCREENS = path.join(__dirname, "screens");

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

// Runs in the page: a machine too old for the global calls (online, refusing), an offline one,
// and a working Captain on alpha with two running worker tabs, which the projects page counts. A
// tabs event from the node replaces a tab's view, so every step re-applies them.
function seed() {
  if (!machines.has("old-mac")) {
    machines.set("old-mac", {
      view: { id: "old-mac", name: "Old Mac", online: true },
      tabs: [],
      transport: { call: () => Promise.reject(new Error("this machine runs pinned 0.8.0, which can't answer that: unknown variant GlobalOrchestratorSettings")) },
    });
    noteProjects("old-mac", []);
  }
  if (!machines.has("vps")) {
    machines.set("vps", { view: { id: "vps", name: "Linux VPS", online: false }, tabs: [], transport: { call: () => Promise.resolve([]) } });
    noteProjects("vps", [{ path: "/srv/infra", name: "infra", repo: "github.com/example/infra.git" }]);
  }
  const captain = tabs.find((t) => t.machine === window.__captainMachine && t.view.cwd === window.__captainDir);
  if (captain) {
    Object.assign(captain.view, { role: "orchestrator", status: "working" });
    for (const key of window.__workers ?? []) {
      const worker = tabByKey(key);
      if (worker) Object.assign(worker.view, { role: "worker", running: true, spawned_by: captain.view.id });
    }
  }
}

(async () => {
  const browser = await playwright.chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 860 }, colorScheme: "dark" });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(HUB);
  await page.fill("#login [name=email]", EMAIL);
  await page.fill("#login [name=password]", PASSWORD);
  await page.click("#login .primary");
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  // The two nodes in name order, so "first" and "second" below mean sweep-node and sweep-node-2.
  const nodes = await waitFor("the sweep nodes", () =>
    page.evaluate(() => {
      const found = [...machines.entries()].filter(([, m]) => m.view.online && /^sweep-node/.test(m.view.name)).map(([id, m]) => ({ id, name: m.view.name }));
      return found.length >= 2 ? found.sort((a, b) => a.name.localeCompare(b.name)) : null;
    }));
  console.log("nodes:", JSON.stringify(nodes));
  const nameOf = (id) => nodes.find((n) => n.id === id)?.name ?? id;
  await page.evaluate(() => setMode("gui"));
  const folders = ["alpha", "beta", "gamma"].map((n) => fs.realpathSync(path.join(WORK, n)));
  const alpha = folders[0];

  // Both nodes open the three work folders, because a node lists only the folders its own app has
  // opened. The Captain on alpha is on the first node, with two workers running there.
  await page.evaluate(async ({ ids, dirs, captain, dir }) => {
    window.__captainMachine = captain;
    window.__captainDir = dir;
    for (const node of ids) {
      for (const d of dirs) if (!tabs.some((t) => t.machine === node && t.view.cwd === d)) await openRemote(node, d, false);
    }
    window.__workers = [];
    for (let i = 0; i < 2; i++) window.__workers.push((await revealTab(captain, await callOn(captain, { cmd: "open_tab", path: dir, bypass: false }))).key);
  }, { ids: nodes.map((n) => n.id), dirs: folders, captain: nodes[0].id, dir: alpha });
  await page.addScriptTag({ content: `${seed};` });
  await waitFor("both nodes' project answers", () =>
    page.evaluate(({ ids, dir }) => ids.every((node) => {
      const names = (machines.get(node).projects ?? []).map((p) => p.name);
      return ["alpha", "beta", "gamma"].every((n) => names.includes(n)) && projectRepos.has(`${node}:${dir}`);
    }), { ids: nodes.map((n) => n.id), dir: alpha }));

  // Settings: one account-wide section, saved to every machine at once. The older and the offline
  // machine are seeded first, so the section's read already includes them.
  await page.evaluate(seed);
  await page.evaluate(() => openSettings());
  await waitFor("the Global orchestrator section", () => page.evaluate(() => !!document.getElementById("global-orchestrator-settings")));
  await waitFor("the section's notes about the machines that cannot take it", () =>
    page.evaluate(() => /Old Mac needs to update pinned/.test(document.getElementById("go-notes").textContent) && /Linux VPS is offline/.test(document.getElementById("go-notes").textContent)));
  const hosts = await page.evaluate(() => [...document.querySelectorAll("#go-host .go-host")].map((b) => `${b.textContent}:${b.dataset.machine}:${b.disabled ? "off" : "on"}`));
  console.log("host chips:", JSON.stringify(hosts));
  for (const n of nodes) check(hosts.includes(`${n.name}:${n.id}:on`), `no online ${n.name} chip: ${hosts}`);
  check(hosts.some((h) => h.startsWith("Old Mac:") && h.endsWith(":off")), `Old Mac chip not disabled: ${hosts}`);

  await page.evaluate(() => {
    window.__goEvent = null;
    document.addEventListener("global-orchestrator-settings", (e) => (window.__goEvent = e.detail));
  });
  await page.click('#global-orchestrator-settings .go-preset[data-name="Admiral"]');
  // Host the orchestrator on whichever node does not host it now, so the save moves it.
  const before = await page.evaluate(() => document.querySelector("#go-host .go-host.active")?.dataset.machine ?? null);
  const chosen = nodes.find((n) => n.id !== before) ?? nodes[1];
  console.log("host chip before:", nameOf(before), "choosing:", chosen.name);
  await page.click(`#go-host .go-host[data-machine="${chosen.id}"]`);
  await page.click("#go-save");
  await waitFor("the save to land", () => page.evaluate(() => document.getElementById("go-status").textContent === "Saved"));
  const perNode = await page.evaluate((ids) => Promise.all(ids.map((id) => callOn(id, { cmd: "global_orchestrator_settings" }))), nodes.map((n) => n.id));
  console.log("saved settings, read back per node:", JSON.stringify(Object.fromEntries(nodes.map((n, i) => [n.name, perNode[i]]))));
  const hosted = nodes.filter((n, i) => perNode[i].host === true).map((n) => n.name);
  const highlighted = await page.evaluate(() => [...document.querySelectorAll("#go-host .go-host.active")].map((b) => b.textContent));
  console.log("highlighted host chip:", JSON.stringify(highlighted), "read back host:true on:", JSON.stringify(hosted));
  check(JSON.stringify(hosted) === JSON.stringify([chosen.name]), `host:true on ${JSON.stringify(hosted)}, chose ${chosen.name}`);
  check(JSON.stringify(highlighted) === JSON.stringify(hosted), `highlighted chip ${JSON.stringify(highlighted)} is not the host ${JSON.stringify(hosted)}`);
  check(perNode.every((s) => s.name === "Admiral"), `names: ${JSON.stringify(perNode)}`);
  const hostSettings = perNode[nodes.indexOf(chosen)];
  check(JSON.stringify(await page.evaluate(() => window.__goEvent)) === JSON.stringify(hostSettings), `event detail: ${JSON.stringify(await page.evaluate(() => window.__goEvent))}`);

  // Every option reads whole in its select, every button's label inside the button, and the save
  // status inside the section, at the desktop width and at a phone's.
  const fits = () =>
    page.evaluate(() => {
      const section = document.getElementById("global-orchestrator-settings");
      const ctx = document.createElement("canvas").getContext("2d");
      const cut = [];
      for (const select of section.querySelectorAll("select")) {
        const css = getComputedStyle(select);
        ctx.font = css.font;
        const room = select.clientWidth - parseFloat(css.paddingLeft) - parseFloat(css.paddingRight);
        for (const o of select.options) if (ctx.measureText(o.text).width > room) cut.push(`${select.id}: "${o.text}" needs ${Math.ceil(ctx.measureText(o.text).width)}px, has ${room}px`);
      }
      const box = section.getBoundingClientRect();
      const status = document.getElementById("go-status").getBoundingClientRect();
      if (status.right > box.right || status.left < box.left || status.right > document.documentElement.clientWidth) cut.push(`Saved at ${status.left}-${status.right}, section ${box.left}-${box.right}`);
      for (const button of section.querySelectorAll("button")) {
        const edge = button.getBoundingClientRect().right;
        if (button.scrollWidth > button.clientWidth + 1 || edge > box.right + 1) cut.push(`"${button.textContent}" needs ${button.scrollWidth}px, has ${button.clientWidth}px, ends at ${edge} of ${box.right}`);
      }
      return cut;
    });
  const desktopCut = await fits();
  check(!desktopCut.length, `desktop: ${desktopCut.join("; ")}`);
  await page.locator("#global-orchestrator-settings").screenshot({ path: path.join(SCREENS, "go-settings.png") });
  await page.setViewportSize({ width: 390, height: 1400 });
  await page.evaluate(() => (document.getElementById("go-status").textContent = "Saved"));
  const phoneCut = await fits();
  check(!phoneCut.length, `phone: ${phoneCut.join("; ")}`);
  await page.locator("#global-orchestrator-settings").screenshot({ path: path.join(SCREENS, "go-settings-phone.png") });
  await page.setViewportSize({ width: 1280, height: 860 });
  console.log("settings layout: every option and the save status fit at 1280 and 390 px");

  // The projects page: every project of the account, its Captain and its workers. It merges a repo
  // across machines whatever the sidebar's Project grouping says, so grouping is off here. A tabs event can
  // rebuild the tab list while the Captain's thread is briefly keyless, and the page's thread GC
  // drops the workers with it, so a row that still looks right is re-rendered before it is read.
  await page.evaluate(() => {
    sideSettings.grouping = false;
    openGlobalProjects();
  });
  await waitFor("alpha working with its two workers", () =>
    page.evaluate(() => {
      seed();
      const alphaRow = () => [...document.querySelectorAll(".gp-row")].find((r) => r.querySelector(".gp-name")?.textContent === "alpha");
      const right = (row) => !!row && row.querySelector(".gp-state").textContent === "Working" && row.querySelector(".gp-workers").textContent === "2 workers";
      if (!right(alphaRow())) renderGlobalProjectsPage();
      return right(alphaRow());
    }));
  const rows = await page.evaluate(() => {
    seed();
    renderGlobalProjectsPage();
    return [...document.querySelectorAll(".gp-row")].map((row) => ({
    name: row.querySelector(".gp-name").textContent,
    state: row.querySelector(".gp-state").textContent,
    repo: row.querySelector(".gp-repo").textContent,
    machines: [...row.querySelectorAll(".gp-machine")].map((b) => `${b.textContent}:${b.disabled ? "off" : "on"}`),
    workers: row.querySelector(".gp-workers").textContent,
  }));
  });
  console.log("project rows:", JSON.stringify(rows));
  const alphaRow = rows.find((r) => r.name === "alpha");
  check(alphaRow && alphaRow.state === "Working", `alpha row: ${JSON.stringify(alphaRow)}`);
  check(alphaRow.workers === "2 workers", `alpha workers: ${JSON.stringify(alphaRow)}`);
  // Each machine chip reads "<machine name><captain state><running workers>"; alpha lists both nodes.
  const alphaHosts = await page.evaluate(() => {
    const row = [...document.querySelectorAll(".gp-row")].find((r) => r.querySelector(".gp-name")?.textContent === "alpha");
    return [...row.querySelectorAll(".gp-machine")].map((b) => b.firstChild.textContent).sort();
  });
  console.log("alpha machines:", JSON.stringify(alphaHosts));
  check(JSON.stringify(alphaHosts) === JSON.stringify(nodes.map((n) => n.name).sort()), `alpha should list ${nodes.map((n) => n.name)}: ${JSON.stringify(alphaHosts)}`);
  check(alphaRow.machines.includes(`${nodes[0].name}Working2 workers:on`), `alpha's Captain and workers on ${nodes[0].name}: ${JSON.stringify(alphaRow)}`);
  // The rows are the account's projects: the three work folders and the offline machine's infra.
  check(JSON.stringify(rows.map((r) => r.name).sort()) === JSON.stringify(["alpha", "beta", "gamma", "infra"]), `unexpected rows: ${JSON.stringify(rows.map((r) => r.name))}`);
  const infraRow = rows.find((r) => r.name === "infra");
  check(infraRow && infraRow.machines.every((m) => m.endsWith(":off")), `infra row offline: ${JSON.stringify(infraRow)}`);
  check(rows.every((r) => !/orquestrador/i.test(r.name)), "the orchestrator's own folder is a project");
  await page.screenshot({ path: path.join(SCREENS, "go-projects.png") });
  await page.evaluate(() => (sideSettings.grouping = true));

  // The global memory: the host's orchestrator folder, as the memory panel of its global tab.
  await page.evaluate(() => openSettings());
  await waitFor("the section again", () => page.evaluate(() => !!document.getElementById("go-memory")));
  await page.click("#go-memory");
  await waitFor("the global memory panel", () => page.evaluate(() => document.querySelector('#panel [data-panel="memory"]') && document.querySelector("#panel .memory-add")));
  const memoryTitle = await page.evaluate(() => document.querySelector('#panel [data-panel="memory"]').getAttribute("title"));
  check(/Admiral's memory/.test(memoryTitle), `panel title: ${memoryTitle}`);
  await page.fill("#panel .memory-add [name=title]", "Deploy pela manhã");
  await page.fill("#panel .memory-add [name=text]", "O orquestrador manda deploys sempre pela manhã, nunca à noite.");
  await page.click("#panel .memory-add button[type=submit]");
  await waitFor("the saved memory in the list", () =>
    page.evaluate(() => [...document.querySelectorAll("#panel .memory-list .title")].some((t) => /Deploy pela manhã/.test(t.textContent))));
  await page.fill("#panel .memory-find", "deploy");
  await waitFor("the search to find it", () =>
    page.evaluate(() => [...document.querySelectorAll("#panel .memory-list .title")].some((t) => /Deploy pela manhã/.test(t.textContent))));
  await page.locator("#panel").screenshot({ path: path.join(SCREENS, "go-memory.png") });

  await browser.close();
  check(!errors.length, `page errors:\n${errors.slice(0, 5).join("\n")}`);
  console.log("global orchestrator: every check passed");
})().catch((error) => {
  console.error(error.stack || String(error));
  process.exit(1);
});
