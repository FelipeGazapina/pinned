#!/usr/bin/env node
// Proves the Claude model list on the real page: the picker shows the current models with the
// legacy ones under "Legacy models", and alpha's Captain, through pinned's MCP server as Claude
// Code calls it, finds the newest model in orchestrator_capabilities and starts a delegate_task on
// it, then reads its task_status. With NEXT ("Fable 5.3"), the fake claude's /model menu lists that
// model as a newer Claude Code would: once the picker goes through that menu, the model is on top
// of the picker and in orchestrator_capabilities, and a delegate_task starts on it.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... MODEL=claude-fable-5-2 NAME="Claude Fable 5.2" LEGACY="Claude Fable 5.1" [NEXT="Fable 5.3"] node scripts/web-sweep/model-list.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes model-list.png, and model-list-next.png with NEXT, (1440x900) into scripts/web-sweep/screens/.
const { spawn } = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HOME = path.join(DIR, "home");
const SCREENS = path.join(__dirname, "screens");
const MODEL = process.env.MODEL || "claude-fable-5-2";
const NAME = process.env.NAME || "Claude Fable 5.2";
const LEGACY = process.env.LEGACY || "Claude Fable 5.1";
const SHOT = process.env.SHOT || "model-list.png";
const NEXT = process.env.NEXT;
const SHOT_NEXT = process.env.SHOT_NEXT || "model-list-next.png";

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

const chromium = process.env.CHROME_BIN || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

async function launch(errors) {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "model-list-"));
  const proc = spawn(chromium, ["--headless=new", "--remote-debugging-port=0", "--remote-allow-origins=*", "--no-first-run",
    "--no-default-browser-check", "--disable-gpu", "--hide-scrollbars", `--user-data-dir=${profile}`, "about:blank"], { stdio: ["ignore", "pipe", "pipe"] });
  const ws = await new Promise((resolve, reject) => {
    const fail = setTimeout(() => reject(new Error("Chromium never said its DevTools port")), 15000);
    let buf = "";
    const on = (chunk) => { buf += chunk; const m = buf.match(/DevTools listening on (ws:\/\/\S+)/); if (m) { clearTimeout(fail); resolve(m[1]); } };
    proc.stderr.on("data", on);
    proc.stdout.on("data", on);
  });
  const base = `http://${ws.match(/^ws:\/\/(\S+?)\//)[1]}`;
  const target = await (await fetch(`${base}/json/new?about:blank`, { method: "PUT" })).json();
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = () => reject(new Error("the DevTools socket did not open")); });
  let id = 0;
  const pending = new Map();
  const listeners = [];
  socket.addEventListener("message", (ev) => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) {
      const { resolve, reject } = pending.get(m.id);
      pending.delete(m.id);
      m.error ? reject(new Error(m.error.message)) : resolve(m.result);
    } else listeners.forEach((l) => l(m));
  });
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    pending.set(++id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async (fn, arg) => {
    const r = await send("Runtime.evaluate", { expression: `(${fn})(${JSON.stringify(arg ?? null)})`, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result.value;
  };
  await send("Runtime.enable");
  await send("Page.enable");
  listeners.push((m) => m.method === "Runtime.exceptionThrown" && errors.push(m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text));
  await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "dark" }] });
  await send("Emulation.setDeviceMetricsOverride", { width: 1440, height: 900, deviceScaleFactor: 1, mobile: false });
  const page = {
    evaluate,
    async goto(url) {
      const loaded = new Promise((resolve) => listeners.push((m) => m.method === "Page.loadEventFired" && resolve()));
      await send("Page.navigate", { url });
      await loaded;
    },
    async screenshot(file) {
      const shot = await send("Page.captureScreenshot", { format: "png", clip: { x: 0, y: 0, width: 1440, height: 900, scale: 1 } });
      fs.writeFileSync(file, Buffer.from(shot.data, "base64"));
    },
  };
  return { proc, profile, page };
}

// Every tool result the fake claude sessions wrote, newest last.
function toolResults(name) {
  const files = [];
  const walk = (dir) => fs.existsSync(dir) && fs.readdirSync(dir, { withFileTypes: true }).forEach((e) => {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p);
    else if (p.endsWith(".jsonl")) files.push(p);
  });
  walk(path.join(HOME, ".claude", "projects"));
  const uses = new Map();
  const results = [];
  for (const file of files) {
    for (const line of fs.readFileSync(file, "utf8").split("\n").filter(Boolean)) {
      const entry = JSON.parse(line);
      for (const part of Array.isArray(entry.message?.content) ? entry.message.content : []) {
        if (part.type === "tool_use") uses.set(`${file}:${part.id}`, part.name);
        if (part.type === "tool_result" && uses.get(`${file}:${part.tool_use_id}`) === `mcp__pinned__${name}`) results.push(part);
      }
    }
  }
  return results;
}

// One Captain turn that makes these pinned tool calls, as Claude Code makes them.
async function callThrough(page, calls) {
  fs.writeFileSync(path.join(HOME, "taste-calls.jsonl"), calls.map((c) => JSON.stringify(c)).join("\n"));
  const done = `Done: ${calls.map((c) => c.name).join(", ")}.`;
  const before = await page.evaluate((d) => document.getElementById("thread").innerText.split(d).length, done);
  await page.evaluate((t) => { prompt.value = t; send(); }, `call ${calls.map((c) => c.name).join(" ")}`);
  await waitFor(done, () => page.evaluate(({ d, n }) => document.getElementById("thread").innerText.split(d).length > n, { d: done, n: before }), 30000);
  return calls.map((c) => {
    const result = toolResults(c.name).at(-1);
    check(result && !result.is_error, `${c.name} failed: ${JSON.stringify(result)}`);
    return JSON.parse(result.content);
  });
}

(async () => {
  const errors = [];
  const { proc, profile, page } = await launch(errors);
  const rows = path.join(HOME, "model-rows.json");
  if (NEXT) fs.writeFileSync(rows, JSON.stringify([["Default (recommended) ✔", "Sonnet 5.5"], ["Sonnet", "Sonnet 5.5"], [NEXT.split(" ")[0], NEXT], ["Opus", "Opus 5.5"], ["Haiku", "Haiku 5.5"]]));
  try {
    await page.goto(HUB);
    await page.evaluate(() => {
      const form = document.querySelector("#login");
      if (!form) return;
      form.querySelector("[name=email]").value = "sweep@example.test";
      form.querySelector("[name=password]").value = "sweep password 1";
      form.querySelector(".primary").click();
    });
    await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
    await waitFor("the sweep node", () => page.evaluate(() => [...machines.values()].some((m) => m.view.online && m.view.name === "sweep-node")));
    await page.evaluate(() => setMode("gui"));
    const node = await page.evaluate(() => [...machines.keys()].find((k) => machines.get(k).view.name === "sweep-node"));
    const alpha = fs.realpathSync(path.join(HOME, "work", "alpha"));
    const captain = await page.evaluate(async ({ machine, alpha }) => {
      const tab = await revealTab(machine, await callOn(machine, { cmd: "open_orchestrator", path: alpha }));
      activate(tab);
      startTab(tab);
      return tab.key;
    }, { machine: node, alpha });
    await waitFor("the Captain's session", () => page.evaluate((k) => tabByKey(k)?.view.running === true, captain));

    // The Captain finds `model` in orchestrator_capabilities, starts a delegate_task on it and reads its task_status.
    const delegates = async (model, name) => {
      const [caps, started] = await callThrough(page, [
        { name: "orchestrator_capabilities", input: {} },
        { name: "delegate_task", input: { task: "Reply OK.", title: `${name} check`, target: { providerInstanceId: "claude", model } } },
      ]);
      const claude = caps.providers.find((p) => p.providerInstanceId === "claude");
      console.log("orchestrator_capabilities claude models:", claude.models.map((m) => `${m.id} (${m.label})`).join(", "));
      check(claude.models.some((m) => m.id === model), `${model} is not in orchestrator_capabilities`);
      console.log("delegate_task:", JSON.stringify(started));
      check(started.taskId && started.model === model, `delegate_task did not start on ${model}`);
      await new Promise((r) => setTimeout(r, 1500));
      const [status] = await callThrough(page, [{ name: "task_status", input: { taskId: started.taskId } }]);
      console.log("task_status:", JSON.stringify(status));
      check(status.model === model, `task_status names ${status.model}`);
    };
    // The picker with the legacy models open: `name` on top and `legacy` under Legacy models.
    const picker = async (name, legacy, shot) => {
      await page.evaluate((k) => activate(tabByKey(k)), captain);
      await page.evaluate(() => document.getElementById("model-button").click());
      await waitFor("the model picker", () => page.evaluate(() => !!document.querySelector('#composer-menu[data-kind="models"] .model')));
      if (await page.evaluate(() => document.querySelector('#composer-menu [data-row="legacy"]').getAttribute("aria-expanded") !== "true")) {
        await page.evaluate(() => document.querySelector('#composer-menu [data-row="legacy"]').click());
      }
      await waitFor("the legacy models", () => page.evaluate((n) => [...document.querySelectorAll("#composer-menu .model .name")].some((e) => e.textContent === n), legacy));
      const names = await page.evaluate(() => [...document.querySelectorAll("#composer-menu .model .name")].map((e) => e.textContent));
      console.log("picker:", names.join(" | "));
      check(names[0].startsWith(name), `the picker's first model is ${names[0]}`);
      check(names.indexOf(legacy) > names.indexOf("Legacy models"), `${legacy} is not under Legacy models`);
      await page.screenshot(path.join(SCREENS, shot));
    };

    await delegates(MODEL, NAME);
    await picker(NAME, LEGACY, SHOT);

    if (NEXT) {
      // Picking Opus goes through claude's /model menu, which now lists NEXT.
      await page.evaluate(() => document.querySelector('#composer-menu .model[data-row="claude-opus-5-5"]').click());
      await waitFor("Opus picked through claude's menu", () => page.evaluate(() => document.getElementById("model-button").textContent.trim() === "Opus 5.5"), 15000);
      const next = `claude-${NEXT.toLowerCase().replace(/[ .]/g, "-")}`;
      await waitFor(`${next} in the machine's list`, () => page.evaluate((slug) => claudeModels()[0]?.slug === slug, next), 10000);
      await picker(`Claude ${NEXT}`, NAME, SHOT_NEXT);
      await page.evaluate(() => document.getElementById("model-button").click());
      await delegates(next, `Claude ${NEXT}`);
    }
    check(!errors.length, `page errors:\n${errors.slice(0, 5).join("\n")}`);
    console.log("model list: every check passed");
  } finally {
    fs.rmSync(rows, { force: true });
    const exited = new Promise((resolve) => proc.once("exit", resolve));
    proc.kill("SIGTERM");
    await exited;
    fs.rmSync(profile, { recursive: true, force: true, maxRetries: 5 });
  }
})().catch((error) => {
  console.error(error.stack || String(error));
  process.exit(1);
});
