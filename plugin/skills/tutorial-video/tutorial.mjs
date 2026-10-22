import fs from "node:fs";
import path from "node:path";

const NAV = "nav, aside, header, [role=navigation], [role=tablist], [role=menu], [role=menubar]";

const OVERLAY = `(() => {
  if (window.__tutorial) return;
  window.__tutorial = true;
  const add = () => {
    if (document.getElementById("__tut-cursor")) return;
    const s = document.createElement("style");
    s.textContent = \`
      #__tut-cursor{position:fixed;z-index:2147483647;left:0;top:0;width:22px;height:22px;margin:-4px 0 0 -4px;pointer-events:none;
        background:url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='22' height='22' viewBox='0 0 22 22'><path d='M3 2l14 8-6 1.5L8.5 18z' fill='black' stroke='white' stroke-width='1.5' stroke-linejoin='round'/></svg>") no-repeat}
      .__tut-ring{position:fixed;z-index:2147483646;pointer-events:none;border:3px solid #ff3b30;border-radius:8px;box-shadow:0 0 0 4px rgba(255,59,48,.25);transition:all .15s}
      .__tut-ripple{position:fixed;z-index:2147483646;pointer-events:none;width:12px;height:12px;margin:-6px 0 0 -6px;border-radius:50%;background:rgba(255,59,48,.5);animation:__tut-r .5s ease-out forwards}
      @keyframes __tut-r{to{transform:scale(4);opacity:0}}
      #__tut-caption{position:fixed;z-index:2147483647;left:50%;bottom:24px;transform:translateX(-50%);max-width:80vw;padding:10px 18px;border-radius:10px;
        background:rgba(20,20,20,.88);color:#fff;font:600 17px/1.35 -apple-system,system-ui,sans-serif;pointer-events:none;text-align:center}\`;
    document.documentElement.append(s);
    const c = document.createElement("div");
    c.id = "__tut-cursor";
    const p = JSON.parse(sessionStorage.getItem("__tut-pos") || "[40,40]");
    c.style.transform = "translate(" + p[0] + "px," + p[1] + "px)";
    document.documentElement.append(c);
    window.addEventListener("mousemove", (e) => {
      c.style.transform = "translate(" + e.clientX + "px," + e.clientY + "px)";
      sessionStorage.setItem("__tut-pos", JSON.stringify([e.clientX, e.clientY]));
    }, true);
    window.addEventListener("mousedown", (e) => {
      const r = document.createElement("div");
      r.className = "__tut-ripple";
      r.style.left = e.clientX + "px";
      r.style.top = e.clientY + "px";
      document.documentElement.append(r);
      setTimeout(() => r.remove(), 600);
    }, true);
  };
  if (document.documentElement) add();
  document.addEventListener("DOMContentLoaded", add);
  window.__tutCaption = (text) => {
    add();
    let el = document.getElementById("__tut-caption");
    if (!text) return el?.remove();
    if (!el) {
      el = document.createElement("div");
      el.id = "__tut-caption";
      document.documentElement.append(el);
    }
    el.textContent = text;
  };
  window.__tutRing = (box) => {
    document.querySelectorAll(".__tut-ring").forEach((r) => r.remove());
    if (!box) return;
    const r = document.createElement("div");
    r.className = "__tut-ring";
    Object.assign(r.style, { left: box.x - 6 + "px", top: box.y - 6 + "px", width: box.width + 12 + "px", height: box.height + 12 + "px" });
    document.documentElement.append(r);
  };
})();`;

/**
 * Starts a tutorial recording in `dir` and returns the steps to drive it.
 *
 * The only way to reach a page after `open` is `click` on something a person can see, so a
 * viewer learns where each link lives. `page.goto`, `reload`, `goBack`, `goForward` and
 * `location` changes throw or fail `finish`.
 */
export async function startTutorial({ chromium, dir, size = { width: 1280, height: 720 }, slowMo = 120, pause = 900 }) {
  fs.mkdirSync(dir, { recursive: true });
  const browser = await chromium.launch({ slowMo });
  const context = await browser.newContext({ viewport: size, recordVideo: { dir, size } });
  await context.addInitScript(OVERLAY);
  const page = await context.newPage();

  const steps = [];
  const stray = [];
  let opened = false;
  let clicking = false;
  let lastClickAt = 0;

  for (const method of ["goto", "reload", "goBack", "goForward"]) {
    const original = page[method].bind(page);
    page[method] = async (...args) => {
      if (!opened && method === "goto") return original(...args);
      throw new Error(`tutorial rule: page.${method}() jumps to a page without showing how. Click a visible link in the nav instead.`);
    };
  }
  page.on("framenavigated", (frame) => {
    if (frame !== page.mainFrame() || !opened) return;
    if (clicking || Date.now() - lastClickAt < 4000) return;
    stray.push(frame.url());
  });

  const caption = async (text) => {
    await page.evaluate((t) => window.__tutCaption?.(t), text).catch(() => {});
  };
  const wait = (ms = pause) => page.waitForTimeout(ms);
  const pointer = async (locator) => {
    await locator.scrollIntoViewIfNeeded();
    const box = await locator.boundingBox();
    if (!box) throw new Error("tutorial rule: the target has no visible box, so a viewer cannot see it");
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 30 });
    await page.evaluate((b) => window.__tutRing?.(b), box);
    return box;
  };
  const record = (kind, locator, text) => steps.push({ kind, text, where: locator });
  const place = async (locator) => {
    const inNav = await locator.evaluate((el, sel) => !!el.closest(sel), NAV).catch(() => false);
    return inNav ? "nav" : "page";
  };

  const api = {
    page,

    /** Loads the first page. The only direct load a tutorial may make. */
    async open(url, text = "Start here") {
      await page.goto(url);
      opened = true;
      await caption(text);
      steps.push({ kind: "open", text, where: "start", url });
      await wait();
    },

    /** Moves the cursor to `locator`, rings it, says why, then clicks. */
    async click(locator, text) {
      await locator.waitFor({ state: "visible" });
      await caption(text);
      await pointer(locator);
      await wait(600);
      steps.push({ kind: "click", text, where: await place(locator) });
      clicking = true;
      try {
        await locator.click();
        await page.waitForLoadState("domcontentloaded");
      } finally {
        clicking = false;
        lastClickAt = Date.now();
      }
      await page.evaluate(() => window.__tutRing?.(null)).catch(() => {});
      await caption(text);
      await wait();
    },

    /** Types into a field the way a person does, one key at a time. */
    async type(locator, value, text) {
      await locator.waitFor({ state: "visible" });
      await caption(text);
      await pointer(locator);
      await locator.click();
      await locator.pressSequentially(value, { delay: 90 });
      steps.push({ kind: "type", text, where: await place(locator), value });
      await page.evaluate(() => window.__tutRing?.(null)).catch(() => {});
      await wait();
    },

    /** Holds a moment on what the viewer should notice. */
    async show(locator, text) {
      await locator.waitFor({ state: "visible" });
      await caption(text);
      await pointer(locator);
      steps.push({ kind: "show", text, where: await place(locator) });
      await wait(1800);
      await page.evaluate(() => window.__tutRing?.(null)).catch(() => {});
    },

    /** Closes the recording, writes `steps.md` beside it and returns the video path. Fails on a stray navigation. */
    async finish() {
      await wait(1200);
      const video = page.video();
      await context.close();
      await browser.close();
      const file = await video.path();
      const lines = steps.map((s, i) => `${i + 1}. [${s.kind}${s.where === "nav" ? ", in the navigation" : ""}] ${s.text}`);
      fs.writeFileSync(path.join(dir, "steps.md"), lines.join("\n") + "\n");
      if (stray.length) {
        throw new Error(`tutorial rule: the page changed without a click on it (${stray.join(", ")}). A viewer cannot follow that.`);
      }
      return file;
    },
  };
  return api;
}
