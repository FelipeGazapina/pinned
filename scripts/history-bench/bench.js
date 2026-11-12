#!/usr/bin/env node
// Times how a long conversation opens, against the web sweep's local hub and node.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/history-bench/bench.js [turns=2000] [runs=3]
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Opens a thread on alpha, swaps its transcript for seed.js's invented one, then reports:
// - the node's reads as the app makes them (`thread` from 0, page after page, until the end), with
//   the bytes each answer carries through the hub;
// - a newest-first read (`thread` with `limit`) when the node knows it;
// - the page's open time: from signing in to the last turn on screen with the composer usable.
// PLAYWRIGHT names the playwright module; CHROME_BIN a browser when playwright's own is missing.
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const [TURNS = "2000", RUNS = "3"] = process.argv.slice(2);
const LAST = `Done with turn ${TURNS}.`;

const waitFor = async (what, fn, timeout = 60000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const found = await fn();
    if (found) return found;
    await new Promise((r) => setTimeout(r, 50));
  }
  throw new Error(`timed out waiting for ${what}`);
};
const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];

/** A fresh page signed in; `t0` is when the sign-in was sent, where the app starts loading the account. */
async function signIn(browser) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });
  const page = await context.newPage();
  page.on("pageerror", (e) => console.error("pageerror", String(e)));
  await page.goto(HUB);
  await page.fill("#login [name=email]", "sweep@example.test");
  await page.fill("#login [name=password]", "sweep password 1");
  const t0 = Date.now();
  await page.click("#login .primary");
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  return { context, page, t0 };
}

(async () => {
  const browser = await playwright.chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {}) });
  const { context, page } = await signIn(browser);
  const machine = await waitFor("sweep-node", () => page.evaluate(() => [...machines.entries()].find(([, m]) => m.view.online && m.view.name === "sweep-node")?.[0]));
  const call = (c) => page.evaluate(([m, c]) => callOn(m, c), [machine, c]);
  const jsonl = () => execFileSync("find", [DIR, "-name", "*.jsonl", "-path", "*projects*"]).toString().split("\n").filter(Boolean);
  const before = new Set(jsonl());
  const tab = await call({ cmd: "open_tab", path: path.join(DIR, "home/work/alpha"), bypass: false });
  await call({ cmd: "start_claude", tab, cols: 100, rows: 30 });
  await new Promise((r) => setTimeout(r, 1500));
  await call({ cmd: "prompt", tab, text: "hello" });
  const file = await waitFor("the session's transcript", () => jsonl().find((f) => !before.has(f)));
  const session = path.basename(file, ".jsonl");
  await new Promise((r) => setTimeout(r, 1500));
  execFileSync("node", [path.join(__dirname, "seed.js"), file, session, TURNS], { stdio: "inherit" });
  const size = fs.statSync(file).size;
  await waitFor("the session", () => call({ cmd: "thread", tab, from: 0 }).then((p) => p.session === session));
  await context.close();

  // As the app reads a thread: from 0, then on from each page's next.
  const reads = [];
  for (let run = 0; run < Number(RUNS); run++) {
    const { context, page } = await signIn(browser);
    const timed = (c) => page.evaluate(async ([m, c]) => {
      const t = performance.now();
      const answer = await callOn(m, c);
      return { ms: performance.now() - t, bytes: JSON.stringify(answer).length, items: answer.items.length, next: answer.next, start: answer.start };
    }, [machine, c]);
    await waitFor("sweep-node", () => page.evaluate((m) => machines.get(m)?.view.online, machine));
    let from = 0;
    const pages = [];
    do {
      const p = await timed({ cmd: "thread", tab, from });
      pages.push(p);
      if (p.next === from) break;
      from = p.next;
    } while (from < size);
    const paged = await timed({ cmd: "thread", tab, from: 0, limit: 150 });
    reads.push({ whole: pages, paged });
    await context.close();
  }
  const sum = (ps, k) => ps.reduce((a, p) => a + p[k], 0);
  console.log(`transcript ${size} bytes, ${TURNS} turns`);
  console.log(`read from 0, as the app does: ${reads[0].whole.length} calls, ${sum(reads[0].whole, "items")} items, ${sum(reads[0].whole, "bytes")} bytes on the wire, median ${median(reads.map((r) => sum(r.whole, "ms"))).toFixed(0)} ms`);
  const p = reads[0].paged;
  console.log(p.start === undefined ? "read with limit: this node ignores limit" : `read with limit 150: ${p.items} items, ${p.bytes} bytes, start ${p.start}, median ${median(reads.map((r) => r.paged.ms)).toFixed(0)} ms`);

  // The page: from signing in to the last turn on screen and the composer usable.
  const opens = [];
  for (let run = 0; run < Number(RUNS); run++) {
    const { context, page, t0 } = await signIn(browser);
    const key = await waitFor("the tab", () => page.evaluate(([m, id]) => tabs.find((t) => t.machine === m && t.view.id === id)?.key, [machine, tab]));
    const tTab = Date.now();
    await page.evaluate((key) => activate(tabByKey(key)), key);
    await waitFor("the last turn", () => page.evaluate((last) => document.getElementById("thread").textContent.includes(last) && !document.getElementById("prompt").disabled, LAST), 120000);
    const t1 = Date.now();
    opens.push({ total: t1 - t0, fromTab: t1 - tTab, items: await page.evaluate((key) => threadOf(tabByKey(key)).items.length, key) });
    if (process.env.SHOTS) await page.screenshot({ path: path.join(process.env.SHOTS, `open-${run}.png`) });
    await context.close();
  }
  console.log(`open, load to last turn shown: median ${median(opens.map((o) => o.total))} ms (${opens.map((o) => o.total).join(", ")}); from the tab listed: median ${median(opens.map((o) => o.fromTab))} ms; items held ${opens[0].items}`);
  await call({ cmd: "close_tab", tab }).catch(() => {});
  await browser.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
