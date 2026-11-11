#!/usr/bin/env node
// Puts each proposed redesign board beside the live app in the same state and measures how far
// apart they are. For every selected screen it renders the board, seeds and opens the app, and
// writes board-<screen>.png, live-<screen>.png, pair-<screen>.png (board left, app right) and
// diff-<screen>.png (differing pixels in red), plus report.json with the share of differing pixels
// overall and in each of the screen's named regions.
//
//   export SWEEP_DIR=... SWEEP_HUB_PORT=... SWEEP_PG_PORT=... SWEEP_GO_PORT=...
//   export CARGO=true CARGO_TARGET_DIR=<a checkout's target>   # reuse a built pinned-server
//   scripts/web-sweep/stack.sh start
//   PLAYWRIGHT=<dir>/node_modules/playwright node scripts/web-sweep/redesign.js [screen ...]
//   scripts/web-sweep/stack.sh stop    # also ends the extra nodes the run started
//
// A screen is a module in redesign/ (main, phone-home, ...) or its board's name; none runs them
// all. A module exports board, viewport, phone, regions ([{name, x, y, width, height}] in CSS
// pixels of the board), open(page, ctx) and optionally seed(page, ctx), scale, and liveViewport
// with clip for a board that is part of a screen. Before seed, redesign/world.js has seeded the
// shared state and set ctx.machines and ctx.threads (Sidebar board row id to tab key).
//
// REDESIGN_OUT (default $SWEEP_DIR/redesign) gets the files, and report.json there keeps the
// screens a run did not select. BOARD_DIR (default the main checkout's
// .claude/research/redesign/boards) holds the boards. REDESIGN_LIVE=0 renders and checks the
// boards alone, with no stack. The browser is Google Chrome (SWEEP_CHANNEL, default "chrome", or
// CHROME_BIN for a binary).
//
// A board counts as complete when board-support.js reports no failed hole or import, no {{ or
// sc-/dc- element is left in the page, and its root has the size its $preview names. A pixel
// differs when any channel is more than DIFF_TOLERANCE (default 32) apart. Each ratio comes as
// all/content: the share of all pixels that differ, then the share among content pixels, those
// with a channel above DIFF_INK (default 40) on either side. Both screens are mostly near-black,
// which keeps the first number small however different the layouts are.
const fs = require("fs");
const http = require("http");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");
const world = require("./redesign/world");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const OUT = process.env.REDESIGN_OUT || path.join(DIR, "redesign");
const BOARD_DIR = world.BOARD_DIR;
const LIVE = process.env.REDESIGN_LIVE !== "0";
const TOLERANCE = Number(process.env.DIFF_TOLERANCE || 32);
const INK = Number(process.env.DIFF_INK || 40);
const MODULES = path.join(__dirname, "redesign");

const screens = fs.readdirSync(MODULES).filter((f) => f.endsWith(".js") && f !== "world.js").map((f) => ({ id: f.slice(0, -3), ...require(path.join(MODULES, f)) }));

function select(names) {
  if (!names.length) return screens;
  return names.map((n) => {
    const found = screens.find((s) => s.id === n.toLowerCase() || s.board.toLowerCase() === n.toLowerCase());
    if (!found) throw new Error(`no screen ${n}; screens: ${screens.map((s) => s.id).join(", ")}`);
    return found;
  });
}

const waitFor = async (what, fn, timeout = 30000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const found = await fn();
    if (found) return found;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`timed out waiting for ${what}`);
};

/** Serves BOARD_DIR with board-support.js as the support.js every board loads. */
function serveBoards() {
  const server = http.createServer((req, res) => {
    const name = decodeURIComponent(new URL(req.url, "http://x").pathname).slice(1);
    const file = name === "support.js" ? path.join(__dirname, "board-support.js") : path.join(BOARD_DIR, path.basename(name));
    fs.readFile(file, (err, body) => {
      if (err) return res.writeHead(404).end();
      res.writeHead(200, { "content-type": file.endsWith(".js") ? "text/javascript" : "text/html; charset=utf-8" }).end(body);
    });
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(server)));
}

const scaleOf = (screen) => screen.scale ?? (screen.phone ? 2 : 1);

// Runs in the board's page: what keeps the board from being drawn whole.
const boardCheck = () => {
  const root = document.querySelector("x-dc")?.firstElementChild;
  const r = root?.getBoundingClientRect();
  const preview = window.__dcPreview;
  return {
    state: document.documentElement.dataset.dc,
    errors: window.__dcErrors,
    holes: (document.body.innerHTML.match(/\{\{/g) || []).length,
    leftovers: document.querySelectorAll("x-dc sc-if, x-dc sc-for, x-dc dc-import").length,
    size: r ? [Math.round(r.width), Math.round(r.height)] : null,
    preview: preview ? [preview.width, preview.height] : null,
  };
};

async function shootBoard(browser, base, screen, file) {
  const context = await browser.newContext({ viewport: screen.viewport, deviceScaleFactor: scaleOf(screen), colorScheme: "dark" });
  const page = await context.newPage();
  try {
    await page.goto(`${base}/${screen.board}.dc.html`);
    await page.waitForSelector("html[data-dc]", { timeout: 15000 });
    await page.evaluate(() => document.fonts.ready);
    const check = await page.evaluate(boardCheck);
    check.complete = check.state === "ready" && !check.holes && !check.leftovers && JSON.stringify(check.size) === JSON.stringify(check.preview);
    await page.screenshot({ path: file, animations: "disabled" });
    return check;
  } finally {
    await context.close();
  }
}

async function login(page) {
  await page.goto(HUB);
  if (await page.isVisible("#login [name=email]")) {
    await page.fill("#login [name=email]", world.EMAIL);
    await page.fill("#login [name=password]", world.PASSWORD);
    await page.click("#login .primary");
  }
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  await page.evaluate(() => setMode("gui"));
}

async function shootLive(browser, screen, ctx, file) {
  const phone = !!screen.phone;
  const viewport = screen.liveViewport ?? screen.viewport;
  const context = await browser.newContext({ viewport, deviceScaleFactor: scaleOf(screen), colorScheme: "dark", isMobile: phone, hasTouch: phone });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  try {
    await login(page);
    await world.ensure(page, ctx);
    await screen.seed?.(page, ctx);
    await screen.open(page, ctx);
    await page.waitForTimeout(600);
    await page.screenshot({ path: file, animations: "disabled", clip: screen.clip });
    return { errors, warnings: [...ctx.warnings] };
  } finally {
    await context.close();
  }
}

// Runs in a blank page: compares two screenshots of one size and draws the pair and the diff.
async function comparePngs({ board, live, regions, scale, tolerance, ink, labels }) {
  const load = (src) => new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("could not load a screenshot"));
    img.src = src;
  });
  const [a, b] = await Promise.all([load(board), load(live)]);
  const w = a.width;
  const h = a.height;
  const pixels = (img) => {
    const c = new OffscreenCanvas(w, h);
    const g = c.getContext("2d");
    g.fillStyle = "#000";
    g.fillRect(0, 0, w, h);
    g.drawImage(img, 0, 0);
    return g.getImageData(0, 0, w, h).data;
  };
  const pa = pixels(a);
  const pb = pixels(b);
  const differs = new Uint8Array(w * h);
  const content = new Uint8Array(w * h);
  const diff = new ImageData(w, h);
  for (let i = 0; i < w * h; i++) {
    const o = i * 4;
    const d = Math.max(Math.abs(pa[o] - pb[o]), Math.abs(pa[o + 1] - pb[o + 1]), Math.abs(pa[o + 2] - pb[o + 2]));
    differs[i] = d > tolerance ? 1 : 0;
    content[i] = Math.max(pa[o], pa[o + 1], pa[o + 2], pb[o], pb[o + 1], pb[o + 2]) > ink ? 1 : 0;
    const grey = (pa[o] + pa[o + 1] + pa[o + 2]) / 12;
    diff.data.set(differs[i] ? [255, 40, 40, 255] : [grey, grey, grey, 255], o);
  }
  const ratio = (x0, y0, x1, y1) => {
    let n = 0;
    let total = 0;
    let inked = 0;
    let inkedDiffers = 0;
    for (let y = Math.max(0, y0); y < Math.min(h, y1); y++) for (let x = Math.max(0, x0); x < Math.min(w, x1); x++) {
      const i = y * w + x;
      n += differs[i];
      total++;
      inked += content[i];
      inkedDiffers += differs[i] & content[i];
    }
    return { all: total ? n / total : 0, content: inked ? inkedDiffers / inked : 0 };
  };
  const byRegion = {};
  for (const r of regions) byRegion[r.name] = ratio(Math.round(r.x * scale), Math.round(r.y * scale), Math.round((r.x + r.width) * scale), Math.round((r.y + r.height) * scale));
  const whole = ratio(0, 0, w, h);

  const url = async (canvas) => {
    const blob = await canvas.convertToBlob({ type: "image/png" });
    return new Promise((resolve) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result);
      reader.readAsDataURL(blob);
    });
  };
  const gap = 16 * scale;
  const head = 28 * scale;
  const pair = new OffscreenCanvas(w * 2 + gap * 3, h + head + gap);
  const pg = pair.getContext("2d");
  pg.fillStyle = "#222";
  pg.fillRect(0, 0, pair.width, pair.height);
  pg.fillStyle = "#eee";
  pg.font = `600 ${14 * scale}px system-ui`;
  pg.fillText(labels[0], gap, head - 8 * scale);
  pg.fillText(labels[1], w + gap * 2, head - 8 * scale);
  pg.drawImage(a, gap, head);
  pg.drawImage(b, w + gap * 2, head);
  const out = new OffscreenCanvas(w, h);
  const og = out.getContext("2d");
  og.putImageData(diff, 0, 0);
  og.strokeStyle = "rgba(90,170,255,0.9)";
  og.fillStyle = "rgba(90,170,255,0.9)";
  og.lineWidth = scale;
  og.font = `600 ${11 * scale}px system-ui`;
  for (const r of regions) {
    og.strokeRect(r.x * scale + 0.5, r.y * scale + 0.5, r.width * scale - 1, r.height * scale - 1);
    og.fillText(r.name, r.x * scale + 4 * scale, r.y * scale + 13 * scale);
  }
  return { overall: whole.all, content: whole.content, regions: Object.fromEntries(Object.entries(byRegion).map(([k, v]) => [k, v.all])), contentRegions: Object.fromEntries(Object.entries(byRegion).map(([k, v]) => [k, v.content])), size: [w, h], liveSize: [b.width, b.height], pair: await url(pair), diff: await url(out) };
}

async function compare(browser, screen, files) {
  const page = await browser.newPage();
  try {
    const png = (f) => `data:image/png;base64,${fs.readFileSync(f).toString("base64")}`;
    const r = await page.evaluate(comparePngs, {
      board: png(files.board), live: png(files.live), regions: screen.regions, scale: scaleOf(screen), tolerance: TOLERANCE, ink: INK,
      labels: [`Board ${screen.board}.dc.html`, `App ${screen.id}`],
    });
    fs.writeFileSync(files.pair, Buffer.from(r.pair.split(",")[1], "base64"));
    fs.writeFileSync(files.diff, Buffer.from(r.diff.split(",")[1], "base64"));
    return { overall: r.overall, content: r.content, regions: r.regions, contentRegions: r.contentRegions, size: r.size, liveSize: r.liveSize };
  } finally {
    await page.close();
  }
}

const pct = (x) => `${(x * 100).toFixed(1)}%`;

(async () => {
  const selected = select(process.argv.slice(2));
  fs.mkdirSync(OUT, { recursive: true });
  const reportFile = path.join(OUT, "report.json");
  const report = fs.existsSync(reportFile) ? JSON.parse(fs.readFileSync(reportFile, "utf8")) : { screens: {} };
  const server = await serveBoards();
  const base = `http://127.0.0.1:${server.address().port}`;
  const browser = await playwright.chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : { channel: process.env.SWEEP_CHANNEL || "chrome" }) });
  const ctx = { hub: HUB, dir: DIR, out: OUT, waitFor, world: world.WORLD, warnings: [] };
  ctx.warn = (m) => ctx.warnings.push(m);
  let failed = 0;
  try {
    for (const screen of selected) {
      const files = Object.fromEntries(["board", "live", "pair", "diff"].map((k) => [k, path.join(OUT, `${k}-${screen.id}.png`)]));
      const entry = { board: screen.board, viewport: screen.viewport, scale: scaleOf(screen), at: new Date().toISOString() };
      try {
        entry.boardCheck = await shootBoard(browser, base, screen, files.board);
        if (!entry.boardCheck.complete) failed++;
        if (LIVE) {
          ctx.phone = !!screen.phone;
          entry.live = await shootLive(browser, screen, ctx, files.live);
          Object.assign(entry, await compare(browser, screen, files));
        }
      } catch (e) {
        entry.error = e.stack || String(e);
        failed++;
      }
      report.screens[screen.id] = entry;
      const regions = Object.entries(entry.regions ?? {}).map(([k, v]) => `${k} ${pct(v)}/${pct(entry.contentRegions[k])}`).join("  ");
      const board = entry.boardCheck?.complete ? "board ok" : `board INCOMPLETE ${JSON.stringify(entry.boardCheck ?? null)}`;
      console.log(`${screen.id.padEnd(16)} ${board}  ${entry.error ? `ERROR ${entry.error.split("\n")[0]}` : entry.overall != null ? `overall ${pct(entry.overall)}/${pct(entry.content)}  ${regions}` : ""}`);
    }
  } finally {
    await browser.close();
    server.close();
    report.boardDir = BOARD_DIR;
    report.tolerance = TOLERANCE;
    report.ink = INK;
    fs.writeFileSync(reportFile, `${JSON.stringify(report, null, 2)}\n`);
  }
  process.exit(failed ? 1 : 0);
})().catch((e) => {
  console.error(e.stack || String(e));
  process.exit(1);
});
