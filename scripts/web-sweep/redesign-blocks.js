#!/usr/bin/env node
// The redesign's shared view pieces in the running app at 390x844: the status dots glyphHtml draws
// and the bottom sheet openSheet opens. Checks each against the board's values (Sidebar,
// Phone-Workers, Phone-Approval, Phone-Actions), then works the sheet: Escape, a tap on the scrim
// and a swipe down close it, focus moves in and back, and a second sheet replaces the first.
// Writes blocks-glyphs.png and blocks-sheet.png into $SWEEP_DIR/redesign.
//
//   export SWEEP_DIR=... SWEEP_HUB_PORT=... SWEEP_PG_PORT=... SWEEP_GO_PORT=...
//   scripts/web-sweep/stack.sh start
//   PLAYWRIGHT=<dir>/node_modules/playwright node scripts/web-sweep/redesign-blocks.js
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const OUT = path.join(DIR, "redesign");
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";

const check = (ok, what) => {
  if (!ok) throw new Error(what);
  console.log(`ok: ${what}`);
};
const waitFor = async (what, fn, timeout = 30000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const found = await fn();
    if (found) return found;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timed out waiting for ${what}`);
};

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await playwright.chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : { channel: process.env.SWEEP_CHANNEL || "chrome" }) });
  try {
    const context = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true, colorScheme: "dark" });
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto(HUB);
    if (await page.isVisible("#login [name=email]")) {
      await page.fill("#login [name=email]", EMAIL);
      await page.fill("#login [name=password]", PASSWORD);
      await page.click("#login .primary");
    }
    await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
    await page.evaluate(() => setMode("gui"));

    const glyphs = await page.evaluate(() => {
      const box = document.createElement("div");
      box.id = "glyph-check";
      box.style.cssText = "position:fixed;inset:0;z-index:200;display:flex;flex-wrap:wrap;gap:16px;padding:24px;background:var(--g-canvas)";
      box.innerHTML = ["working", "needs", "unread", "done", "idle", "failed", "stopped"].flatMap((s) => [glyphHtml(s, 8), glyphHtml(s, 10), glyphHtml(s, 12)]).join("");
      document.body.append(box);
      const look = (el) => {
        const s = getComputedStyle(el);
        return { state: el.dataset.glyph, size: s.width, border: `${s.borderTopWidth} ${s.borderTopStyle} ${s.borderTopColor}`, bg: s.backgroundColor, anim: s.animationName === "none" ? "none" : `${s.animationName} ${s.animationDuration} ${s.animationTimingFunction}` };
      };
      return [...box.children].map(look);
    });
    await page.screenshot({ path: path.join(OUT, "blocks-glyphs.png"), clip: { x: 0, y: 0, width: 390, height: 140 } });
    const at = (state, size) => glyphs.find((g) => g.state === state && g.size === size);
    check(at("working", "10px").border === "2px dashed rgb(81, 162, 255)" && /glyph-spin 2.4s linear/.test(at("working", "10px").anim), "working is a dashed #51a2ff ring that spins in 2.4s, 2px wide on a phone");
    check(at("needs", "10px").bg === "rgb(254, 154, 0)", "needs is solid #fe9a00");
    check(at("unread", "10px").bg === "rgb(52, 107, 241)", "unread is solid #346bf1");
    check(at("done", "10px").bg === "rgb(0, 188, 125)", "done is solid #00bc7d");
    check(at("idle", "10px").border === "2px solid rgb(90, 90, 90)" && at("idle", "10px").bg === "rgba(0, 0, 0, 0)", "idle is a #5a5a5a ring");
    check(at("failed", "10px").bg === "rgb(251, 65, 74)", "failed takes the error color");
    check(glyphs.filter((g) => g.size === "8px").length === 7 && glyphs.filter((g) => g.size === "12px").length === 7, "each state draws at 8, 10 and 12px");
    await page.evaluate(() => document.getElementById("glyph-check").remove());

    await page.evaluate(() => {
      const opener = document.createElement("button");
      opener.id = "sheet-opener";
      opener.textContent = "Open";
      document.body.append(opener);
      opener.focus();
      window.__closed = [];
      window.__sheet = openSheet({
        label: "Workers",
        content: `<div style="display:flex;align-items:center;gap:10px;padding:10px 8px 6px 20px"><h2 style="margin:0;font-size:20px;font-weight:700">Workers</h2><span style="flex:1;font-size:14px;color:#a3a3a3">1 of 3 done</span><button type="button" id="sheet-x">Close</button></div><div style="height:240px"></div>`,
        onClose: () => window.__closed.push("workers"),
      });
    });
    await page.waitForTimeout(400);
    const look = await page.evaluate(() => {
      const panel = document.querySelector(".sheet-panel");
      const p = getComputedStyle(panel);
      const scrim = getComputedStyle(document.querySelector(".sheet-scrim"));
      const grab = getComputedStyle(document.querySelector(".sheet-grabber"));
      const r = panel.getBoundingClientRect();
      return { radius: p.borderRadius, bg: p.backgroundColor, pad: p.padding, bottom: Math.round(r.bottom), left: r.left, width: r.width, scrim: scrim.backgroundColor, grab: `${grab.width} ${grab.height} ${grab.borderRadius} ${grab.backgroundColor}`, role: panel.getAttribute("role"), label: panel.getAttribute("aria-label"), focus: document.activeElement.id };
    });
    await page.screenshot({ path: path.join(OUT, "blocks-sheet.png") });
    check(look.scrim === "rgba(0, 0, 0, 0.62)", "the scrim is rgba(0,0,0,0.62)");
    check(look.radius === "22px 22px 0px 0px" && look.bg === "rgb(20, 20, 20)", "the panel has the 22px top radius and #141414");
    check(look.pad === "8px 0px 34px" && look.bottom === 844 && look.left === 0 && look.width === 390, "the panel sits on the bottom edge, full width, padded 8px 0 34px");
    check(look.grab === "36px 5px 3px rgba(255, 255, 255, 0.22)", "the grabber is 36x5, radius 3, white at 0.22");
    check(look.role === "dialog" && look.label === "Workers", "the sheet is a dialog named Workers");
    check(look.focus === "sheet-x", "focus moved to the sheet's first control");

    await page.keyboard.press("Escape");
    check(await page.evaluate(() => !document.querySelector(".sheet-root") && document.activeElement.id === "sheet-opener" && window.__closed.join() === "workers"), "Escape closes it, onClose runs once and focus goes back to the opener");

    await page.evaluate(() => openSheet({ label: "Command approval", tone: "approval", content: "<p>git push origin readme-tidy</p>", onClose: () => window.__closed.push("approval") }));
    const border = await page.evaluate(() => getComputedStyle(document.querySelector(".sheet-panel")).borderTop);
    check(border === "1px solid rgba(254, 154, 0, 0.35)", "the approval tone has the amber top border");
    await page.mouse.click(195, 100);
    check(await page.evaluate(() => !document.querySelector(".sheet-root") && window.__closed.join() === "workers,approval"), "a tap on the scrim closes it");

    await page.evaluate(() => {
      openSheet({ label: "First", content: "<p>one</p>", onClose: () => window.__closed.push("first") });
      openSheet({ label: "Thread actions", content: "<p>two</p><div style='height:200px'></div>", onClose: () => window.__closed.push("second") });
    });
    check(await page.evaluate(() => document.querySelectorAll(".sheet-root").length === 1 && document.querySelector(".sheet-panel").getAttribute("aria-label") === "Thread actions" && window.__closed.at(-1) === "first"), "a second sheet replaces the first");
    await page.waitForTimeout(300);
    const top = await page.evaluate(() => document.querySelector(".sheet-panel").getBoundingClientRect().top);
    await page.mouse.move(195, top + 10);
    await page.mouse.down();
    await page.mouse.move(195, top + 40, { steps: 4 });
    await page.mouse.move(195, top + 60, { steps: 4 });
    await page.mouse.up();
    check(await page.evaluate(() => document.querySelectorAll(".sheet-root").length === 1), "a short slow drag springs back");
    await page.waitForTimeout(300);
    await page.mouse.move(195, top + 10);
    await page.mouse.down();
    await page.mouse.move(195, top + 150, { steps: 8 });
    await page.mouse.up();
    check(await page.evaluate(() => !document.querySelector(".sheet-root") && window.__closed.at(-1) === "second"), "a swipe down closes it");
    check(!errors.length, `no page errors${errors.length ? `: ${errors.join("; ")}` : ""}`);
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
