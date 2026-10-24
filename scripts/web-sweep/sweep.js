// Drives every screen of the web app in Chromium, as a Pixel 7 and as a desktop browser, and
// prints one row per step: works, broken, or unusable on touch, with the console errors and
// failed requests the step caused.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/sweep.js [phone|desktop ...]
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// PLAYWRIGHT names the playwright package when `require("playwright")` can't find it.
// Screenshots and sweep-<profile>.json land in SWEEP_DIR. Exits 1 when any step is not "works".
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");
const { devices } = playwright;

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const HOME = path.join(DIR, "home");

const PROFILES = {
  phone: { ...devices["Pixel 7"] },
  desktop: { viewport: { width: 1280, height: 800 }, hasTouch: false, isMobile: false },
  safari: { engine: "webkit", viewport: { width: 1280, height: 800 }, hasTouch: false, isMobile: false },
};

class Broken extends Error {}
class TouchUnusable extends Error {}
const check = (ok, what) => {
  if (!ok) throw new Broken(what);
};
const touchCheck = (ok, what) => {
  if (!ok) throw new TouchUnusable(what);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// iOS Safari keeps the layout viewport its full height under the on-screen keyboard, shrinks the
// visual viewport, and pans it down to the focused field. Chromium has no such mode, so the phone
// profile swaps in a visual viewport whose height, offsets, and scale a step can set; otherwise it
// mirrors the real one.
function stubVisualViewport() {
  const real = window.visualViewport;
  const stub = new EventTarget();
  let set = {};
  for (const key of ["width", "height", "offsetTop", "offsetLeft", "pageTop", "pageLeft", "scale"]) {
    Object.defineProperty(stub, key, { get: () => set[key] ?? real[key] });
  }
  for (const type of ["resize", "scroll"]) real.addEventListener(type, () => stub.dispatchEvent(new Event(type)));
  window.setVisualViewport = (next) => {
    const was = { height: stub.height, offsetTop: stub.offsetTop, scale: stub.scale };
    set = next;
    if (stub.height !== was.height || stub.scale !== was.scale) stub.dispatchEvent(new Event("resize"));
    if (stub.offsetTop !== was.offsetTop) stub.dispatchEvent(new Event("scroll"));
  };
  Object.defineProperty(window, "visualViewport", { value: stub, configurable: true });
}

async function api(route, body, token) {
  const res = await fetch(`${HUB}${route}`, {
    method: "POST",
    headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: JSON.stringify(body ?? {}),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${route} ${res.status} ${text}`);
  return text ? JSON.parse(text) : null;
}

// The account's connectors can only be filled by a machine (an app's first sign-in), so a
// throwaway machine imports accounts with fake keys. It stays in the account, offline.
async function seedConnectors(name) {
  const { token } = await api("/api/login", { email: EMAIL, password: PASSWORD });
  const machine = await api("/api/machines", { name, os: "linux" }, token);
  const ws = new WebSocket(`${HUB.replace("http", "ws")}/ws/machine`);
  const reply = new Promise((resolve, reject) => {
    ws.onmessage = (e) => {
      const frame = JSON.parse(e.data);
      if (frame.type === "reply") ("err" in frame ? reject(new Error(frame.err)) : resolve(frame.ok));
    };
    ws.onerror = () => reject(new Error("seed socket failed"));
  });
  await new Promise((r) => (ws.onopen = r));
  ws.send(JSON.stringify({ type: "auth", token: machine.token, name, os: "linux", version: "sweep" }));
  const store = {
    linear: { name: "Sweep Bot", email: "bot@example.test", organization: "Acme" },
    slack: { team: "Acme", userId: "U1", name: "pinned-bot" },
    github: { login: "sweeper", name: "Sweep" },
    connectors: [],
  };
  ws.send(JSON.stringify({ type: "call", id: 1, call: { cmd: "import", store, linear_key: "lin_api_fake", slack_key: "xoxb-fake", github_key: "ghp_fake" } }));
  const imported = await reply.finally(() => ws.close());
  if (!imported) throw new Error("the account already had connectors, so the seed was not imported");
}

async function sweep(profileName) {
  const { engine = "chromium", ...profile } = PROFILES[profileName];
  const touch = !!profile.hasTouch;
  const shots = path.join(DIR, "shots", profileName);
  fs.mkdirSync(shots, { recursive: true });
  const browser = await playwright[engine].launch({ headless: true });
  const context = await browser.newContext({ ...profile, permissions: engine === "chromium" ? ["clipboard-read", "clipboard-write"] : [] });
  if (touch) await context.addInitScript(stubVisualViewport);
  await context.addInitScript(() => localStorage.getItem("mode") || localStorage.setItem("mode", "terminal"));
  const page = await context.newPage();

  let seen = [];
  page.on("console", (m) => ["error", "warning"].includes(m.type()) && seen.push(`console.${m.type()}: ${m.text()}`));
  page.on("pageerror", (e) => seen.push(`pageerror: ${e.message}`));
  page.on("requestfailed", (r) => seen.push(`requestfailed: ${r.method()} ${r.url()} ${r.failure()?.errorText}`));
  page.on("response", (r) => r.status() >= 400 && seen.push(`HTTP ${r.status()}: ${r.request().method()} ${r.url()}`));

  const tapOrClick = (selector) => (touch ? page.tap(selector) : page.click(selector));
  const press = async (locator) => (touch ? locator.tap() : locator.click());
  const visible = (selector) => page.locator(selector).filter({ visible: true });
  const text = async (selector) => (await page.locator(selector).first().textContent())?.trim() ?? "";
  const termText = () =>
    page.evaluate(() => {
      const buffer = active?.term.buffer.active;
      if (!buffer) return "";
      let text = "";
      for (let i = 0; i < buffer.length; i++) {
        const line = buffer.getLine(i);
        text += (i && !line.isWrapped ? "\n" : "") + line.translateToString(true);
      }
      return text;
    });
  async function waitFor(what, test, ms = 8000) {
    const start = Date.now();
    for (;;) {
      if (await test()) return;
      if (Date.now() - start > ms) throw new Broken(`timed out waiting for ${what}`);
      await sleep(100);
    }
  }
  const waitTerm = (re, ms) => waitFor(`terminal to show ${re}`, async () => re.test(await termText()), ms);
  const typeInTerm = async (keys) => {
    await page.locator(".term:not([hidden]) .xterm-helper-textarea").focus();
    await page.keyboard.type(keys);
  };
  const rect = (selector) => page.locator(selector).first().evaluate((el) => el.getBoundingClientRect().toJSON());
  const overflowing = () =>
    page.evaluate(() =>
      [...document.querySelectorAll("body *")]
        .filter((el) => {
          const r = el.getBoundingClientRect();
          if (!r.width || !el.checkVisibility?.() || el.closest(".xterm-helpers, [aria-hidden=true]")) return false;
          // Children of a horizontal scroller may extend past the screen by design.
          for (let p = el.parentElement; p; p = p.parentElement) {
            const s = getComputedStyle(p);
            if (["auto", "scroll", "hidden"].includes(s.overflowX) && p.scrollWidth > p.clientWidth + 1) return false;
          }
          return r.right > innerWidth + 1 || r.left < -1;
        })
        .map((el) => el.id || el.className || el.tagName)
        .slice(0, 5),
    );
  const smallTargets = (selector) =>
    page.locator(selector).evaluateAll((els) =>
      els
        .filter((el) => el.checkVisibility())
        .map((el) => [el.textContent.trim() || el.title || el.className, el.getBoundingClientRect()])
        .filter(([, r]) => r.width < 40 || r.height < 40)
        .map(([name, r]) => `${name} ${Math.round(r.width)}x${Math.round(r.height)}`),
    );
  async function swipe(selector, dy) {
    const r = await rect(selector);
    const x = r.left + r.width / 2;
    const y = r.top + r.height / 2;
    const cdp = await context.newCDPSession(page);
    const point = (yy) => [{ x, y: yy, id: 1 }];
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: point(y) });
    for (let i = 1; i <= 10; i++) await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: point(y + (dy * i) / 10) });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await cdp.detach();
  }
  const settingsPage = async (name) => {
    if (!(await page.evaluate(() => document.body.dataset.page === "settings"))) await tapOrClick("#settings-toggle");
    await tapOrClick(`#settings-nav [data-page="${name}"]`);
  };
  const action = (name, nth = 0) => press(page.locator(`#settings-page [data-action="${name}"]`).nth(nth));
  const hubCall = (call) => page.evaluate((c) => hub.hub(c), call);
  const tabCount = () => page.locator("#tab-list .tab").count();
  const openFolderFromPicker = async (folder, { bypass = false } = {}) => {
    await tapOrClick("#new-tab");
    await page.waitForSelector("#projects .choices button");
    if (bypass) await tapOrClick("#projects .bypass");
    await press(page.locator("#projects .choices button", { hasText: "Browse folders…" }));
    await page.fill("#projects .browse input", `${folder}/`);
    await waitFor("the folder listing", async () => (await text("#projects .folders button")).startsWith("Open "));
    await press(page.locator("#projects .folders button").first());
  };

  const results = [];
  async function step(name, fn) {
    seen = [];
    let verdict = "works";
    let note = "";
    try {
      note = (await fn()) ?? "";
    } catch (e) {
      verdict = e instanceof TouchUnusable ? "unusable on touch" : "broken";
      note = e.message.split("\n")[0];
    }
    const thrown = seen.find((s) => s.startsWith("pageerror"));
    if (verdict === "works" && thrown) [verdict, note] = ["broken", thrown];
    await page.screenshot({ path: path.join(shots, `${String(results.length + 1).padStart(2, "0")}-${name.replace(/\W+/g, "-")}.png`) }).catch(() => {});
    results.push({ step: name, verdict, note, seen: [...new Set(seen)] });
    console.log(`${profileName.padEnd(7)} ${verdict.padEnd(17)} ${name}${note ? `: ${note}` : ""}`);
  }

  await page.goto(HUB);

  // The page's scripts share one global scope, where a second function of the same name silently wins.
  await step("scripts: no global name declared twice", async () => {
    const owners = new Map();
    for (const file of fs.readdirSync(path.join(__dirname, "../../ui")).filter((f) => f.endsWith(".js"))) {
      const source = fs.readFileSync(path.join(__dirname, "../../ui", file), "utf8");
      for (const [, name] of source.matchAll(/^(?:async )?(?:function|const|let|var|class) ([\w$]+)/gm)) owners.set(name, [...(owners.get(name) ?? []), file]);
    }
    const twice = [...owners].filter(([, files]) => files.length > 1).map(([name, files]) => `${name} in ${files.join(", ")}`);
    check(!twice.length, twice.join("; "));
    return `${owners.size} names`;
  });

  await step("scripts: the menu parser reads recorded claude screens", async () => {
    const screen = (name) => fs.readFileSync(path.join(__dirname, "screens", name), "utf8").split("\n");
    const idle = await page.evaluate((lines) => menuOn(lines), screen("claude-2.1.310-idle.txt"));
    check(idle === null, `an idle screen parsed as ${JSON.stringify(idle)}`);
    const menu = await page.evaluate((lines) => menuOn(lines), screen("claude-2.1.310-permission.txt"));
    check(menu?.box === "touch made-by-bash-2.txt", `the command box is ${JSON.stringify(menu?.box)}`);
    check(menu.question.split("\n").at(-1) === "Do you want to proceed?" && menu.question.startsWith("Bash command"), `the question is ${JSON.stringify(menu.question)}`);
    const labels = menu.options.map((o) => o.label);
    check(labels.length === 4 && labels[1] === "Yes, and always allow access to /Users/me/work/app from this project" && menu.options[0].chosen, `options: ${labels.join(" | ")}`);
    return labels.join(" | ");
  });

  await step("login: wrong password shows an error", async () => {
    await page.fill("#login [name=email]", EMAIL);
    await page.fill("#login [name=password]", "not the password");
    await tapOrClick("#login .primary");
    await waitFor("an error", async () => (await text("#login .error")) !== "");
    return text("#login .error");
  });

  await step("sign up a new account, empty state", async () => {
    await page.fill("#login [name=email]", `new-${profileName}-${Date.now()}@example.test`);
    await page.fill("#login [name=password]", "a fresh password");
    await tapOrClick("#login [name=signup]");
    await page.waitForFunction(() => document.body.dataset.hub === "online");
    check(await page.locator("#empty").isVisible(), "the empty-state hint is hidden");
    await tapOrClick("#new-tab");
    await sleep(300);
    const said = (await page.locator("#projects").isVisible()) ? await text("#projects") : "";
    check(/machine/i.test(said), "＋ with no machine online shows nothing");
    return said;
  });

  await step("sign out", async () => {
    await settingsPage("account");
    await action("sign-out");
    await page.waitForSelector("#login", { state: "visible" });
    check((await page.evaluate(() => localStorage.getItem("hub-token"))) === null, "the token stays in storage");
  });

  await step("sign in", async () => {
    await page.fill("#login [name=email]", EMAIL);
    await page.fill("#login [name=password]", PASSWORD);
    await tapOrClick("#login .primary");
    await page.waitForFunction(() => document.body.dataset.hub === "online");
    await waitFor("the node", () => page.evaluate(() => [...machines.values()].some((m) => m.view.online)));
  });

  await step("viewport: keyboard resizes the page, safe areas padded", async () => {
    const meta = await page.getAttribute("meta[name=viewport]", "content");
    const css = fs.readFileSync(path.join(__dirname, "../../ui/style.css"), "utf8");
    check(/interactive-widget=resizes-content/.test(meta), "the on-screen keyboard overlays the page (no interactive-widget=resizes-content)");
    check(/safe-area-inset-bottom/.test(css), "nothing pads the safe-area insets");
  });

  await step("＋ picker: machines row, choices, bypass toggle", async () => {
    await tapOrClick("#new-tab");
    await page.waitForSelector("#projects .choices button");
    const chips = await page.locator("#projects .machine").allTextContents();
    check(chips.some((c) => c.includes("sweep-node")), `machines row lacks the node: ${chips}`);
    const choices = await page.locator("#projects .choices button .name").allTextContents();
    check(choices.includes("Browse folders…") && choices.includes("Clone from GitHub…"), `choices: ${choices}`);
    await tapOrClick("#projects .bypass");
    check((await text("#projects .bypass")).endsWith("on"), "bypass did not turn on");
    await tapOrClick("#projects .bypass");
    check((await text("#projects .bypass")).endsWith("off"), "bypass did not turn off");
    if (touch) {
      const small = await smallTargets("#projects button");
      touchCheck(!small.length, `small targets: ${small.join(", ")}`);
    }
    return choices.join(" | ");
  });

  await step("Clone from GitHub while not connected", async () => {
    await press(page.locator("#projects .choices button", { hasText: "Clone from GitHub…" }));
    await waitFor("the error", async () => (await text("#picker-error")) !== "");
    const said = await text("#picker-error");
    check(/Connect GitHub/.test(said), said);
    await page.keyboard.press("Escape");
    return said;
  });

  await step("Browse folders: type a path, step in, open", async () => {
    if (!(await page.locator("#projects").isVisible())) await tapOrClick("#new-tab");
    await page.waitForSelector("#projects .choices button");
    await press(page.locator("#projects .choices button", { hasText: "Browse folders…" }));
    await page.fill("#projects .browse input", "~/work/al");
    await waitFor("alpha", async () => (await page.locator("#projects .folders button", { hasText: "alpha" }).count()) === 1);
    await press(page.locator("#projects .folders button", { hasText: "alpha" }));
    check((await page.inputValue("#projects .browse input")).endsWith("/work/alpha/"), "choosing a folder did not step into it");
    await waitFor("Open entry", async () => (await text("#projects .folders button")).startsWith("Open "));
    await press(page.locator("#projects .folders button").first());
    await waitTerm(/fake claude in .*work\/alpha/);
    check(!(await page.locator("#projects").isVisible()), "the picker stayed open");
  });

  await step("header: title, status, dots", async () => {
    const title = await text("#title");
    check(/sweep-node\s+\S*work\/alpha$/.test(title), `title is "${title}"`);
    check((await page.getAttribute("#tab-list .tab.active", "data-status")) !== null, "the tab has no status dot");
    return title;
  });

  await step("tap the terminal focuses its input", async () => {
    await page.locator("#settings-toggle").focus();
    await press(page.locator(".term:not([hidden]) .xterm-screen"));
    const focused = await page.evaluate(() => document.activeElement?.className);
    check(focused === "xterm-helper-textarea", `focus went to ${focused}`);
  });

  await step("type a prompt, status turns done", async () => {
    await typeInTerm("hello web");
    await page.keyboard.press("Enter");
    await waitTerm(/echo: hello web/);
    await waitFor("done status", async () => (await page.getAttribute("header", "data-status")) === "done");
    return text("#status-text");
  });

  await step("keys row: Esc Tab ⇧Tab ↑ ↓ Ctrl-C Enter", async () => {
    if (!touch) return check(!(await page.locator("#keys").isVisible()), "the keys row shows on desktop");
    check(await page.locator("#keys").isVisible(), "the keys row is hidden on the phone");
    for (const key of ["Esc", "Tab", "⇧Tab", "↑", "↓", "Ctrl-C"]) await page.tap(`#keys button:text-is("${key}")`);
    await waitTerm(/\[esc\]\[tab\]\[shift-tab\]\[up\]\[down\]\[ctrl-c\]/);
    await typeInTerm("k");
    await page.tap('#keys button:text-is("Enter")');
    await waitTerm(/echo: k/);
    const small = await smallTargets("#keys button");
    touchCheck(!small.length, `small keys: ${small.join(", ")}`);
  });

  await step("paste text into the terminal", async () => {
    await page.evaluate(() => navigator.clipboard.writeText("pasted words"));
    if (touch) {
      const paste = page.locator("#keys button", { hasText: /^Paste$/ });
      touchCheck((await paste.count()) === 1, "no way to paste text on a touch screen: the terminal has no paste menu and the keys row has no Paste");
      await paste.tap();
    } else {
      await page.locator(".term:not([hidden]) .xterm-helper-textarea").focus();
      await page.keyboard.press("ControlOrMeta+V");
    }
    await waitTerm(/pasted words/);
    await page.keyboard.press("Enter");
    await waitTerm(/echo: .*pasted words/);
  });

  await step("scroll the terminal back", async () => {
    await typeInTerm("long");
    await page.keyboard.press("Enter");
    await waitTerm(/line 120 of a long answer/);
    const before = await page.evaluate(() => active.term.buffer.active.viewportY);
    if (touch) await swipe(".term:not([hidden]) .xterm-screen", 300);
    else {
      const r = await rect(".term:not([hidden]) .xterm-screen");
      await page.mouse.move(r.left + 50, r.top + 50);
      await page.mouse.wheel(0, -600);
    }
    await sleep(300);
    const after = await page.evaluate(() => active.term.buffer.active.viewportY);
    (touch ? touchCheck : check)(after < before, `viewport stayed at line ${after}`);
    await page.evaluate(() => active.term.scrollToBottom());
    return `line ${before} to ${after}`;
  });

  await step("select terminal text", async () => {
    if (!touch) {
      const r = await rect(".term:not([hidden]) .xterm-screen");
      await page.mouse.move(r.left + 2, r.top + 2);
      await page.mouse.down();
      await page.mouse.move(r.left + 200, r.top + 40);
      await page.mouse.up();
      const picked = await page.evaluate(() => active.term.getSelection());
      check(picked.length > 0, "dragging selected nothing");
      return `${picked.length} characters`;
    }
    const select = page.locator("#keys button", { hasText: /^Select$/ });
    touchCheck((await select.count()) === 1, "xterm has no touch selection and nothing offers the text another way");
    await select.tap();
    const sheet = page.locator("#term-text");
    check(await sheet.isVisible(), "the text sheet did not open");
    check(/line 120 of a long answer/.test(await sheet.textContent()), "the sheet lacks the terminal's text");
    check((await sheet.evaluate((el) => getComputedStyle(el).userSelect)) !== "none", "the sheet's text can't be selected");
    await select.tap();
    check(!(await sheet.isVisible()), "Select again did not close the sheet");
  });

  await step("on-screen keyboard: terminal refits above it", async () => {
    if (!touch) return "desktop has no on-screen keyboard";
    const vp = page.viewportSize();
    const rows = await page.evaluate(() => active.term.rows);
    await page.setViewportSize({ width: vp.width, height: vp.height - 320 });
    await sleep(400);
    const shrunk = await page.evaluate(() => ({ rows: active.term.rows, bottom: document.querySelector("#terms").getBoundingClientRect().bottom }));
    await page.setViewportSize(vp);
    await sleep(400);
    const back = await page.evaluate(() => active.term.rows);
    check(shrunk.rows < rows && shrunk.bottom <= vp.height - 320 + 1, `rows ${rows} -> ${shrunk.rows}, terminal bottom ${shrunk.bottom}`);
    check(back === rows, `rows did not come back: ${back}`);
    return `rows ${rows} -> ${shrunk.rows} -> ${back}`;
  });

  // Where each edge sits on the screen: the visual viewport's top is screen y 0.
  const onScreen = () =>
    page.evaluate(() => {
      const buffer = active.term.buffer.active;
      return {
        top: document.querySelector("#expanded").getBoundingClientRect().top - visualViewport.offsetTop,
        bottom: document.querySelector("#terms").getBoundingClientRect().bottom - visualViewport.offsetTop,
        rows: active.term.rows,
        atBottom: buffer.viewportY === buffer.baseY,
      };
    });
  const keyboard = async (viewport) => {
    await page.evaluate((v) => setVisualViewport(v), viewport);
    await sleep(400);
  };

  await step("Safari keyboard: the page follows the panned visual viewport", async () => {
    if (!touch) return "desktop has no on-screen keyboard";
    const vp = page.viewportSize();
    await page.evaluate(() => active.term.scrollToBottom());
    const rows = await page.evaluate(() => active.term.rows);
    await keyboard({ height: 500 });
    const open = await onScreen();
    await keyboard({ height: 500, offsetTop: 120 });
    const panned = await onScreen();
    await keyboard({ height: 500, offsetTop: 120, scale: 2 });
    const zoomed = await onScreen();
    await keyboard({});
    const closed = await onScreen();
    check(open.rows < rows && open.bottom <= 501, `keyboard open: rows ${rows} -> ${open.rows}, terminal bottom ${open.bottom} of 500`);
    check(panned.top >= -1 && panned.bottom <= 501, `panned by 120: header top ${panned.top}, terminal bottom ${panned.bottom} of 500`);
    check(panned.atBottom, "the terminal scrolled away from the cursor line");
    check(zoomed.top === -120, `pinch-zoomed: header top ${zoomed.top}, the pan must not move the page`);
    check(closed.rows === rows && closed.top === 0 && closed.bottom <= vp.height + 1, `keyboard closed: rows ${closed.rows}, header top ${closed.top}, terminal bottom ${closed.bottom}`);
    return `rows ${rows} -> ${open.rows} -> ${closed.rows}; panned: header top ${panned.top}, terminal bottom ${panned.bottom}`;
  });

  // Focuses the field, opens the panned keyboard over it, and says where it sits on the screen.
  const fieldUnderKeyboard = async (field) => {
    await field.focus();
    await keyboard({ height: 500, offsetTop: 120 });
    const r = await field.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const clip = el.closest("#settings-page")?.getBoundingClientRect();
      return { top: r.top - visualViewport.offsetTop, bottom: r.bottom - visualViewport.offsetTop, clipped: !!clip && (r.top < clip.top || r.bottom > clip.bottom) };
    });
    await keyboard({});
    check(!r.clipped, `the settings page scrolled the field out of view (${r.top}..${r.bottom} on screen)`);
    check(r.top >= 0 && r.bottom <= 500, `the field sits at ${r.top}..${r.bottom} of 500`);
    return `field at ${Math.round(r.top)}..${Math.round(r.bottom)} of 500`;
  };

  await step("Safari keyboard: a focused settings field stays on screen", async () => {
    if (!touch) return "desktop has no on-screen keyboard";
    await settingsPage("account");
    try {
      return await fieldUnderKeyboard(page.locator(".new-password [name=password]"));
    } finally {
      await tapOrClick("#settings-toggle");
    }
  });

  await step("landscape", async () => {
    if (!touch) return "desktop";
    const vp = page.viewportSize();
    await page.setViewportSize({ width: vp.height, height: vp.width });
    await sleep(400);
    const rows = await page.evaluate(() => active.term.rows);
    const over = await overflowing();
    await page.screenshot({ path: path.join(shots, "landscape.png") });
    await page.setViewportSize(vp);
    await sleep(300);
    check(!over.length, `overflowing: ${over}`);
    touchCheck(rows >= 10, `only ${rows} terminal rows in landscape`);
    return `${rows} rows`;
  });

  await step("needs-you status", async () => {
    await typeInTerm("/ask");
    await page.keyboard.press("Enter");
    await waitFor("needs-input", async () => (await page.getAttribute("header", "data-status")) === "needs-input");
    const said = await text("#status-text");
    await page.keyboard.press("Escape");
    await waitTerm(/chose esc/);
    return said;
  });

  await step("claude exits, Enter restarts it", async () => {
    await typeInTerm("/exit");
    await page.keyboard.press("Enter");
    await waitTerm(/claude exited, press Enter to restart/);
    if (touch) await page.tap('#keys button:text-is("Enter")');
    else await page.keyboard.press("Enter");
    await waitFor("a second banner", async () => ((await termText()).match(/fake claude in/g) ?? []).length >= 2);
  });

  await step("usage meter", async () => {
    await waitFor("usage", () => page.evaluate(() => !document.getElementById("usage").hidden), 4000).catch(() => {});
    const shown = await page.locator("#usage").isVisible();
    check(shown, "the usage meter is not shown");
    return text("#usage");
  });

  await step("usage meter after a reload", async () => {
    await page.reload();
    await page.waitForFunction(() => document.body.dataset.hub === "online");
    await waitFor("the tab", async () => (await tabCount()) >= 1);
    await sleep(500);
    check(await page.locator("#usage").isVisible(), "a page loaded after the last statusline report shows no usage");
    return text("#usage");
  });

  await step("second tab from the projects list, with bypass", async () => {
    await openFolderFromPicker("~/work/beta", { bypass: true });
    await waitTerm(/fake claude in .*work\/beta \(bypass\)/);
    check((await tabCount()) === 2, `${await tabCount()} tabs`);
    check((await text("#tab-list .tab.active .name")).startsWith("⚠"), "the bypass tab has no ⚠");
  });

  await step("switch tabs", async () => {
    await press(page.locator("#tab-list .tab .name", { hasText: "alpha" }));
    check(/alpha/.test(await text("#title")), "tapping the alpha tab did not show it");
    await waitTerm(/work\/alpha/);
  });

  const gui = (selector) => page.locator(selector).filter({ visible: true }).first();
  const sendPrompt = async (words) => {
    await page.fill("#prompt", words);
    if (touch) await page.tap("#send");
    else {
      // Enter in the `/` menu picks its highlighted command, so a word sent as typed closes it first.
      if (await page.locator('#composer-menu:is([data-kind="slash"], [data-kind="mention"])').isVisible()) await page.press("#prompt", "Escape");
      await page.press("#prompt", "Enter");
    }
  };
  const showSidebar = async () => {
    if (await page.locator("#sidebar").isVisible()) return;
    await tapOrClick("#chat-sidebar-toggle");
    await waitFor("the sidebar", () => page.locator("#sidebar").isVisible());
  };

  await step("app view: switch from the terminal, threads in the sidebar", async () => {
    await typeInTerm("typed in the terminal");
    await page.keyboard.press("Enter");
    await waitTerm(/echo: typed in the terminal/);
    await tapOrClick("#mode-gui");
    await waitFor("the app view", () => page.locator("#chat").isVisible());
    check(!(await page.locator("#tab-bar").isVisible()), "the tab bar still shows in the app view");
    await showSidebar();
    const rows = await page.locator("#thread-list .thread-row").count();
    check(rows === 2, `${rows} threads in the sidebar, expected 2`);
    check(/alpha/.test(await text("#thread-list .thread-row.active .project")), "the active row is not alpha");
    if (touch) await page.tap("#workspace", { position: { x: (await rect("#workspace")).width - 4, y: 300 } });
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    return `${rows} threads`;
  });

  await step("app view: the thread shows what was typed in the terminal", async () => {
    await waitFor("the transcript", async () => (await page.locator("#thread .bubble").count()) > 0);
    const prompts = await page.locator("#thread .bubble").allTextContents();
    check(prompts.includes("typed in the terminal"), `prompts: ${prompts.join(" | ")}`);
    check((await page.locator("#thread .assistant", { hasText: "echo: typed in the terminal" }).count()) === 1, "the answer is missing");
    check(/alpha/.test(await text("#chat-head .crumb")), "the header doesn't name the project");
    return `${prompts.length} prompts`;
  });

  await step("app view: a prompt from the composer reaches the same session", async () => {
    await sendPrompt("hello from the app");
    await waitFor("the answer", async () => (await page.locator("#thread .assistant", { hasText: "echo: hello from the app" }).count()) > 0);
    check(/hello from the app/.test(await termText()), "the terminal behind the app view never got the prompt");
    check((await page.inputValue("#prompt")) === "", "the composer kept the sent text");
  });

  await step("app view: a turn's changed files open in the Diff panel", async () => {
    await sendPrompt("edit");
    await waitFor("the changed-files card", async () => (await page.locator("#thread .changes").count()) > 0, 10000);
    const card = await text("#thread .changes .count");
    await press(gui("#thread .open-diff"));
    await waitFor("the diff panel", () => page.locator("#panel-body .file").first().isVisible());
    check(/notes\.md/.test(await text("#panel-body .file .path")), "the diff doesn't show notes.md");
    const added = await page.locator("#panel-body .dl.add").count();
    check(added >= 2, `${added} added lines in the diff`);
    await press(gui('#panel-body [data-diff="split"]'));
    check((await page.locator("#panel-body .dl.split").count()) > 0, "split view shows no side-by-side rows");
    await press(gui('#panel-body [data-diff="unified"]'));
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    if (touch) await tapOrClick("#panel-close");
    return `${card}, ${added} lines added`;
  });

  await step("app view: the PR link opens the pull request panel", async () => {
    check(await page.locator("#pr-chip").isVisible(), "the header has no PR chip after the session linked #42");
    check(/#42/.test(await text("#composer-footer .pr")), "the composer footer doesn't show #42");
    await tapOrClick("#pr-chip");
    const prPanelSettled = async () => /#42/.test(await text("#panel-body")) && !/Loading/.test(await text("#panel-body"));
    await waitFor("the PR panel", prPanelSettled, 15000);
    check(await page.locator('#panel-tabs [data-panel="pr"].active').isVisible(), "the PR tab is not the active one");
    const shown = (await text("#panel-body")).slice(0, 80);
    await tapOrClick('#thread .pr-link[data-pr$="/pull/41"]');
    await waitFor("#41 in the PR panel", async () => /#41/.test(await text("#panel-body")) && !/Loading/.test(await text("#panel-body")), 15000);
    check(/#41/.test(await text('#panel-tabs [data-panel="pr"]')), `the PR tab reads ${await text('#panel-tabs [data-panel="pr"]')} after clicking #41`);
    if (!touch) {
      const tabsBefore = await page.locator("#thread-list .thread-row").count();
      await page.keyboard.press("Meta+w");
      await waitFor("⌘W to close the panel", async () => !(await page.locator("#panel").isVisible()));
      check((await page.locator("#thread-list .thread-row").count()) === tabsBefore, "⌘W closed a thread while the panel was open");
      await tapOrClick("#toggle-panel");
    }
    if (touch) {
      await tapOrClick("#panel-close");
      await tapOrClick("#toggle-panel");
    }
    return shown;
  });

  await step("app view: the browser panel loads a URL", async () => {
    await press(gui('#panel-tabs [data-panel="browser"]'));
    await page.fill("#panel-body .url-bar input", `${HUB}/manifest.webmanifest`);
    await page.press("#panel-body .url-bar input", "Enter");
    await waitFor("the frame", async () => (await page.locator("#panel-body iframe").getAttribute("src"))?.endsWith("manifest.webmanifest"));
    const frame = page.frameLocator("#panel-body iframe");
    await waitFor("the page in the frame", async () => /pinned/.test((await frame.locator("body").textContent().catch(() => "")) ?? ""));
    await tapOrClick("#panel-close");
    check(!(await page.locator("#panel").isVisible()), "the panel didn't close");
    await sleep(200);
  });

  await step("app view: the approval banner answers claude's prompt", async () => {
    await sendPrompt("/ask");
    await waitFor("the banner", () => page.locator("#approval").isVisible());
    check(/Command approval/.test(await text("#approval .kind")), `banner says ${await text("#approval .kind")}`);
    check((await text("#approval pre")) === "rm -rf build", `the banner shows the command as ${await text("#approval pre")}`);
    check((await text("#approval .about")) === "Remove the build folder", `the banner describes it as ${await text("#approval .about")}`);
    check(/Do you want to proceed/.test(await text("#approval .what")), `the banner asks ${await text("#approval .what")}`);
    const options = await page.locator("#approval .options button").allTextContents();
    check(options.length === 3 && options[1].startsWith("Yes, and don't ask again"), `options: ${options.join(" | ")}`);
    await page.fill("#prompt", "typed while claude asks");
    check(!/typed while claude asks/.test(await termText()), "the composer typed into claude's menu");
    await page.fill("#prompt", "");
    const model = await text("#model-button");
    await tapOrClick("#model-button");
    await tapOrClick("#composer-menu button");
    await sleep(300);
    check(!/\/model/.test(await termText()), "picking a model typed into claude's menu");
    check((await text("#model-button")) === model, `the model button changed to ${await text("#model-button")} with nothing sent`);
    await press(gui('#approval [data-option="2"]'));
    await waitTerm(/chose 2/);
    await waitFor("the banner to go", async () => !(await page.locator("#approval").isVisible()), 1500);
    const row = await text("#thread-list .thread-row.active");
    check(!/ran it/.test(await termText()), "claude finished before the banner was checked");
    check(!/Approval/.test(row), `after the answer, while claude still works: ${JSON.stringify(row)}`);
    await waitTerm(/ran it/);
    return options.join(" | ");
  });

  await step("app view: claude's folder trust menu, answered from the banner", async () => {
    await showSidebar();
    await tapOrClick("#new-thread");
    await page.waitForSelector("#projects .choices button");
    await press(page.locator("#projects .choices button", { hasText: "Browse folders…" }));
    await page.fill("#projects .browse input", `${HOME}/work/gamma/`);
    await waitFor("the folder listing", async () => (await text("#projects .folders button")).startsWith("Open "));
    await press(page.locator("#projects .folders button").first());
    await waitFor("the trust banner", () => page.locator("#approval .options button").first().isVisible());
    const options = await page.locator("#approval .options button").allTextContents();
    check(options.join("|") === "No, exit|Yes, I trust this folder", `options: ${options.join(" | ")}`);
    check(/No, exit/.test(await text("#approval [data-chosen]")), "the banner doesn't mark claude's own default");
    await press(gui('#approval [data-option="2"]'));
    await waitTerm(/fake claude in .*work\/gamma/);
    await waitFor("the banner to go", async () => !(await page.locator("#approval").isVisible()));
    await page.evaluate(() => closeTab(active));
    await waitFor("two threads again", async () => (await page.locator("#thread-list .thread-row").count()) === 2);
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
    return options.join(" | ");
  });

  await step("composer: the menu parser reads claude 2.1's questions and mode line", async () => {
    const screen = (name) => fs.readFileSync(path.join(__dirname, "screens", name), "utf8").split("\n");
    const question = await page.evaluate((lines) => menuOn(lines), screen("claude-2.1.310-question.txt"));
    const options = question?.options.map((o) => `${o.head}${o.more ? ` (${o.more})` : ""}`).join(" | ");
    check(options === "Red (warm and bold) | Blue (calm and cool) | Type something. | Chat about this", `question options: ${options}`);
    check(question.question === "☐ Color\nWhich color do you prefer?", `the question is ${JSON.stringify(question.question)}`);
    const permission = await page.evaluate((lines) => menuOn(lines), screen("claude-2.1.310-permission.txt"));
    check(permission.amend === true, "the permission menu's Tab to amend wasn't read");
    const idle = await page.evaluate((lines) => menuOn(lines), screen("claude-2.1.310-manual-mode.txt"));
    check(idle === null, `the idle prompt box parsed as ${JSON.stringify(idle)}`);
    const plan = await page.evaluate((lines) => menuOn(lines), screen("claude-2.1.310-plan.txt"));
    const planned = plan?.options.map((o) => `${o.head}${o.more ? ` (${o.more})` : ""}`).join(" | ");
    check(planned === "Yes, and use auto mode | Yes, manually approve edits | Tell Claude what to change (shift+tab to approve with this feedback)", `plan options: ${planned}`);
    const modes = await page.evaluate((screens) => screens.map(modeOn), ["manual-mode", "accept-edits", "idle"].map((s) => screen(`claude-2.1.310-${s}.txt`)));
    check(modes.join() === "manual,accept,", `modes: ${modes.join()}`);
    return options;
  });

  await step("composer: a question shows option descriptions and takes number keys or a typed answer", async () => {
    await sendPrompt("/question");
    await waitFor("the question", () => page.locator("#approval .choice").first().isVisible());
    const labels = await page.locator("#approval .choice .label").allTextContents();
    const about = await page.locator("#approval .choice .about").allTextContents();
    check(labels.join("|") === "Red|Blue|Type something.|Chat about this" && about.join("|") === "warm and bold|calm and cool", `options: ${labels.join("|")} / ${about.join("|")}`);
    check((await page.locator("#approval .choice[data-key='4']").count()) === 1, "the options have no 1-9 key hints");
    const placeholder = await page.getAttribute("#prompt", "placeholder");
    check(placeholder.startsWith("Type your own answer"), `placeholder: ${placeholder}`);
    await page.fill("#prompt", "");
    await page.focus("#prompt");
    await page.keyboard.press("2");
    await waitTerm(/answered: Blue/);
    check((await page.inputValue("#prompt")) === "", "the 2 went into the prompt");
    await waitFor("the banner to go", async () => !(await page.locator("#approval").isVisible()));
    await sendPrompt("/question");
    await waitFor("the question again", () => page.locator("#approval .choice").first().isVisible());
    await sendPrompt("Green, please");
    await waitTerm(/answered: Green, please/);
    await waitFor("the banner to go", async () => !(await page.locator("#approval").isVisible()));
    return `${labels.join(" | ")}; ${placeholder}`;
  });

  await step("composer: typing while claude asks for permission declines with that feedback", async () => {
    await sendPrompt("/ask");
    await waitFor("the banner", () => page.locator("#approval").isVisible());
    const placeholder = await page.getAttribute("#prompt", "placeholder");
    check(placeholder === "Send feedback to decline this request", `placeholder: ${placeholder}`);
    await sendPrompt("use the trash instead");
    await waitTerm(/declined: use the trash instead/);
    await waitFor("the banner to go", async () => !(await page.locator("#approval").isVisible()));
    check((await page.inputValue("#prompt")) === "", "the composer kept the feedback");
    await waitFor("claude's answer", async () => (await page.locator("#thread .assistant", { hasText: "Understood: use the trash instead" }).count()) > 0);
    return placeholder;
  });

  await step("composer: claude's plan is refined by typed feedback, or implemented from a blank send", async () => {
    await sendPrompt("/plan-ready");
    await waitFor("the plan banner", async () => (await text("#approval .kind")) === "Plan ready");
    const placeholder = await page.getAttribute("#prompt", "placeholder");
    check(placeholder === "Add feedback to refine the plan, or leave this blank to implement it", `placeholder: ${placeholder}`);
    await sendPrompt("also say goodbye");
    await waitTerm(/answered: also say goodbye/);
    await waitFor("the banner to go", async () => !(await page.locator("#approval").isVisible()));
    await sendPrompt("/plan-ready");
    await waitFor("the plan banner again", async () => (await text("#approval .kind")) === "Plan ready");
    await sendPrompt("");
    await waitTerm(/answered: Yes, and use auto mode/);
    await waitFor("the banner to go", async () => !(await page.locator("#approval").isVisible()));
    return placeholder;
  });

  await step("composer: the mode control shows claude's mode and sets it with Shift+Tab", async () => {
    check(!(await page.locator("#mode-button").isVisible()), "a mode shows before claude drew its mode line");
    await sendPrompt("/modes");
    await waitFor("the mode control", async () => (await text("#mode-button")) === "Supervised");
    await tapOrClick("#mode-button");
    const modes = await page.locator("#composer-menu .label").allTextContents();
    check(modes.join("|") === "Supervised|Auto-accept edits|Plan|Auto", `modes: ${modes.join("|")}`);
    check((await page.locator("#composer-menu button.checked .label").allTextContents()).join() === "Supervised", "the menu doesn't check Supervised");
    await press(page.locator("#composer-menu button", { hasText: "Plan" }).first());
    await waitFor("Plan", async () => (await text("#mode-button")) === "Plan");
    check(/plan mode on/.test(await termText()), "claude's screen never reached plan mode");
    if (!touch) {
      await page.focus("#prompt");
      await page.keyboard.press("Shift+Tab");
      await waitFor("Shift+Tab to cycle to Auto", async () => (await text("#mode-button")) === "Auto");
      await page.keyboard.press("Meta+Shift+A");
      await waitFor("⌘⇧A to open the mode menu", () => page.locator("#composer-menu button.checked", { hasText: "Auto" }).isVisible());
      await page.keyboard.press("Escape");
    }
    await tapOrClick("#mode-button");
    await press(page.locator("#composer-menu button", { hasText: "Supervised" }).first());
    await waitFor("Supervised again", async () => (await text("#mode-button")) === "Supervised");
    return modes.join(" | ");
  });

  await step("composer: the model menu checks the current model, ⌘⇧M opens it", async () => {
    if (touch) await tapOrClick("#model-button");
    else await page.keyboard.press("Meta+Shift+M");
    await waitFor("the model menu", () => page.locator("#composer-menu").isVisible());
    const checked = await page.locator("#composer-menu button.checked .label").allTextContents();
    check(checked.join() === "Default", `checked: ${checked.join()}`);
    if (touch) await page.tap("#thread");
    else await page.keyboard.press("Escape");
    await waitFor("the menu to close", async () => !(await page.locator("#composer-menu").isVisible()));
  });

  await step("composer: / lists claude's commands with the project's and user's own", async () => {
    const own = [[path.join(HOME, "work/alpha/.claude/commands/ship-it.md"), "---\ndescription: Ship the current branch\n---\nShip it."], [path.join(HOME, ".claude/skills/tidy-up/SKILL.md"), "---\nname: tidy-up\ndescription: Tidy the code\n---\n"]];
    for (const [file, body] of own) {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, body);
    }
    try {
      await page.fill("#prompt", "/");
      await waitFor("the / menu", () => page.locator('#composer-menu[data-kind="slash"] [data-pick="/ship-it"]').isVisible());
      const names = await page.locator("#composer-menu [data-pick]").evaluateAll((els) => els.map((e) => e.dataset.pick.slice(1)));
      check(names.includes("compact") && names.includes("tidy-up"), `commands: ${names.join(", ")}`);
      await page.fill("#prompt", "/ship");
      await waitFor("ship-it first", async () => (await page.locator("#composer-menu [data-pick]").first().getAttribute("data-pick")) === "/ship-it");
      check((await text("#composer-menu [data-pick] .source")) === "project", "ship-it isn't marked as the project's");
      await page.fill("#prompt", "/tidy");
      await waitFor("tidy-up first", async () => (await page.locator("#composer-menu [data-pick]").first().getAttribute("data-pick")) === "/tidy-up");
      if (touch) await page.tap("#composer-menu [data-pick]");
      else await page.press("#prompt", "Enter");
      check((await page.inputValue("#prompt")) === "/tidy-up ", `the pick left ${JSON.stringify(await page.inputValue("#prompt"))}`);
      check(!(await page.locator("#composer-menu").isVisible()), "the menu stayed open after the pick");
      await page.fill("#prompt", "");
      return `${names.length} commands`;
    } finally {
      for (const dir of ["work/alpha/.claude", ".claude/skills"]) fs.rmSync(path.join(HOME, dir), { recursive: true, force: true });
    }
  });

  await step("composer: @ lists the folder's files and folders", async () => {
    fs.mkdirSync(path.join(HOME, "work/alpha/docs"), { recursive: true });
    fs.writeFileSync(path.join(HOME, "work/alpha/docs/notes-guide.md"), "guide");
    try {
      await page.fill("#prompt", "look at @");
      await waitFor("the @ menu", () => page.locator('#composer-menu[data-kind="mention"] [data-pick="@docs/"]').isVisible());
      await page.fill("#prompt", "look at @notes");
      await waitFor("notes.md first", async () => (await page.locator("#composer-menu [data-pick]").first().getAttribute("data-pick")) === "@notes.md");
      const picks = await page.locator("#composer-menu [data-pick]").evaluateAll((els) => els.map((e) => e.dataset.pick));
      check(picks.join() === "@notes.md,@docs/notes-guide.md", `files: ${picks.join()}`);
      if (touch) await page.tap("#composer-menu [data-pick]");
      else await page.press("#prompt", "Enter");
      check((await page.inputValue("#prompt")) === "look at @notes.md ", `the pick left ${JSON.stringify(await page.inputValue("#prompt"))}`);
      await page.fill("#prompt", "");
      return picks.join(", ");
    } finally {
      fs.rmSync(path.join(HOME, "work/alpha/docs"), { recursive: true, force: true });
    }
  });

  await step("composer: the composer rests as one row while the thread is scrolled up", async () => {
    await page.fill("#prompt", "");
    await page.evaluate(() => document.activeElement.blur());
    await page.evaluate(() => (threadView.scrollTop = 0));
    await waitFor("the resting composer", () => page.evaluate(() => composer.dataset.resting === "true"));
    const resting = (await rect("#composer .box")).height;
    check(resting < 60, `the resting composer is ${Math.round(resting)}px tall`);
    await page.focus("#prompt");
    await waitFor("the full composer", () => page.evaluate(() => composer.dataset.resting === "false"));
    const full = (await rect("#composer .box")).height;
    check(full > 100, `the focused composer is ${Math.round(full)}px tall`);
    await page.evaluate(() => (threadView.scrollTop = threadView.scrollHeight));
    return `${Math.round(resting)}px → ${Math.round(full)}px`;
  });

  await step("composer: ArrowUp and ArrowDown walk the thread's prompts", async () => {
    const bubbles = await page.locator("#thread .bubble").allTextContents();
    const sent = bubbles.filter((b, i) => b !== bubbles[i - 1]);
    await page.fill("#prompt", "");
    await page.focus("#prompt");
    const walk = [];
    for (const key of ["ArrowUp", "ArrowUp", "ArrowDown", "ArrowDown"]) {
      await page.keyboard.press(key);
      walk.push(await page.inputValue("#prompt"));
    }
    check(JSON.stringify(walk) === JSON.stringify([sent.at(-1), sent.at(-2), sent.at(-1), ""]), `walked ${JSON.stringify(walk)} over ${JSON.stringify(sent.slice(-2))}`);
    return walk.join(" → ");
  });

  await step("composer: the More menu switches sending to ⌘Enter", async () => {
    const toggle = async () => {
      await tapOrClick("#more-button");
      await press(page.locator("#composer-menu button", { hasText: "Send with ⌘Enter" }));
    };
    await toggle();
    check((await page.getAttribute("#send", "title")) === "Send (⌘Enter)", `the send button says ${await page.getAttribute("#send", "title")}`);
    if (!touch) {
      await page.fill("#prompt", "two");
      await page.press("#prompt", "Enter");
      await page.keyboard.type("lines");
      check((await page.inputValue("#prompt")) === "two\nlines", `Enter left ${JSON.stringify(await page.inputValue("#prompt"))}`);
      await page.press("#prompt", "Meta+Enter");
      await waitTerm(/echo: two/);
      check((await page.inputValue("#prompt")) === "", "⌘Enter didn't send");
    }
    await toggle();
    check((await page.getAttribute("#send", "title")) === "Send (Enter)", "the setting didn't switch back");
  });

  await step("composer: attached files show above the prompt and go out with it", async () => {
    const attach = async (file) => {
      const [chooser] = await Promise.all([page.waitForEvent("filechooser"), tapOrClick("#composer-attach")]);
      await chooser.setFiles(file);
    };
    const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==", "base64");
    await attach({ name: "shot.png", mimeType: "image/png", buffer: png });
    await waitFor("the thumbnail", () => page.locator("#attachments .attachment img").isVisible());
    check((await text("#attachments .name")) === "shot.png", `the chip reads ${await text("#attachments .name")}`);
    check((await page.inputValue("#prompt")) === "", "the file's path went into the text");
    await sendPrompt("what is in this picture");
    await waitTerm(/echo: what is in this picture \S*shot\.png/);
    await waitFor("the chips to go", async () => !(await page.locator("#attachments").isVisible()));
    await attach({ name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from("hi") });
    await waitFor("the file chip", () => page.locator("#attachments .attachment").isVisible());
    await press(page.locator("#attachments .remove"));
    check(!(await page.locator("#attachments").isVisible()), "the chip's X left it there");
    await attach({ name: "huge.bin", mimeType: "application/octet-stream", buffer: Buffer.alloc(26 << 20) });
    await waitFor("the upload error", async () => /over 25 MB/.test(await text("#composer-note")));
    check(await page.locator("#composer-note").isVisible(), "the upload error isn't in the app view");
    return await text("#composer-note");
  });

  await step("composer: a draft stays with its thread, and ⌘S stashes it", async () => {
    const row = (name) => page.locator("#thread-list .thread-row", { hasText: name }).first();
    await page.fill("#prompt", "half-written idea");
    await showSidebar();
    await press(row("beta"));
    await waitFor("beta", async () => /beta/.test(await text("#chat-head .crumb")));
    check((await page.inputValue("#prompt")) === "", "beta shows alpha's draft");
    await showSidebar();
    check(await row("alpha").locator(".draft-pen").isVisible(), "alpha's row has no draft pen");
    await press(row("alpha"));
    await waitFor("alpha", async () => /alpha/.test(await text("#chat-head .crumb")));
    check((await page.inputValue("#prompt")) === "half-written idea", `alpha's draft came back as ${JSON.stringify(await page.inputValue("#prompt"))}`);
    if (touch) {
      await page.fill("#prompt", "");
      return "drafts; ⌘S needs a keyboard";
    }
    await page.focus("#prompt");
    await page.keyboard.press("Meta+s");
    await waitFor("the stash tab", () => page.locator("#stash").isVisible());
    check((await text("#stash .count")) === "1" && (await page.inputValue("#prompt")) === "", "⌘S didn't move the draft to the stash");
    await tapOrClick("#stash");
    await press(page.locator("#composer-menu button", { hasText: "half-written idea" }).first());
    check((await page.inputValue("#prompt")) === "half-written idea", "the stash didn't restore the draft");
    check(!(await page.locator("#stash").isVisible()), "the stash tab stayed with nothing in it");
    await page.fill("#prompt", "");
  });

  await step("composer: an empty thread centers the composer, and Send spins until claude takes it", async () => {
    await showSidebar();
    await press(page.locator("#thread-list .thread-row", { hasText: "beta" }).first());
    await waitFor("beta", async () => /beta/.test(await text("#chat-head .crumb")));
    await waitFor("the centered composer", () => page.evaluate(() => document.getElementById("chat").dataset.hero === "true"));
    const [box, chat] = [await rect("#composer .box"), await rect("#chat")];
    check(Math.abs(box.top + box.height / 2 - (chat.top + chat.height / 2)) < chat.height / 5, `the composer sits at ${Math.round(box.top)} in a ${Math.round(chat.height)}px column`);
    await page.evaluate(() => {
      const transport = machines.get(active.machine).transport;
      const call = transport.call;
      transport.call = (c) => (c.cmd === "prompt" ? new Promise((r) => setTimeout(r, 800)).then(() => call(c)) : call(c));
      window.restoreTransport = () => (transport.call = call);
    });
    try {
      await sendPrompt("hello beta");
      await waitFor("the spinner", () => page.evaluate(() => composer.dataset.sending === "true" && document.getElementById("send").innerHTML.includes("M21 12a9 9")), 700);
      await waitTerm(/echo: hello beta/);
    } finally {
      await page.evaluate(() => restoreTransport());
    }
    await waitFor("the composer back at the bottom", () => page.evaluate(() => document.getElementById("chat").dataset.hero === "false"));
    check(!(await page.evaluate(() => composer.dataset.sending)), "the spinner stayed");
  });

  await step("composer: the branch picker switches branches, ⌘⇧G, and the Git action asks claude", async () => {
    const git = (...args) => require("child_process").execFileSync("git", ["-C", path.join(HOME, "work/beta"), "-c", "user.email=s@example.test", "-c", "user.name=s", ...args]).toString().trim();
    git("commit", "-q", "--allow-empty", "-m", "first");
    git("branch", "feature-x");
    const start = git("symbolic-ref", "--short", "HEAD");
    if (touch) await tapOrClick("#composer-footer .branch");
    else await page.keyboard.press("Meta+Shift+G");
    await waitFor("the branch list", () => page.locator("#composer-menu [data-ref]").first().isVisible());
    const refs = await page.locator("#composer-menu [data-ref]").evaluateAll((els) => els.map((e) => `${e.dataset.ref}${e.disabled ? " (current)" : ""}`));
    check(refs.join() === `${start} (current),feature-x`, `branches: ${refs.join()}`);
    await page.fill("#composer-menu .search", "feat");
    await press(page.locator('#composer-menu [data-ref="feature-x"]'));
    await waitFor("feature-x in the footer", async () => (await text("#composer-footer .branch")) === "feature-x");
    check(git("symbolic-ref", "--short", "HEAD") === "feature-x", "the folder didn't switch");
    await tapOrClick("#composer-footer .branch");
    await waitFor("the branch list", () => page.locator("#composer-menu .search").isVisible());
    await page.fill("#composer-menu .search", "try-this");
    await press(page.locator('#composer-menu [data-create="try-this"]'));
    await waitFor("try-this in the footer", async () => (await text("#composer-footer .branch")) === "try-this");
    await waitFor("the Git action", async () => (await text("#git-action .main")) === "Push & create PR");
    await tapOrClick("#git-action .main");
    await waitTerm(/echo: Push the current branch, then open a pull request with gh pr create\./);
    await tapOrClick("#git-action .more");
    const actions = await page.locator("#git-menu .label").allTextContents();
    check(actions.join("|") === "Commit|Push|Create PR", `Git menu: ${actions.join("|")}`);
    await tapOrClick("#git-action .more");
    await showSidebar();
    await press(page.locator("#thread-list .thread-row", { hasText: "alpha" }).first());
    await waitFor("alpha", async () => /alpha/.test(await text("#chat-head .crumb")));
    return refs.join(", ");
  });

  await step("composer: the terminal drawer resizes from its top edge", async () => {
    await tapOrClick("#toggle-terminal");
    await waitFor("the drawer", () => page.locator("#drawer-resize").isVisible());
    const dragTo = async (dy) => {
      const r = await rect("#drawer-resize");
      const [x, y] = [r.left + r.width / 2, r.top + r.height / 2];
      await page.mouse.move(x, y);
      await page.mouse.down();
      await page.mouse.move(x, y + dy, { steps: 6 });
      await page.mouse.up();
      return (await rect("#terms")).height;
    };
    const before = (await rect("#terms")).height;
    const taller = await dragTo(-120);
    check(Math.abs(taller - before - 120) < 8, `dragged 120px up: ${Math.round(before)} → ${Math.round(taller)}`);
    check(Number(await page.evaluate(() => localStorage.getItem("gui-drawer-height"))) === Math.round(taller), "the height wasn't saved");
    const room = (await rect("#workspace")).height;
    const top = await dragTo(-2000);
    check(Math.abs(top - room * 0.75) < 3, `dragged to the top: ${Math.round(top)} of ${Math.round(room)}`);
    const bottom = await dragTo(2000);
    check(Math.abs(bottom - 180) < 3, `dragged to the bottom: ${Math.round(bottom)}`);
    const r = await rect("#drawer-resize");
    await page.mouse.dblclick(r.left + r.width / 2, r.top + r.height / 2);
    await waitFor("the default height", async () => Math.abs((await rect("#terms")).height - 280) < 3);
    await tapOrClick("#toggle-terminal");
    return `${Math.round(before)} → ${Math.round(taller)}, ${Math.round(top)}, ${Math.round(bottom)}`;
  });

  await step("app view: the terminal drawer shows the same session", async () => {
    await tapOrClick("#toggle-terminal");
    await waitFor("the drawer", () => page.locator(".term:not([hidden])").evaluate((el) => getComputedStyle(el).visibility === "visible" && el.getBoundingClientRect().height > 60));
    check(/hello from the app/.test(await termText()), "the drawer's terminal lacks the prompt sent from the composer");
    await tapOrClick("#toggle-terminal");
  });

  await step("app view: the sidebar switches threads", async () => {
    await showSidebar();
    await press(page.locator("#thread-list .thread-row", { hasText: "beta" }).first());
    await waitFor("beta", async () => /beta/.test(await text("#chat-head .crumb")));
    await showSidebar();
    await press(page.locator("#thread-list .thread-row", { hasText: "alpha" }).first());
    await waitFor("alpha", async () => /alpha/.test(await text("#chat-head .crumb")));
  });

  await step("app view: controls are big enough to tap", async () => {
    if (!touch) return "desktop";
    const small = await smallTargets("#chat-head button, #composer button, #composer-footer button");
    touchCheck(!small.length, `small: ${small.join(", ")}`);
  });

  await step("app view: back to the terminal", async () => {
    await showSidebar();
    await tapOrClick("#mode-terminal");
    await waitFor("the tab bar", () => page.locator("#tab-bar").isVisible());
    check(!(await page.locator("#chat").isVisible()), "the app view still shows");
    await waitTerm(/work\/alpha/);
    if (!touch) {
      await page.keyboard.press("Meta+e");
      await waitFor("⌘E to open the app view", () => page.locator("#chat").isVisible());
      const panelOpen = () => page.evaluate(() => document.body.dataset.panel === "open");
      const before = await panelOpen();
      await page.keyboard.press("Meta+Alt+KeyB");
      check((await panelOpen()) !== before, "⌘⌥B didn't toggle the side panel");
      await page.keyboard.press("Meta+Alt+KeyB");
      await page.keyboard.press("Meta+e");
      await waitFor("⌘E to go back to the terminal", () => page.locator("#tab-bar").isVisible());
      check(!(await page.locator("#chat").isVisible()), "⌘E left the app view up");
    }
  });

  await step("session actions: skip permissions mid-session, then ask again", async () => {
    const shown = () => page.locator("#actions").evaluate((el) => getComputedStyle(el).opacity === "1");
    const openMenu = async () => {
      if (!touch) {
        const terms = await rect("#terms");
        await page.mouse.move(terms.left + terms.width / 2, terms.top + terms.height / 2);
        await waitFor("the actions button to fade out over the terminal", async () => !(await shown()));
        const edge = await rect("#actions-edge");
        await page.mouse.move(edge.left + edge.width / 2, edge.top + edge.height / 2);
        await waitFor("the actions button to fade in", shown);
      }
      check(await shown(), "the actions button is not shown");
      await tapOrClick("#actions-toggle");
      await page.waitForSelector("#actions-menu button");
    };
    const item = (label) => page.locator("#actions-menu button", { hasText: label });
    await typeInTerm("hello\r");
    await waitTerm(/echo: hello/);

    await openMenu();
    await page.keyboard.press("Escape");
    check(await page.locator("#actions-menu").isHidden(), "Escape left the menu open");
    await openMenu();
    const labels = await page.locator("#actions-menu button > span:first-child").allTextContents();
    check(labels.join("|") === "⚠ Skip all permissions|Attach files…|Change folder…|Close tab", `menu: ${labels}`);
    await press(item("Skip all permissions"));
    await waitTerm(/work\/alpha \(bypass\)\s*>\s*$/);
    await waitFor("the ⚠ on the tab", async () => (await text("#tab-list .tab.active .name")).startsWith("⚠"));
    check((await tabCount()) === 2, `${await tabCount()} tabs`);

    await openMenu();
    await press(item("Ask for permissions again"));
    await waitTerm(/work\/alpha\s*>\s*$/);
    await waitFor("the ⚠ to go", async () => !(await text("#tab-list .tab.active .name")).startsWith("⚠"));
    if (touch) {
      const small = await smallTargets("#actions-toggle");
      touchCheck(!small.length, `small targets: ${small.join(", ")}`);
    }
  });

  await step("drag a tab to reorder", async () => {
    const names = () => page.locator("#tab-list .tab .name").allTextContents();
    const before = await names();
    const first = await rect("#tab-list .tab:nth-child(1)");
    const second = await rect("#tab-list .tab:nth-child(2)");
    if (touch) {
      const cdp = await context.newCDPSession(page);
      const at = (x) => [{ x, y: first.top + first.height / 2, id: 1 }];
      await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: at(first.left + 10) });
      await sleep(600);
      for (let i = 1; i <= 10; i++) await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: at(first.left + 10 + ((second.right - first.left) * i) / 10) });
      await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
      await cdp.detach();
    } else {
      await page.mouse.move(first.left + 10, first.top + first.height / 2);
      await page.mouse.down();
      await page.mouse.move(second.right - 2, first.top + first.height / 2, { steps: 10 });
      await page.mouse.up();
    }
    const after = await names();
    (touch ? touchCheck : check)(after[0] === before[1], `order stayed ${after}`);
    return `${before} -> ${after}`;
  });

  await step("tab bar side", async () => {
    if (touch) return "the phone keeps the tabs on top";
    const handle = await rect("#tab-bar .handle");
    await page.mouse.move(handle.left + 3, handle.top + 3);
    await page.mouse.down();
    await page.mouse.move(5, 400, { steps: 10 });
    await page.mouse.up();
    check((await page.evaluate(() => document.body.dataset.tabs)) === "left", "the bar did not move left");
    await page.mouse.move(handle.left + 3, 200);
    const back = await rect("#tab-bar .handle");
    await page.mouse.move(back.left + 3, back.top + 3);
    await page.mouse.down();
    await page.mouse.move(640, 70, { steps: 10 });
    await page.mouse.up();
    check((await page.evaluate(() => document.body.dataset.tabs)) === "top", "the bar did not come back to the top");
  });

  await step("📁 moves the tab to another folder", async () => {
    await press(page.locator("#tab-list .tab .name", { hasText: "alpha" }));
    await tapOrClick("#folder");
    await page.waitForSelector("#projects[data-mode=move] .choices button");
    await press(page.locator("#projects .choices button", { hasText: "Browse folders…" }));
    await page.fill("#projects .browse input", "~/work/");
    await waitFor("listing", async () => (await page.locator("#projects .folders button", { hasText: "alpha" }).count()) === 1);
    await page.fill("#projects .browse input", `${HOME}/`);
    await waitFor("home listing", async () => (await page.locator("#projects .folders button", { hasText: "work" }).count()) === 1);
    await press(page.locator("#projects .folders button").first());
    await waitTerm(/fake claude in \S*\/home$/m);
    return text("#title");
  });

  await step("close a tab", async () => {
    const before = await tabCount();
    await press(page.locator("#tab-list .tab.active .close"));
    await waitFor("one tab fewer", async () => (await tabCount()) === before - 1);
    const tabs = await page.evaluate(() => hub.call([...machines.keys()].find((id) => machines.get(id).view.online), { cmd: "tabs" }));
    check(tabs.length === before - 1, `the node still has ${tabs.length} tabs`);
    if (touch) {
      const small = await smallTargets("#tab-list .tab .close, #new-tab, header button");
      touchCheck(!small.length, `small targets: ${small.join(", ")}`);
    }
  });

  await step("settings: Connectors bad keys show errors", async () => {
    await settingsPage("integrations");
    const errors = [];
    for (const [field, button, error] of [["#linear-key", "connect-linear", "#linear-error"], ["#slack-token", "connect-slack", "#slack-error"], ["#github-token", "connect-github", "#github-error"]]) {
      await page.fill(field, "definitely-not-a-key");
      await action(button);
      await waitFor(`${error}`, async () => (await text(error)) !== "", 15000);
      errors.push(await text(error));
    }
    const over = await overflowing();
    check(!over.length, `overflowing: ${over}`);
    return errors.map((e) => e.slice(0, 50)).join(" | ");
  });

  await step("settings: Connectors connected, then the GitHub list in ＋", async () => {
    await seedConnectors(`seed-${profileName}`);
    await settingsPage("projects");
    await settingsPage("integrations");
    await waitFor("connected badges", async () => (await page.locator('#settings-page .badge[data-on="true"]').count()) === 3);
    await tapOrClick("#settings-toggle");
    await tapOrClick("#new-tab");
    await page.waitForSelector("#projects .choices button");
    const offline = await page.locator("#projects .machine:disabled").allTextContents();
    check(offline.some((t) => t.includes(`seed-${profileName}`)), `the offline machine is not greyed out: ${offline}`);
    await press(page.locator("#projects .choices button", { hasText: "Clone from GitHub…" }));
    await waitFor("the GitHub answer", async () => (await text("#picker-error")) !== "" || (await page.locator("#projects .folders button").count()) > 0, 15000);
    const said = await text("#picker-error");
    await page.keyboard.press("Escape");
    await page.keyboard.press("Escape");
    return `fake token: ${said.slice(0, 60)}`;
  });

  await step("Workflows: create a Linear workflow", async () => {
    await settingsPage("connectors");
    await action("new-connector");
    await press(page.locator('#settings-page [data-action="pick-trigger"][data-kind="linear-label"]'));
    await page.fill('#settings-page [data-bind="trigger.label"]', "sweep");
    await press(page.locator('#settings-page .step[data-step="agent"] .node'));
    await page.fill('#settings-page [data-bind="folder"]', `${HOME}/work/alpha`);
    await page.locator('#settings-page [data-bind="folder"]').dispatchEvent("change");
    await action("save-connector");
    await waitFor("Saved", async () => (await text("#save-connector")) === "Saved");
    const over = await overflowing();
    check(!over.length, `overflowing: ${over}`);
    if (touch) {
      const small = await smallTargets("#settings-page .editor-bar button, #settings-page .switch input");
      touchCheck(!small.length, `small targets: ${small.join(", ")}`);
    }
  });

  await step("Workflows: edit steps and save", async () => {
    await press(page.locator('#settings-page .step[data-step="picked"] .node'));
    await action("remove-step");
    check((await text("#save-connector")) === "Save", "removing a step did not mark the draft changed");
    await action("save-connector");
    await waitFor("Saved", async () => (await text("#save-connector")) === "Saved");
    const saved = (await hubCall({ cmd: "get_settings" })).connectors[0];
    check(saved.columns.picked === "" && saved.trigger.label === "sweep", JSON.stringify(saved.columns));
  });

  await step("Workflows: the list's switch turns it off", async () => {
    await action("close-editor");
    await page.waitForSelector("#settings-page .flow-card");
    check((await page.locator("#settings-page .flow-card").count()) === 1, "the new workflow is listed twice");
    await press(page.locator("#settings-page .flow-card .switch"));
    await waitFor("off", async () => (await page.getAttribute("#settings-page .flow-card", "data-on")) === "false");
    check((await hubCall({ cmd: "get_settings" })).connectors[0].enabled === false, "the hub still has it enabled");
  });

  await step("Workflows: Slack workflow with a missing team", async () => {
    await action("new-connector");
    await press(page.locator('#settings-page [data-action="pick-trigger"][data-kind="slack-mention"]'));
    await page.fill('#settings-page [data-bind="trigger.channel"]', "support");
    await press(page.locator('#settings-page .step[data-step="bug"] .node'));
    await page.selectOption('#settings-page [data-bind="fix"]', "ask");
    await action("save-connector");
    await waitFor("the team error", async () => /team/.test(await text("#editor-error")));
    await page.fill('#settings-page [data-bind="team"]', "ENG");
    await action("save-connector");
    await waitFor("Saved", async () => (await text("#save-connector")) === "Saved");
    check((await hubCall({ cmd: "get_settings" })).connectors.length === 2, "the hub lacks the Slack workflow");
  });

  await step("Workflows: delete both", async () => {
    for (let i = 0; i < 2; i++) {
      if ((await page.locator("#settings-page .editor-bar").count()) === 0) await press(page.locator("#settings-page .flow-card .open").first());
      await action("delete-connector");
      await action("delete-connector");
      await page.waitForSelector("#settings-page .new-card");
    }
    check((await hubCall({ cmd: "get_settings" })).connectors.length === 0, "workflows remain");
  });

  await step("Projects: list and history", async () => {
    await settingsPage("projects");
    await waitFor("projects", async () => (await page.locator("#settings-page .project").count()) >= 2);
    const names = await page.locator("#settings-page .project b").allTextContents();
    await action("open-project");
    await waitFor("history", async () => (await page.locator("#settings-page .history").count()) === 1);
    const shared = await hubCall({ cmd: "shared_projects" });
    await action("projects-back");
    return `${names.join(", ")}; shared ${shared.map((p) => p.key).join(", ") || "none"}`;
  });

  await step("Machines: list, rename, remove", async () => {
    await settingsPage("machines");
    await page.waitForSelector("#settings-page .machine-row");
    const node = page.locator("#settings-page .machine-row", { hasText: "sweep-node" });
    await press(node.locator('[data-action="rename-machine"]'));
    await page.fill("#settings-page .rename", "sweep-node renamed");
    await page.keyboard.press("Enter");
    await waitFor("the new name", async () => (await page.locator("#settings-page .machine-row", { hasText: "sweep-node renamed" }).count()) === 1);
    await press(page.locator("#settings-page .machine-row", { hasText: "sweep-node renamed" }).locator('[data-action="rename-machine"]'));
    await page.fill("#settings-page .rename", "sweep-node");
    await page.keyboard.press("Enter");
    await waitFor("the old name", async () => (await page.locator("#settings-page .machine-row b", { hasText: /^sweep-node$/ }).count()) === 1);
    const seed = page.locator("#settings-page .machine-row", { hasText: `seed-${profileName}` });
    check(/seen/.test(await seed.textContent()), "the offline machine shows no last-seen time");
    await press(seed.locator('[data-action="remove-machine"]'));
    await action("confirm-remove");
    await waitFor("removed", async () => (await page.locator("#settings-page .machine-row", { hasText: `seed-${profileName}` }).count()) === 0);
    const over = await overflowing();
    check(!over.length, `overflowing: ${over}`);
    if (touch) {
      const small = await smallTargets("#settings-page .machine-row button, #settings-nav button");
      touchCheck(!small.length, `small targets: ${small.join(", ")}`);
    }
  });

  await step("Machines: pairing command and copy", async () => {
    await action("pair");
    await page.waitForSelector("#settings-page .row.command code");
    const command = await text("#settings-page .row.command code");
    check(command.startsWith(`curl -fsSL ${HUB}/install.sh | sh -s -- `), command);
    await action("copy-pair");
    await waitFor("Copied", async () => (await text('#settings-page [data-action="copy-pair"]')) === "Copied", 3000);
    // WebKit lets a page write the clipboard but never lets automation read it back.
    if (engine === "chromium") check((await page.evaluate(() => navigator.clipboard.readText())) === command, "the clipboard lacks the command");
    const script = await fetch(`${HUB}/install.sh`);
    check(script.ok && (await script.text()).startsWith("#!"), `install.sh answered ${script.status}`);
    return command;
  });

  await step("Connectors: disconnect all", async () => {
    await settingsPage("integrations");
    for (const name of ["disconnect-linear", "disconnect-slack", "disconnect-github"]) {
      await action(name);
      await page.waitForSelector(`#settings-page [data-action="${name}"]`, { state: "detached" });
    }
    check((await page.locator('#settings-page .badge[data-on="true"]').count()) === 0, "a connector still shows connected");
  });

  await step("Account: change the password", async () => {
    await settingsPage("account");
    await page.fill(".new-password [name=current]", "wrong current");
    await page.fill(".new-password [name=password]", "another password 2");
    await press(page.locator(".new-password .primary"));
    await waitFor("an error", async () => (await text("#password-error")) !== "");
    const wrong = await text("#password-error");
    for (const [current, next] of [[PASSWORD, "another password 2"], ["another password 2", PASSWORD]]) {
      await page.fill(".new-password [name=current]", current);
      await page.fill(".new-password [name=password]", next);
      await press(page.locator(".new-password .primary"));
      await waitFor("done", async () => (await text("#password-done")) !== "");
      await page.evaluate(() => (document.getElementById("password-done").textContent = ""));
    }
    const over = await overflowing();
    check(!over.length, `overflowing: ${over}`);
    return `wrong current: ${wrong}`;
  });

  await step("Settings at phone width: nav and pages fit", async () => {
    for (const name of ["integrations", "projects", "connectors", "account", "machines"]) {
      await settingsPage(name);
      await sleep(200);
      const over = await overflowing();
      check(!over.length, `${name} overflows: ${over}`);
    }
    if (touch) {
      const small = await smallTargets("#settings-nav button, #settings-page button.primary, #settings-page button.secondary");
      touchCheck(!small.length, `small targets: ${small.join(", ")}`);
    }
  });

  await step("session expiry signs the page out", async () => {
    await tapOrClick("#settings-toggle");
    const token = await page.evaluate(() => localStorage.getItem("hub-token"));
    await api("/api/logout", {}, token);
    await page.waitForSelector("#login", { state: "visible", timeout: 10000 });
    const said = await text("#login .error");
    check(/signed out/.test(said), `the form says "${said}"`);
    return said;
  });

  await step("a stale token at load shows the sign-in form", async () => {
    await page.evaluate(() => localStorage.setItem("hub-token", "stale"));
    await page.reload();
    await page.waitForSelector("#login", { state: "visible", timeout: 10000 });
    const said = await text("#login .error");
    check(/signed out/.test(said), `the form says "${said}"`);
    return said;
  });

  await step("Safari keyboard: the sign-in form's field stays on screen", async () => {
    if (!touch) return "desktop has no on-screen keyboard";
    return fieldUnderKeyboard(page.locator("#login [name=password]"));
  });

  await page.fill("#login [name=email]", EMAIL);
  await page.fill("#login [name=password]", PASSWORD);
  await tapOrClick("#login .primary");
  await page.waitForFunction(() => document.body.dataset.hub === "online");
  for (const id of await page.evaluate(() => tabs.map((t) => t.key))) {
    await page.evaluate((key) => closeTab(tabByKey(key)), id);
  }
  await browser.close();
  fs.writeFileSync(path.join(DIR, `sweep-${profileName}.json`), JSON.stringify(results, null, 2));
  return results;
}

(async () => {
  const names = process.argv.slice(2).length ? process.argv.slice(2) : Object.keys(PROFILES);
  let failed = false;
  for (const name of names) {
    const results = await sweep(name);
    failed ||= results.some((r) => r.verdict !== "works");
    console.log(`\n| ${name} step | result | console / network |\n|---|---|---|`);
    for (const r of results) console.log(`| ${r.step} | ${r.verdict}${r.note ? `: ${r.note}` : ""} | ${r.seen.join("; ").slice(0, 300) || "clean"} |`);
    console.log();
  }
  process.exit(failed ? 1 : 0);
})();
