#!/usr/bin/env node
// Whether a tab terminal the page never showed can read real Claude Code's menu. Real claude runs
// in a PTY of the size a machine gives a session nobody watches (120x32), against the stack's
// OpenCode Go mock, until it asks the mock's AskUserQuestion; its bytes then go into xterm.js
// terminals of several sizes, as the hub replays them into a hidden tab, and the app's own
// terminalMenu reads each screen. The app's terminals start at newTerminal's size (ui/main.js); the old
// default was xterm's 80x24, and a terminal narrower than the PTY misreads the screen.
//
//   export SWEEP_DIR=... SWEEP_HUB_PORT=... SWEEP_PG_PORT=... SWEEP_GO_PORT=...
//   scripts/web-sweep/stack.sh start            # for the OpenCode Go mock
//   SWEEP_REAL_CLAUDE=~/.local/bin/claude PLAYWRIGHT=<dir>/node_modules/playwright node scripts/web-sweep/hidden-menu.js
//
// HIDDEN_MENU_PTY (default 120x32) is the PTY's size. Claude's home and project live in a fresh
// folder under SWEEP_DIR; nothing else is written.
const { spawn } = require("child_process");
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const MOCK = `http://127.0.0.1:${process.env.SWEEP_GO_PORT || 8798}`;
const CLAUDE = process.env.SWEEP_REAL_CLAUDE;
const UI = path.join(__dirname, "../../ui");
const [COLS, ROWS] = (process.env.HIDDEN_MENU_PTY || "120x32").split("x").map(Number);
const UNFITTED = fs.readFileSync(path.join(UI, "main.js"), "utf8").match(/function newTerminal\(\) \{\s+const term = new Terminal\(\{\s+cols: (\d+),\s+rows: (\d+),/).slice(1).join("x");

const check = (ok, what) => {
  if (!ok) throw new Error(what);
  console.log(`ok: ${what}`);
};

/** Claude's output from its start until its question menu has been up a moment, as one buffer. */
async function record() {
  const work = fs.mkdtempSync(path.join(DIR, "hidden-menu-"));
  const home = path.join(work, "home");
  const project = path.join(work, "project");
  fs.mkdirSync(home);
  fs.mkdirSync(project);
  fs.writeFileSync(path.join(home, ".claude.json"), JSON.stringify({ hasCompletedOnboarding: true, theme: "dark", projects: { [fs.realpathSync(project)]: { hasTrustDialogAccepted: true } } }));
  const out = path.join(work, "pty.bin");
  const env = {
    HOME: home, PATH: "/usr/bin:/bin", TERM: "xterm-256color", LANG: "en_US.UTF-8",
    ANTHROPIC_BASE_URL: `${MOCK}/inference/go/anthropic`, ANTHROPIC_AUTH_TOKEN: "oc_sk_sweep",
    ANTHROPIC_MODEL: "deepseek-v4.1-flash", ANTHROPIC_SMALL_FAST_MODEL: "deepseek-v4.1-flash",
    DISABLE_AUTOUPDATER: "1", CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
  };
  await fetch(`${MOCK}/mock/script`, { method: "POST", body: JSON.stringify({ first: [], gapMs: 50 }) });
  const run = spawn("script", ["-q", "-F", out, "sh", "-c", `stty cols ${COLS} rows ${ROWS}; exec "$0" "check the sweep"`, CLAUDE], { cwd: project, env, stdio: ["ignore", "ignore", "ignore"], detached: true });
  const end = Date.now() + 90000;
  let shown = 0;
  while (Date.now() < end) {
    await new Promise((r) => setTimeout(r, 250));
    const text = fs.existsSync(out) ? fs.readFileSync(out, "latin1") : "";
    if (/Enter to select/.test(text)) shown ||= Date.now();
    if (shown && Date.now() - shown > 2000) break;
  }
  process.kill(-run.pid, "SIGKILL");
  check(shown, `real claude asked its question in a ${COLS}x${ROWS} PTY`);
  return fs.readFileSync(out);
}

(async () => {
  if (!CLAUDE) throw new Error("set SWEEP_REAL_CLAUDE to a claude binary");
  const bytes = [...(await record())];
  const browser = await playwright.chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : { channel: process.env.SWEEP_CHANNEL || "chrome" }) });
  try {
    const page = await browser.newPage();
    await page.addScriptTag({ path: path.join(UI, "vendor/xterm.js") });
    await page.addScriptTag({ path: path.join(UI, "terminal-menu.js") });
    const read = (size) => page.evaluate(async ({ bytes, size }) => {
      const [cols, rows] = size.split("x").map(Number);
      const term = new Terminal({ cols, rows });
      await new Promise((resolve) => term.write(new Uint8Array(bytes), resolve));
      return terminalMenu({ term })?.options.map((o) => o.head) ?? null;
    }, { bytes, size });
    const wanted = ["North route", "South route", "Type something.", "Chat about this"];
    for (const size of [`${COLS}x${ROWS}`, UNFITTED, "200x50", "80x24"]) console.log(`  a ${size} terminal reads ${JSON.stringify(await read(size))}`);
    check(JSON.stringify(await read(`${COLS}x${ROWS}`)) === JSON.stringify(wanted), "a terminal of the PTY's size reads the whole menu");
    check(JSON.stringify(await read(UNFITTED)) === JSON.stringify(wanted), `the app's unfitted terminal (${UNFITTED}) reads the whole menu`);
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
