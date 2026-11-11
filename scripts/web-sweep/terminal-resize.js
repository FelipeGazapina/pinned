#!/usr/bin/env node
// Whether a tab terminal that keeps no history survives a resize. xterm.js moves the lines a
// narrower screen pushes off its top into the history, and with none it leaves the screen a line
// short of its rows, after which every write throws. The app resizes such a terminal through
// withHistory (ui/main.js). This writes screens of several fills into real xterm.js terminals of the
// sizes a tab starts at, resizes each to the sizes a phone, a tablet and a desktop fit, and checks
// that the screen is whole and still takes writes. It needs no stack.
//
//   PLAYWRIGHT=<dir>/node_modules/playwright node scripts/web-sweep/terminal-resize.js
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const UI = path.join(__dirname, "../../ui");
const main = fs.readFileSync(path.join(UI, "main.js"), "utf8");
const at = main.indexOf("function withHistory(");
const withHistory = main.slice(at, main.indexOf("\n}\n", at) + 2);
const SCROLLBACK = Number(main.match(/const SCROLLBACK = (\d+);/)[1]);

const check = (ok, what) => {
  if (!ok) throw new Error(what);
  console.log(`ok: ${what}`);
};

(async () => {
  const browser = await playwright.chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : { channel: process.env.SWEEP_CHANNEL || "chrome" }) });
  try {
    const page = await browser.newPage();
    await page.addScriptTag({ path: path.join(UI, "vendor/xterm.js") });
    await page.addScriptTag({ content: `const SCROLLBACK = ${SCROLLBACK};\n${withHistory}` });
    const result = await page.evaluate(async () => {
      const starts = [[120, 32], [80, 24], [200, 50]];
      const sizes = [[52, 59], [30, 40], [52, 20], [197, 63], [40, 80], [20, 100], [119, 33], [60, 32]];
      const fills = [0, 10, 31, 40, 200];
      const widths = [40, 100, 119, 300];
      const out = { bare: { broken: 0, total: 0 }, app: { broken: [], total: 0 } };
      for (const how of ["bare", "app"]) for (const start of starts) for (const size of sizes) for (const lines of fills) for (const width of widths) {
        const term = new Terminal({ cols: start[0], rows: start[1], scrollback: 0 });
        const write = (text) => new Promise((done) => term.write(text, done));
        let text = "";
        for (let i = 0; i < lines; i++) text += `${String(i).padEnd(4)}${"x".repeat(width)}\r\n`;
        await write(`${text}> prompt`);
        if (how === "app") withHistory(term, () => term.resize(size[0], size[1]));
        else term.resize(size[0], size[1]);
        const screen = term.buffer.active;
        let whole = screen.length >= screen.baseY + term.rows && !!screen.getLine(screen.baseY + screen.cursorY);
        if (whole) {
          await write("\r\nafter the resize");
          whole = false;
          for (let y = 0; y < screen.length; y++) whole ||= !!screen.getLine(y)?.translateToString(true).includes("after the resize");
        }
        out[how].total++;
        if (!whole && how === "bare") out.bare.broken++;
        if (!whole && how === "app") out.app.broken.push(`${start.join("x")} to ${size.join("x")}, ${lines} lines of ${width}`);
        if (how === "app" && term.options.scrollback !== 0) out.app.broken.push(`${start.join("x")} to ${size.join("x")} kept a history`);
        term.dispose();
      }
      return out;
    });
    console.log(`  xterm's own resize breaks ${result.bare.broken} of ${result.bare.total} screens with no history`);
    check(!result.app.broken.length, `withHistory resizes all ${result.app.total} screens whole, and each takes a write after${result.app.broken.length ? `: ${result.app.broken.slice(0, 5).join("; ")}` : ""}`);
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
