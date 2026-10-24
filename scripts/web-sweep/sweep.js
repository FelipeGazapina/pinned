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
  const context = await browser.newContext({ ...profile, colorScheme: "dark", permissions: engine === "chromium" ? ["clipboard-read", "clipboard-write"] : [] });
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

  // The timeline's own content goes to beta, so alpha's turns stay as the steps after these expect.
  const lastTurn = (selector = "") => gui(`#thread .turn:last-of-type ${selector}`);
  const clipboard = () => (engine === "chromium" ? page.evaluate(() => navigator.clipboard.readText()) : null);
  const answered = (words) => waitFor(`the answer "${words}"`, async () => (await page.locator("#thread .assistant", { hasText: words }).count()) > 0, 10000);

  await step("app view: tool runs group under a summary, with thinking and highlighted code", async () => {
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/beta"))));
    await waitFor("beta", async () => /beta/.test(await text("#chat-head .crumb")));
    await sendPrompt("rich");
    await waitFor("the code block", async () => (await page.locator("#thread .answer .codeblock").count()) > 0, 10000);
    const fold = lastTurn(".work > summary");
    check(/^Worked for /.test(await fold.textContent()), `the fold reads ${await fold.textContent()}`);
    check(/\d{4}$/.test(await fold.locator(".when").getAttribute("title")), "the fold row has no time to show on hover");
    check(/Looked around/.test(await text("#thread .turn:last-of-type > .assistant")), "the first message is not in sight");
    await press(fold);
    const group = lastTurn(".group > summary");
    const summary = (await group.textContent()).trim();
    check(summary.startsWith("Read 2 files and ran 2 commands"), `the group reads ${summary}`);
    check((await group.locator(".failed").count()) === 1, "the group doesn't mark its failed command");
    check((await page.locator("#thread .turn:last-of-type .work > .steps > .tool").count()) === 1, "the lone Grep is not its own row");
    await press(group);
    const thought = await text("#thread .turn:last-of-type .group .tool.thought .label");
    check(thought === "Planning the read", `the thinking row reads ${thought}`);
    check((await text("#thread .codeblock .lang")) === "rust", `the code block's language is ${await text("#thread .codeblock .lang")}`);
    // Shiki with T3 Code's Pierre Dark theme colors `let` as storage, which highlight.js can't tell from a keyword.
    const colors = () => page.evaluate(() => [...document.querySelectorAll("#thread .codeblock code.language-rust span")].filter((s) => ["fn", "main", "let", "42"].includes(s.textContent)).map((s) => `${s.textContent}=${s.style.color}`).join(" "));
    await waitFor("Shiki's colors", async () => (await colors()) === "fn=rgb(255, 103, 141) main=rgb(157, 106, 251) let=rgb(213, 104, 234) 42=rgb(104, 205, 242)", 8000).catch(async () => check(false, `the code is colored ${await colors()}`));
    const block = gui("#thread .codeblock");
    check((await block.getAttribute("data-wrap")) === "true", "code doesn't wrap by default");
    await press(block.locator("[data-code-wrap]"));
    check((await block.getAttribute("data-wrap")) === "false", "Wrap didn't turn wrapping off");
    await press(block.locator("[data-copy-code]"));
    await waitFor("the copied check", async () => (await block.locator("[data-copy-code].copied").count()) === 1, 1500);
    const copied = await clipboard();
    check(copied === null || copied === "fn main() {\n    let answer = 42;\n}", `the clipboard holds ${JSON.stringify(copied)}`);
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    return summary;
  });

  await step("app view: links show their address, a GitHub PR link opens that PR", async () => {
    const pr = gui("#thread .answer a[data-pr]");
    check((await pr.getAttribute("title")) === "https://github.com/example/beta/pull/7", `the PR link's tooltip is ${await pr.getAttribute("title")}`);
    check((await pr.locator("svg.github").count()) === 1, "the GitHub link has no GitHub glyph");
    const docs = gui('#thread .answer a[href="https://example.com/docs"]');
    check((await docs.getAttribute("title")) === "https://example.com/docs", "the docs link has no tooltip");
    check((await docs.locator("svg").count()) === 0, "a non-GitHub link got the glyph");
    await press(pr);
    await waitFor("#7 in the PR tab", async () => /#7\b/.test(await text('#panel-tabs [data-panel="pr"]')), 15000);
    await tapOrClick("#toggle-panel");
    await waitFor("the panel to close", async () => !(await page.locator("#panel").isVisible()));
  });

  await step("app view: hovering a message shows its time and a Copy button", async () => {
    const user = lastTurn(".user");
    const meta = user.locator(".meta");
    if (!touch) {
      check((await meta.evaluate((el) => getComputedStyle(el).opacity)) === "0", "the user message's actions show before hover");
      await user.hover();
      await sleep(300);
    }
    check((await meta.evaluate((el) => getComputedStyle(el).opacity)) === "1", "hovering doesn't show the actions");
    const when = meta.locator(".when");
    check(/\d:\d\d/.test(await when.textContent()) && /\d{4}$/.test(await when.getAttribute("title")), `the time reads ${await when.textContent()} / ${await when.getAttribute("title")}`);
    await press(meta.locator("[data-copy-item]"));
    await waitFor("the copied check", async () => (await meta.locator(".copied").count()) === 1, 1500);
    check([null, "rich"].includes(await clipboard()), `the clipboard holds ${await clipboard()}`);
    const answer = lastTurn(".answer");
    if (!touch) await answer.hover();
    await press(answer.locator(".meta [data-copy-item]"));
    await waitFor("the answer's copied check", async () => (await answer.locator(".meta .copied").count()) === 1, 1500);
    const copied = await clipboard();
    check(copied === null || copied.startsWith("Here is the fix:"), `the clipboard holds ${copied}`);
  });

  await step("app view: a long prompt clamps, mentions are chips, images open full size", async () => {
    await sendPrompt("long-prompt");
    await answered("Here is the walkthrough.");
    const body = gui("#thread .bubble .body.clamped");
    check((await body.locator("ol > li").count()) === 12, "the prompt's numbered lines don't render as a list");
    check((await body.evaluate((el) => el.getBoundingClientRect().height)) <= 177, "the long prompt isn't clamped");
    check((await text("#thread .mention")) === "main.rs" && (await gui("#thread .mention").getAttribute("title")) === "src/app/main.rs", `the mention reads ${await text("#thread .mention")}`);
    const more = gui('#thread .bubble [data-toggle^="full:"]');
    check((await more.textContent()) === "Show full message", `the toggle reads ${await more.textContent()}`);
    await press(more);
    await waitFor("the full message", async () => (await page.locator("#thread .bubble .body.clamped").count()) === 0);
    check((await text('#thread .bubble [data-toggle^="full:"]')) === "Show less", "the toggle doesn't offer Show less");
    await sendPrompt("picture");
    await answered("A single dot.");
    const img = gui("#thread .bubble .picture img");
    check(await img.evaluate((el) => el.complete && el.naturalWidth === 1), "the picture didn't load");
    await press(gui("#thread .bubble .picture"));
    await waitFor("the zoomed image", () => page.locator("#image-zoom").isVisible());
    await tapOrClick("#image-zoom");
    await waitFor("the zoom to close", async () => !(await page.locator("#image-zoom").isVisible()));
  });

  await step("app view: errors, stopped turns and answered questions stay in sight", async () => {
    await sendPrompt("oops");
    await waitFor("the API error", async () => (await page.locator("#thread .error-row", { hasText: "API Error: 529 Overloaded" }).count()) === 1);
    check(/service is busy/.test(await text("#thread .error-row .message")), "the error's detail is missing");
    await sendPrompt("fail");
    await waitFor("the failed build", async () => (await page.locator("#thread .error-row", { hasText: "Build the app" }).count()) === 1);
    check(await gui("#thread .error-row:has-text('Build the app') .message").isVisible(), "the failure's output is not in sight");
    await sendPrompt("stop");
    await waitFor("the stopped turn", async () => /^You stopped (after |this response)/.test(await text("#thread .turn:last-of-type .work > summary .label")));
    await sendPrompt("ask");
    await answered("Blue it is.");
    const row = lastTurn(".tool.answered");
    check((await row.locator(".label").textContent()) === "Which color should the button be?", `the question row reads ${await row.textContent()}`);
    check((await row.locator(".message").textContent()) === "Blue", `the answer reads ${await row.locator(".message").textContent()}`);
  });

  await step("app view: a proposed plan shows as a card to copy or download", async () => {
    await sendPrompt("plan");
    await answered("Plan approved.");
    const card = lastTurn(".plan-card");
    check((await card.locator(".badge").textContent()) === "Plan" && (await card.locator("h3").textContent()) === "Ship the fix", `the card reads ${(await card.textContent()).slice(0, 60)}`);
    check(!/Summary/.test(await card.locator(".plan-body").textContent()), "the plan's title and Summary heading repeat in its body");
    check((await card.locator(".plan-body.clamped").count()) === 1, "the long plan isn't clamped");
    await press(card.locator(".plan-foot button"));
    await waitFor("the whole plan", async () => (await card.locator(".plan-foot button").textContent()) === "Collapse plan");
    await press(card.locator("[data-plan-menu]"));
    await press(card.locator("[data-plan-copy]"));
    const copied = await clipboard();
    check(copied === null || copied.startsWith("# Ship the fix"), `the clipboard holds ${copied?.slice(0, 40)}`);
    await press(card.locator("[data-plan-menu]"));
    const [download] = await Promise.all([page.waitForEvent("download"), press(card.locator("[data-plan-download]"))]);
    check(download.suggestedFilename() === "ship-the-fix.md", `the download is named ${download.suggestedFilename()}`);
  });

  await step("app view: the live row shows thinking and Running <program>, and /compact shows Compacting…", async () => {
    await sendPrompt("think");
    await waitFor("the live thought", async () => (await text("#thread .live-tool .label")) === "Weighing the options", 6000);
    await answered("Done thinking.");
    await sendPrompt("build");
    await waitFor("the running command", async () => (await text("#thread .live-tool .label")) === "Running npm", 6000);
    await answered("Built.");
    await sendPrompt("/compact");
    await waitFor("Compacting…", async () => (await text("#thread .work.live .shimmer")) === "Compacting…", 6000);
    await waitFor("the compacted note", async () => (await page.locator("#thread .note", { hasText: "The conversation was compacted." }).count()) > 0, 10000);
  });

  await step("app view: changed files show as a folder tree with 1.2k counts", async () => {
    await sendPrompt("tree");
    await answered("Wrote the tree.");
    await waitFor("the changed-files card", async () => (await page.locator("#thread .turn:last-of-type .changes").count()) > 0, 10000);
    const card = lastTurn(".changes");
    check(/\+1\.2k/.test(await card.locator("summary").textContent()), `the card reads ${await card.locator("summary").textContent()}`);
    await press(card.locator("summary .show"));
    await waitFor("the tree", async () => (await card.locator("li").count()) > 0);
    const shifted = await page.evaluate(() => [...document.querySelectorAll("#expanded, #workspace")].map((el) => el.scrollLeft).join(","));
    check(shifted === "0,0", `opening the card scrolled the view sideways: ${shifted}`);
    const rows = async () => (await card.locator("li").allTextContents()).map((r) => r.replace(/[+−].*/, "").trim());
    check((await rows()).join("|") === "docs|guide.md|src/app|lib.rs|main.rs|README.md", `rows: ${(await rows()).join("|")}`);
    check((await card.locator("summary").evaluate((el) => getComputedStyle(el).position)) === "sticky", "the card's header doesn't stick");
    await press(card.locator("[data-all-dirs]"));
    await waitFor("folders to collapse", async () => (await rows()).join("|") === "docs|src/app|README.md");
    await press(card.locator('li[data-dir="docs"]'));
    await waitFor("docs to open", async () => (await rows()).join("|") === "docs|guide.md|src/app|README.md");
    await press(card.locator("[data-all-dirs]"));
    await waitFor("folders to expand", async () => (await card.locator("li[data-file]").count()) === 4);
    return (await rows()).join(" | ");
  });

  await step("app view: a long thread loads earlier turns, and a pill scrolls to the end", async () => {
    await sendPrompt("many");
    await answered("answer 45");
    await waitFor("the earlier button", () => gui("#thread [data-earlier]").isVisible());
    const before = Number(await page.evaluate(() => threadView.querySelector(".lane").dataset.turns));
    check(before === 40, `${before} turns shown`);
    await page.evaluate(() => (threadView.scrollTop = 0));
    await waitFor("the scroll pill", () => page.locator("#scroll-end").isVisible(), 2000);
    await press(gui("#thread [data-earlier]"));
    await waitFor("more turns", async () => Number(await page.evaluate(() => threadView.querySelector(".lane").dataset.turns)) > before);
    await tapOrClick("#scroll-end");
    await waitFor("the end", () => page.evaluate(() => threadView.scrollHeight - threadView.scrollTop - threadView.clientHeight < 40), 3000);
    await waitFor("the pill to go", async () => !(await page.locator("#scroll-end").isVisible()), 2000);
    return `${before} then ${await page.evaluate(() => threadView.querySelector(".lane").dataset.turns)} turns`;
  });

  await step("app view: ⌘F finds text in the thread and opens the folds around it", async () => {
    if (touch) await page.evaluate(() => openFind());
    else await page.keyboard.press(`${(await page.evaluate(() => IS_MAC)) ? "Meta" : "Control"}+f`);
    await waitFor("the find bar", () => page.locator("#thread-find").isVisible());
    await page.fill("#thread-find input", "test result: ok");
    await waitFor("one match", async () => (await text("#thread-find .count")) === "1/1");
    check(await page.locator("#thread .tool[open] pre", { hasText: "test result: ok" }).isVisible(), "the match's tool output is still folded");
    const painted = await page.evaluate(() => (window.CSS?.highlights ? CSS.highlights.get("thread-find-active")?.size : 1));
    check(painted === 1, `${painted} active highlights`);
    await page.fill("#thread-find input", "answer 4");
    await waitFor("several matches", async () => /^1\/\d+$/.test(await text("#thread-find .count")));
    const count = await text("#thread-find .count");
    await page.press("#thread-find input", "Enter");
    check((await text("#thread-find .count")).startsWith("2/"), `Enter moved to ${await text("#thread-find .count")}`);
    await page.press("#thread-find input", "Escape");
    await waitFor("the bar to close", async () => !(await page.locator("#thread-find").isVisible()));
    return count;
  });

  await step("app view: appearance: grain, pressed buttons, chat width, diff colors, code size", async () => {
    check(/svg/.test(await page.evaluate(() => getComputedStyle(document.getElementById("expanded")).backgroundImage)), "the app view has no grain");
    const r = await rect("#toggle-panel");
    await page.mouse.move(r.left + r.width / 2, r.top + r.height / 2);
    await page.mouse.down();
    await sleep(200);
    const pressed = await page.evaluate(() => getComputedStyle(document.getElementById("toggle-panel")).scale);
    await page.mouse.move(0, 0);
    await page.mouse.up();
    check(pressed === "0.97", `a pressed button scales to ${pressed}`);
    await page.evaluate(() => toggleSettings());
    await tapOrClick('#settings-nav [data-page="general"]');
    await page.selectOption("#look-chat-width", "wide");
    await page.selectOption("#look-diff-colors", "blue-orange");
    await page.selectOption("#look-code-size", "16px");
    // The thread draws only the turns on screen, so a count and a code block are drawn here to read their styles.
    const looks = await page.evaluate(() => {
      const probe = Object.assign(document.createElement("div"), { innerHTML: '<span class="add">+1</span><div class="md"><pre><code>x</code></pre></div>' });
      threadView.append(probe);
      const read = [getComputedStyle(document.documentElement).getPropertyValue("--lane").trim(), getComputedStyle(probe.querySelector(".add")).color, getComputedStyle(probe.querySelector("code")).fontSize];
      probe.remove();
      return read;
    });
    check(looks.join("|") === "1152px|rgb(81, 162, 255)|16px", `looks: ${looks.join(" | ")}`);
    await page.selectOption("#look-chat-width", "full");
    check((await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--lane").trim())) === "100%", "Full doesn't widen the lane");
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    for (const name of ["chat-width", "diff-colors", "code-size"]) await page.selectOption(`#look-${name}`, "");
    await page.evaluate(() => toggleSettings());
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
    await waitFor("alpha", async () => /alpha/.test(await text("#chat-head .crumb")));
    return looks.join(" | ");
  });

  // Read in one go: the thread re-renders as it polls, and a style read from a replaced node is empty in WebKit.
  const styleOf = (selector, prop) => page.evaluate(([sel, p]) => [...document.querySelectorAll(sel)].filter((el) => el.checkVisibility()).map((el) => getComputedStyle(el)[p]).at(-1) ?? "", [selector, prop]);
  const toThread = async (name) => {
    await page.evaluate((n) => activate(tabs.find((t) => t.view.cwd.endsWith(`/${n}`))), name);
    await waitFor(name, async () => new RegExp(name).test(await text("#chat-head .crumb")));
  };

  await step("app view: the live row counts whole seconds, shows the thought over the tool, and Thinking between tools", async () => {
    await toThread("beta");
    await sendPrompt("think");
    await waitFor("the live thought", async () => (await text("#thread .live-tool .thought-line .label")) === "Weighing the options", 6000);
    check((await text("#thread .live-tool .line:not(.thought-line) .label")) === "Thinking", `the live row reads ${await text("#thread .live-tool")}`);
    check(/^\d+s$/.test(await text("#thread .work.live .ticker")), `the clock reads ${await text("#thread .work.live .ticker")}`);
    await answered("Done thinking.");
    await sendPrompt("build");
    await waitFor("the running command", async () => (await text("#thread .live-tool .label")) === "Running npm", 6000);
    check((await page.locator("#thread .live-tool").count()) === 1, "a Thinking row shows while a tool runs");
    await answered("Built.");
  });

  await step("app view: a steer continues the turn under its own fold, and pinned's own prompts say who sent them", async () => {
    await sendPrompt("steer");
    await answered("Checked both.");
    const steer = lastTurn(".user");
    check((await steer.locator(".steer").textContent()).trim() === "Steer", "the steer has no marker");
    check((await steer.locator(".bubble").textContent()).trim() === "also check the docs", `the steer reads ${await steer.locator(".bubble").textContent()}`);
    const folds = await page.locator("#thread .turn .work > summary .label").allTextContents();
    check(/^Continued( for |$)/.test(folds.at(-1)) && /^Worked( for |$)/.test(folds.at(-2)), `the folds read ${folds.slice(-2).join(" | ")}`);
    await sendPrompt("Continue from where you left off.");
    await answered("echo: Continue from where you left off.");
    check((await text("#thread .turn:last-of-type .sent-by")) === "Sent by Pinned", "pinned's own prompt isn't marked");
    check((await page.locator("#thread .turn:nth-last-of-type(2) .sent-by").count()) === 0, "a typed prompt is marked as pinned's");
    await page.evaluate(() => {
      active.view.connector = "sweep-workflow";
      rerender();
      threadView.scrollTop = 0;
    });
    const auto = gui("#thread .user.automation");
    await waitFor("the automation fold", () => auto.isVisible());
    check((await auto.locator(".who").textContent()) === "Automation" && (await auto.locator(".summary").textContent()) === "rich", `the fold reads ${await auto.textContent()}`);
    await press(auto.locator(".fold-head"));
    await waitFor("the automation's text", () => gui("#thread .user.automation .body").isVisible());
    check(await gui("#thread .user.automation [data-open-automation]").isVisible(), "no Open automation link");
    await page.evaluate(() => {
      delete active.view.connector;
      rerender();
      threadView.scrollTop = threadView.scrollHeight;
    });
  });

  await step("app view: notices are check rows, and a usage limit reads in the warning tone", async () => {
    await sendPrompt("notice");
    await waitFor("the notice", async () => (await page.locator("#thread .tool.info", { hasText: "Set model to Haiku 5.5" }).count()) > 0);
    const notice = gui("#thread .turn:last-of-type .tool.info");
    check((await notice.locator(".row .label").textContent()) === "Set model to Haiku 5.5", `the notice reads ${await notice.textContent()}`);
    check((await notice.locator(".message").textContent()) === "for this session", "the notice's rest is missing");
    check((await notice.locator(".row svg").first().innerHTML()).includes("M20 6 9 17l-5-5"), "the notice has no Check icon");
    await sendPrompt("limit");
    await waitFor("the limit row", async () => (await page.locator("#thread .error-row.limit").count()) > 0);
    const limit = gui("#thread .error-row.limit");
    const color = await styleOf("#thread .error-row.limit .label", "color");
    check(color === "rgb(254, 154, 0)", `the limit reads in ${color}`);
    check((await limit.locator(".label").textContent()).startsWith("You've hit your limit"), `the limit reads ${await limit.textContent()}`);
    await sendPrompt("oops");
    await waitFor("the API error", async () => (await page.locator("#thread .turn:last-of-type .error-row", { hasText: "API Error: 529" }).count()) === 1);
    const error = await styleOf("#thread .turn:last-of-type .error-row .label", "color");
    check(error === "rgb(251, 65, 74)", `an error reads in ${error}`);
  });

  await step("app view: wide tables cut cells until expanded and copy as Markdown, a shell block runs in the terminal, links show favicons", async () => {
    await sendPrompt("table");
    await answered("Open the app");
    const table = gui("#thread .turn:last-of-type .md-table");
    await waitFor("the clipped mark", async () => (await table.getAttribute("data-clipped")) === "true");
    check((await table.getAttribute("data-expanded")) === "true", "a table doesn't wrap by default");
    await press(table.locator("[data-table-expand]"));
    check((await table.getAttribute("data-expanded")) === "false", "the toggle didn't collapse the cells");
    const cell = await table.locator("td").nth(1).evaluate((el) => [el.scrollWidth > el.clientWidth, Math.round(el.getBoundingClientRect().height)]);
    check(cell[0] && cell[1] < 40, `a collapsed cell is cut ${cell[0]}, ${cell[1]}px tall`);
    await press(table.locator("[data-table-menu]"));
    await press(table.locator('[data-copy-table="markdown"]'));
    const copied = await clipboard();
    check(copied === null || copied.startsWith("| Name | Notes |\n| --- | --- |\n| alpha | a long note"), `the clipboard holds ${copied?.slice(0, 60)}`);
    const answer = gui("#thread .turn:last-of-type .answer");
    check((await answer.locator('a[href^="https://example.com"] img.favicon').getAttribute("src")) === "https://www.google.com/s2/favicons?domain=example.com&sz=32", "the docs link has no favicon");
    check((await answer.locator('a[href^="http://localhost"] img').count()) === 0 && (await answer.locator('a[href^="http://localhost"] svg').count()) === 1, "a local link asks Google for its favicon");
    await press(answer.locator("[data-run-code]"));
    await answered("echo: ! echo ran-from-thread");
    check((await text("#thread .turn:last-of-type .bubble")) === "! echo ran-from-thread", `the run reads ${await text("#thread .turn:last-of-type .bubble")}`);
    check(await page.evaluate(() => document.body.dataset.drawer === "open"), "the terminal drawer didn't open");
    await page.evaluate(() => toggleDrawer(false));
    check((await page.locator("#thread .codeblock .lang", { hasText: "rust" }).count()) === 0 || (await page.locator("#thread .codeblock:has(.lang:text-is('rust')) [data-run-code]").count()) === 0, "a Rust block offers to run");
    await page.evaluate(() => wantLang("cobol"));
    await waitFor("a rare language's grammar", () => page.evaluate(() => shikiLangs.has("cobol")), 8000);
  });

  await step("app view: typography sets the interface, prompt, code and terminal fonts, and the lane keeps 48px gutters down to 640px", async () => {
    await page.evaluate(() => toggleSettings());
    await tapOrClick('#settings-nav [data-page="general"]');
    const resetTypography = () =>
      page.evaluate(() => {
        for (const name of ["ui-size", "prompt-size", "terminal-size", "sans-font", "mono-font", "advanced", "smoothing", "word-wrap"]) save(`look-${name}`, "");
        applyLooks();
        if (settingsOpen()) toggleSettings();
      });
    try {
      check((await page.locator("#look-terminal-size").count()) === 0, "the terminal row shows without Advanced");
      check((await text('label[for="look-mono-font"]')) === "Monospace font", `the simple code row reads ${await text('label[for="look-mono-font"]')}`);
      await page.selectOption("#look-ui-size", "18px");
      const zoom = await page.evaluate(() => getComputedStyle(document.getElementById("chat")).zoom);
      check(zoom === "1.125", `the interface zooms ${zoom}`);
      await page.fill("#look-sans-font", "Georgia");
      await page.press("#look-sans-font", "Enter");
      await page.fill("#look-mono-font", "Courier New");
      await page.press("#look-mono-font", "Enter");
      const fonts = await page.evaluate(() => [getComputedStyle(document.body).fontFamily, getComputedStyle(document.querySelector("#thread .md pre code")).fontFamily, active.term.options.fontFamily, active.term.options.fontSize].map((f) => String(f).split(",")[0].replaceAll('"', "")));
      check(fonts.join("|") === "Georgia|Courier New|Courier New|12", `fonts: ${fonts.join(" | ")}`);
      await page.locator("#look-advanced").setChecked(true);
      await waitFor("the advanced rows", () => page.locator("#look-terminal-size").isVisible());
      await page.selectOption("#look-terminal-size", "15px");
      await page.selectOption("#look-prompt-size", "18px");
      const sizes = await page.evaluate(() => [active.term.options.fontSize, getComputedStyle(prompt).fontSize, active.term.options.fontFamily.split(",")[0]]);
      check(sizes.join("|") === "15|18px|ui-monospace", `advanced sizes: ${sizes.join(" | ")}`);
      if (await page.evaluate(() => IS_MAC)) {
        check((await page.evaluate(() => getComputedStyle(document.body).webkitFontSmoothing)) === "antialiased" || engine !== "chromium", "text isn't antialiased by default");
        await page.locator("#look-smoothing").setChecked(false);
        check((await page.evaluate(() => document.documentElement.dataset.smoothing)) === "false", "smoothing didn't turn off");
        await page.locator("#look-smoothing").setChecked(true);
      }
      await page.locator("#look-word-wrap").setChecked(false);
      check((await page.evaluate(() => document.querySelector("#thread .codeblock:has([data-run-code])")?.dataset.wrap)) === "false", "Word wrap off still wraps code");
      await page.locator("#look-word-wrap").setChecked(true);
      for (const id of ["#look-terminal-size", "#look-prompt-size", "#look-ui-size"]) await page.selectOption(id, "");
      for (const id of ["#look-sans-font", "#look-mono-font"]) {
        await page.fill(id, "");
        await page.press(id, "Enter");
      }
      await page.locator("#look-advanced").setChecked(false);
      const reset = await page.evaluate(() => [getComputedStyle(document.getElementById("chat")).zoom, active.term.options.fontSize, getComputedStyle(document.body).fontFamily.split(",")[0]]);
      check(reset.join("|") === "1|12|-apple-system", `after a reset: ${reset.join(" | ")}`);
    } finally {
      await resetTypography();
    }
    if (!touch) {
      const gutter = async (width) => {
        await page.setViewportSize({ width, height: 800 });
        await sleep(150);
        return page.evaluate(() => getComputedStyle(threadView).paddingLeft);
      };
      const gutters = [await gutter(700), await gutter(600)];
      await page.setViewportSize(profile.viewport);
      check(gutters.join("|") === "48px|12px", `gutters at 700 and 600px: ${gutters.join(" | ")}`);
    }
  });


  // T3 Code's themes: light and dark, System following prefers-color-scheme, the palettes, and
  // the keys, ⌘K items, Settings section and editor that pick them.
  const themeKey = async (key, shift = false) => {
    const mod = (await page.evaluate(() => IS_MAC)) ? "Meta" : "Control";
    await page.keyboard.press(`${mod}+Alt+${shift ? "Shift+" : ""}${key}`);
  };
  const appearance = () => page.evaluate(() => [document.documentElement.dataset.appearance, document.documentElement.dataset.themeId ?? ""].join("|"));
  const resetTheme = () =>
    page.evaluate(() => {
      for (const key of [THEME_KEY, THEME_MODE_KEY, THEME_HALVES_KEY, CUSTOM_THEMES_KEY]) localStorage.removeItem(key);
      customThemes = [];
      if (themeEditor) closeThemeEditor();
      applyTheme();
    });
  /** Visible text whose contrast with what is painted behind it is under 2:1, compositing translucent
   * fills and opacity up the tree onto the canvas. T3's own faint labels (muted at 60-70%, the
   * warning orange) sit between 2 and 3 in its light theme; text the theme missed sits near 1.
   * Syntax tokens keep pierre-light's own colors, as T3's do, so code is left out. */
  const unreadable = (scope) =>
    page.evaluate((scope) => {
      const ctx = Object.assign(document.createElement("canvas"), { width: 1, height: 1 }).getContext("2d", { willReadFrequently: true });
      const rgba = (css) => {
        ctx.clearRect(0, 0, 1, 1);
        ctx.fillStyle = "#000";
        ctx.fillStyle = css;
        ctx.fillRect(0, 0, 1, 1);
        const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data;
        return [r, g, b, a / 255];
      };
      const over = (top, under) => top.slice(0, 3).map((c, i) => c * top[3] + under[i] * (1 - top[3])).concat(1);
      const lum = ([r, g, b]) => [r, g, b].map((c) => ((c /= 255) <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)).reduce((s, c, i) => s + c * [0.2126, 0.7152, 0.0722][i], 0);
      const ratio = (x, y) => (Math.max(lum(x), lum(y)) + 0.05) / (Math.min(lum(x), lum(y)) + 0.05);
      const canvas = rgba(getComputedStyle(document.documentElement).getPropertyValue("--g-canvas"));
      const bad = [];
      for (const el of document.querySelectorAll(`${scope} *`)) {
        if (![...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim())) continue;
        if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true }) || el.closest("[aria-hidden=true], [disabled], [aria-disabled=true], .xterm, iframe, .wireframe, pre code, .dl .code, .fl .code")) continue;
        const r = el.getBoundingClientRect();
        if (!r.width || !r.height || r.bottom < 0 || r.top > innerHeight || r.right < 0 || r.left > innerWidth) continue;
        const fills = [];
        let opacity = 1;
        for (let p = el; p; p = p.parentElement) {
          const s = getComputedStyle(p);
          opacity *= Number(s.opacity);
          const fill = rgba(s.backgroundColor);
          if (fill[3] > 0) fills.push(fill);
          if (fill[3] >= 1) break;
        }
        const behind = fills.reverse().reduce((under, fill) => over(fill, under), canvas);
        const text = rgba(getComputedStyle(el).color);
        const shown = over([...text.slice(0, 3), text[3] * opacity], behind);
        const c = ratio(shown, behind);
        if (c < 2) bad.push(`${el.id || el.className || el.tagName} "${el.textContent.trim().slice(0, 24)}" ${c.toFixed(2)}`);
      }
      return [...new Set(bad)];
    }, scope);

  await step("theme: no literal colors left in the app view's css, by scripts/theme-tokens.mjs --check", async () => {
    const run = require("child_process").spawnSync(process.execPath, [path.join(__dirname, "../theme-tokens.mjs"), "--check"], { encoding: "utf8" });
    check(run.status === 0, (run.stdout + run.stderr).trim().split("\n").slice(0, 3).join("; "));
  });

  await step("app view: System follows prefers-color-scheme into T3's light and dark tokens, and the drawer's terminal and code follow", async () => {
    await resetTheme();
    try {
      check((await appearance()) === "dark|", `under a dark system the app is ${await appearance()}`);
      await page.emulateMedia({ colorScheme: "light" });
      await waitFor("the light appearance", async () => (await appearance()) === "light|");
      const look = await page.evaluate(() => {
        const css = (sel, p) => getComputedStyle(document.querySelector(sel))[p];
        return [css("#sidebar", "color"), getComputedStyle(document.documentElement).getPropertyValue("--g-canvas").trim(), active.term.options.theme.foreground, codeTheme(), document.querySelector('meta[name="theme-color"]').content];
      });
      check(look.join("|") === "rgb(39, 39, 42)|rgb(252 252 252)|#27272a|pierre-light|rgb(252 252 252)", `light: ${look.join(" | ")}`);
      await page.emulateMedia({ colorScheme: "dark" });
      await waitFor("the dark appearance", async () => (await appearance()) === "dark|");
      const dark = await page.evaluate(() => [getComputedStyle(document.querySelector("#sidebar")).color, active.term.options.theme.foreground, codeTheme()]);
      check(dark.join("|") === "rgb(241, 243, 247)|#f5f5f5|pierre-dark", `dark: ${dark.join(" | ")}`);
      await page.evaluate(() => setMode("terminal"));
      check((await page.evaluate(() => JSON.stringify(active.term.options.theme))) === '{"background":"#00000000"}', "the terminal view's xterm took the app view's colors");
      await page.evaluate(() => setMode("gui"));
    } finally {
      await page.emulateMedia({ colorScheme: "dark" });
      await resetTheme();
    }
  });

  await step("app view: ⌘⌥⇧A cycles System, Light, Dark with a toast, and ⌘⌥A opens Change theme, whose pick paints the palette", async () => {
    await resetTheme();
    try {
      const seen = [];
      for (let i = 0; i < 3; i++) {
        await themeKey("A", true);
        seen.push(`${await page.evaluate(() => themeState().mode)}:${await appearance()}:${await text("#panel-note span")}`);
      }
      check(seen.join(" ") === "light:light|:Appearance: Light dark:dark|:Appearance: Dark system:dark|:Appearance: System", `cycle: ${seen.join(" ")}`);
      await themeKey("A");
      await waitFor("the Change theme list", async () => (await page.locator("#palette .group .label").first().textContent()) === "Change theme");
      const names = await page.locator("#palette .item .name").allTextContents();
      check(names.join(",") === "T3 Code,T3 Chat,Grove,Ocean,Ember,Iris", `themes: ${names.join(", ")}`);
      check((await page.locator("#palette .item").first().locator(".current").textContent()) === "Current", "T3 Code isn't marked current");
      await press(page.locator("#palette .item", { hasText: "Grove" }));
      await waitFor("Grove", async () => (await appearance()) === "dark|grove");
      const canvas = await page.evaluate(() => [colorHex(getComputedStyle(document.querySelector("#composer .box")).backgroundColor), colorHex(colorsFor(themeById("grove"), "dark").surface)]);
      check(canvas[0] === `${canvas[1]}cc`, `the composer is ${canvas[0]}, Grove's dark surface ${canvas[1]}`);
      await themeKey("A", true);
      check((await appearance()) === "light|grove", `Grove in light is ${await appearance()}`);
    } finally {
      await page.evaluate(() => closePalette());
      await resetTheme();
    }
  });

  await step("app view: ⌘K has Change theme, Change appearance and Toggle theme editor, as T3's palette does", async () => {
    await resetTheme();
    try {
      await page.evaluate(() => openPalette());
      await page.fill("#palette input", "theme");
      const found = await page.locator("#palette .item .name").allTextContents();
      check(["Change theme", "Toggle theme editor"].every((n) => found.includes(n)), `"theme" finds ${found.join(", ")}`);
      await page.fill("#palette input", "appearance");
      await press(page.locator("#palette .item", { hasText: "Change appearance" }));
      const modes = await page.locator("#palette .item .name").allTextContents();
      check(modes.join(",") === "System,Light,Dark", `appearance: ${modes.join(", ")}`);
      check((await page.locator("#palette .item", { hasText: "System" }).locator(".current").count()) === 1, "System isn't current");
      await press(page.locator("#palette .item", { hasText: "Light" }));
      check((await appearance()) === "light|", `after Light: ${await appearance()}`);
      await page.evaluate(() => openPalette());
      await page.fill("#palette input", "toggle theme editor");
      await press(page.locator("#palette .item", { hasText: "Toggle theme editor" }));
      await waitFor("the editor", () => page.locator("#theme-editor").isVisible());
    } finally {
      await page.evaluate(() => closePalette());
      await resetTheme();
    }
  });

  await step("app view: Settings' Color scheme tiles and theme cards pick the appearance, a palette, and one half", async () => {
    await resetTheme();
    await page.evaluate(() => toggleSettings());
    await tapOrClick('#settings-nav [data-page="general"]');
    try {
      const tiles = await page.locator("#theme-section .mode-tile .label").allTextContents();
      check(tiles.join(",") === "System,Light,Dark", `tiles: ${tiles.join(", ")}`);
      check((await page.locator('#theme-section [data-theme-mode="system"]').getAttribute("aria-pressed")) === "true", "System isn't pressed");
      await press(page.locator('#theme-section [data-theme-mode="light"]'));
      check((await appearance()) === "light|", `the Light tile gives ${await appearance()}`);
      const settingsText = await page.evaluate(() => getComputedStyle(document.querySelector("#settings-page h2")).color);
      check(settingsText === "rgb(39, 39, 42)", `Settings' heading reads in ${settingsText}`);
      const cards = await page.locator("#theme-section .theme-card .name").allTextContents();
      check(cards.join(",") === "T3 Code,T3 Chat,Grove,Ocean,Ember,Iris", `cards: ${cards.join(", ")}`);
      await press(page.locator('#theme-section .theme-card[data-theme-card="ocean"] .name'));
      check((await appearance()) === "light|ocean", `Ocean gives ${await appearance()}`);
      await press(page.locator('#theme-section .theme-card[data-theme-card="ember"] [data-theme-half="dark"]'));
      const halves = await page.evaluate(() => JSON.stringify(themeState().halves));
      check(halves === '{"dark":"ember"}', `halves: ${halves}`);
      await press(page.locator('#theme-section [data-theme-mode="dark"]'));
      check((await appearance()) === "dark|ember", `dark with an Ember half is ${await appearance()}`);
      check((await page.locator('#theme-section .theme-card[data-theme-card="ember"] .pick.picked').count()) === 1, "Ember's dark circle isn't picked");
      const bad = await unreadable("#settings");
      check(!bad.length, `unreadable in Settings under Ember: ${bad.slice(0, 4).join("; ")}`);
    } finally {
      await resetTheme();
      await page.evaluate(() => settingsOpen() && toggleSettings());
    }
  });

  await step("app view: ⌘⌥⇧T's theme editor derives a palette from Background and Accent live, creates it, edits, exports and removes it", async () => {
    await resetTheme();
    try {
      await themeKey("T", true);
      await waitFor("the editor", () => page.locator("#theme-editor").isVisible());
      check((await text("#theme-editor .te-title b")) === "Create theme", `the editor is ${await text("#theme-editor .te-title b")}`);
      const simple = await page.locator("#theme-editor .te-label").allTextContents();
      check(simple.join(",") === "Background,Accent", `simple fields: ${simple.join(", ")}`);
      await page.fill('#theme-editor .te-value[data-role="canvas"]', "#102030");
      await page.fill('#theme-editor .te-value[data-role="accent"]', "#e0569a");
      const preview = await page.evaluate(() => [document.documentElement.dataset.themeId, document.documentElement.dataset.appearance, colorHex(getComputedStyle(document.documentElement).getPropertyValue("--g-canvas").trim()), colorHex(getComputedStyle(document.documentElement).getPropertyValue("--g-primary").trim()) !== "#346bf1"]);
      check(preview.join("|") === "__preview|dark|#102030|true", `the preview: ${preview.join(" | ")}`);
      check(await page.locator('#theme-editor [data-te="save"]').isDisabled(), "an unnamed theme can be saved");
      await page.fill("#theme-name", "Harbor");
      await press(page.locator('#theme-editor [data-te="save"]'));
      await waitFor("the saved theme", async () => (await appearance()) === "dark|harbor");
      check((await page.locator("#theme-editor").count()) === 0, "the editor stayed open");
      check(/Harbor created/.test(await text("#panel-note span")), `the toast reads ${await text("#panel-note span")}`);
      await page.evaluate(() => toggleSettings());
      await tapOrClick('#settings-nav [data-page="general"]');
      const card = page.locator('#theme-section .theme-card[data-theme-card="harbor"]');
      check((await card.locator("[data-theme-action]").count()) === 4, "Harbor's card lacks Duplicate, Edit, Export, Remove");
      await press(card.locator('[data-theme-action="edit"]'));
      await waitFor("the edit panel", async () => (await text("#theme-editor .te-title b")) === "Edit theme");
      await page.locator('#theme-editor [data-te="advanced"]').setChecked(true);
      check((await page.locator("#theme-editor .te-group h4").allTextContents()).join(",") === "Foundation,Brand & content,Search,Context,Status", "advanced lacks T3's families");
      await press(page.locator('#theme-editor [data-te="inspect"]'));
      check((await text("#theme-editor .te-title span")) === "Select an element · Esc to cancel", `inspecting reads ${await text("#theme-editor .te-title span")}`);
      const bubble = gui("#thread .turn .bubble");
      const edge = { position: { x: 14, y: 3 } };
      if (!touch) {
        await bubble.hover(edge);
        await waitFor("the inspector's label", async () => (await page.locator("#theme-inspector-hover:not([hidden]) .label").textContent().catch(() => "")) === "Message surface");
      }
      await (touch ? bubble.tap(edge) : bubble.click(edge));
      await waitFor("the picked family", async () => (await page.locator("#theme-editor .te-field.selected").getAttribute("data-theme-color-role").catch(() => "")) === "messageSurface");
      check((await page.evaluate(() => document.activeElement?.dataset.role)) === "messageSurface", "the picked field isn't focused");
      await page.fill("#theme-editor .te-filter", "sidebar");
      check((await page.locator("#theme-editor .te-label").allTextContents()).join(",") === "Sidebar background,Sidebar controls,Sidebar selection", "the filter doesn't narrow to the sidebar");
      await page.fill('#theme-editor .te-value[data-role="sidebar"]', "#000000");
      await press(page.locator('#theme-editor [data-te="save"]'));
      await waitFor("the saved edit", async () => (await page.evaluate(() => colorHex(customThemes[0].colors.sidebar))) === "#000000");
      check((await appearance()) === "dark|harbor", `after the edit: ${await appearance()}`);
      const file = await page.evaluate(() => JSON.parse(themeFile(customThemes[0])));
      check(file.version === 1 && file.name === "Harbor" && file.appearance === "dark" && /^oklch\(/.test(file.colors.canvas), `the export: ${JSON.stringify(file).slice(0, 80)}`);
      await press(card.locator('[data-theme-action="remove"]'));
      await press(page.locator("#shell-dialog [data-dialog-remove]"));
      await waitFor("the removal", async () => (await page.locator('#theme-section .theme-card[data-theme-card="harbor"]').count()) === 0);
      check((await appearance()) === "dark|", `after removing it: ${await appearance()}`);
    } finally {
      await resetTheme();
      await page.evaluate(() => settingsOpen() && toggleSettings());
    }
  });

  await step("app view: in light mode the sidebar, thread, composer, diff, PR, browser, settings and ⌘K have no unreadable text", async () => {
    await resetTheme();
    await page.emulateMedia({ colorScheme: "light" });
    const shot = (name) => page.screenshot({ path: path.join(shots, `light-${name}.png`) });
    const bad = [];
    const panelWas = await page.evaluate(() => ({ panel: JSON.stringify(panelOf(active)), diff: threadOf(active).diff ? JSON.stringify(threadOf(active).diff) : null, saved: [`gui-panel:${threadKey(active)}`, `gui-diff:${threadKey(active)}`].map((k) => [k, localStorage.getItem(k)]) }));
    const audit = async (name, scope) => {
      await sleep(400);
      await shot(name);
      bad.push(...(await unreadable(scope)).map((b) => `${name}: ${b}`));
    };
    try {
      await waitFor("the light appearance", async () => (await appearance()) === "light|");
      if (touch) await page.evaluate(() => toggleSidebar(true));
      await audit("sidebar", "#sidebar");
      if (touch) await page.evaluate(() => toggleSidebar(false));
      await audit("thread", "#chat");
      await page.evaluate(() => openPanel("diff"));
      await audit("diff", "#panel");
      if (await page.evaluate(() => !!shownPr(active))) {
        await page.evaluate(() => openSurface("pr"));
        await audit("pr", "#panel");
      }
      await page.evaluate(() => openSurface("browser"));
      await audit("browser", "#panel");
      await page.evaluate(() => closePanel());
      await page.evaluate(() => openPalette());
      await audit("palette", "#palette");
      await page.evaluate(() => closePalette());
      await page.evaluate(() => toggleSettings());
      await tapOrClick('#settings-nav [data-page="general"]');
      await audit("settings", "#settings");
      await page.evaluate(() => toggleSettings());
      check(!bad.length, bad.slice(0, 5).join("; "));
    } finally {
      await page.evaluate(() => settingsOpen() && toggleSettings());
      await page.evaluate((was) => {
        panelStates.set(active.key, JSON.parse(was.panel));
        threadOf(active).diff = was.diff ? JSON.parse(was.diff) : undefined;
        for (const [key, value] of was.saved) value === null ? localStorage.removeItem(key) : localStorage.setItem(key, value);
        renderPanel();
      }, panelWas);
      await page.emulateMedia({ colorScheme: "dark" });
      await resetTheme();
    }
  });

  await step("app view: changed files and mentions carry T3 Code's file-type icons", async () => {
    await sendPrompt("icons");
    await answered("Wrote two files.");
    await waitFor("the changed-files card", async () => (await page.locator("#thread .turn:last-of-type .changes").count()) > 0, 10000);
    const card = lastTurn(".changes");
    await press(card.locator("summary .show"));
    await waitFor("the tree", async () => (await card.locator("li[data-file]").count()) === 2);
    const icons = await card.locator("li[data-file]").evaluateAll((rows) => rows.map((r) => `${r.dataset.file}=${r.querySelector("svg.pierre use")?.getAttribute("href")}`));
    check(icons.join(" ") === "web/app.ts=#file-tree-builtin-typescript Cargo.toml=#t3-file-icon-toml", `icons: ${icons.join(" ")}`);
    const drawn = await card.locator('li[data-file="web/app.ts"] svg.pierre').evaluate((el) => [getComputedStyle(el).color, Math.round(el.getBoundingClientRect().width)]);
    check(drawn.join("|") === "rgb(105, 177, 255)|14", `the TypeScript icon draws as ${drawn.join(" ")}`);
    check(await page.evaluate(() => markdown("@web/app.ts", promptMarked).includes("#file-tree-builtin-typescript")), "a mention chip has no file-type icon");
  });

  await step("app view: an Agent row opens its subagent's own thread, and back", async () => {
    await sendPrompt("agent");
    await answered("The agent reported back.");
    await press(lastTurn(".work > summary"));
    const open = gui('#thread .turn:last-of-type [data-subagent]');
    check((await open.textContent()) === "Open subagent", `the link reads ${await open.textContent()}`);
    await press(open);
    await waitFor("the subagent's answer", async () => (await page.locator("#thread .assistant", { hasText: "The subagent found it in src/parse.rs." }).count()) === 1);
    check((await text("#thread .bubble")) === "Find where the parser drops the last line.", `the subagent's prompt reads ${await text("#thread .bubble")}`);
    check((await text("#chat-head .crumb .title:last-of-type")) === "Look for the bug", `the header reads ${await text("#chat-head .crumb")}`);
    check(!(await page.locator("#composer").isVisible()), "the composer shows over a subagent");
    await press(gui("#chat-head [data-subagent-back]"));
    await waitFor("the thread again", async () => (await page.locator("#thread .assistant", { hasText: "The agent reported back." }).count()) === 1);
    check(await page.locator("#composer").isVisible(), "the composer didn't come back");
  });

  await step("app view: Save to workspace writes the plan into the thread's folder", async () => {
    await sendPrompt("plan");
    await answered("Plan approved.");
    const card = lastTurn(".plan-card");
    await press(card.locator("[data-plan-menu]"));
    await press(card.locator("[data-plan-save]"));
    const field = page.locator(".tl-dialog input");
    await waitFor("the save dialog", () => field.isVisible());
    check((await field.inputValue()) === "ship-the-fix.md", `the path starts as ${await field.inputValue()}`);
    await field.fill("plans/ship.md");
    await press(page.locator('.tl-dialog button[value="save"]'));
    const file = path.join(HOME, "work", "beta", "plans", "ship.md");
    await waitFor("the saved plan", async () => fs.existsSync(file), 5000);
    check(fs.readFileSync(file, "utf8").startsWith("# Ship the fix"), "the saved file isn't the plan");
    await waitFor("the saved note", async () => /Plan saved to workspace: plans\/ship\.md/.test(await text("#composer-note")));
  });

  await step("app view: Edit from here rewinds claude to before a prompt, puts its files back, and returns it to the composer", async () => {
    const file = path.join(HOME, "work", "beta", "revert.txt");
    await sendPrompt("revertme");
    await answered("Wrote revert.txt.");
    check(fs.existsSync(file), "the fake didn't write revert.txt");
    await waitFor("the turn's changes", async () => (await page.locator("#thread .turn:last-of-type .changes").count()) > 0, 10000);
    const before = await page.evaluate(() => threadOf(active).session);
    const user = lastTurn(".user");
    if (!touch) await user.hover();
    await press(user.locator("[data-rewind]"));
    await waitFor("the dialog", () => page.locator(".tl-dialog").isVisible());
    check((await text(".tl-dialog h2")) === "Edit from here?", `the dialog reads ${await text(".tl-dialog h2")}`);
    await press(page.locator('.tl-dialog button[value="files"]'));
    await waitFor("the rewound session", async () => (await page.evaluate(() => threadOf(active).session)) !== before, 10000);
    await waitFor("the composer to hold the prompt", async () => (await page.inputValue("#prompt")) === "revertme");
    check(!fs.existsSync(file), "revert.txt is still there");
    await waitFor("the thread without the prompt", async () => (await page.locator("#thread .assistant", { hasText: "Wrote revert.txt." }).count()) === 0 && (await page.locator("#thread .assistant", { hasText: "Plan approved." }).count()) > 0, 10000);
    await waitFor("claude resumed", async () => /resumed [0-9a-f-]{36}/.test(await termText()), 8000);
    await page.fill("#prompt", "");
  });

  await step("app view: Fork from this response opens a thread that carries on from that answer", async () => {
    const tabs = await tabCount();
    const answer = lastTurn(".answer");
    if (!touch) await answer.hover();
    await press(answer.locator("[data-fork]"));
    await waitFor("the forked thread", async () => (await tabCount()) === tabs + 1 && (await page.evaluate(() => threadOf(active).items.length)) > 0, 10000);
    await waitFor("the fork's last answer", async () => (await page.locator("#thread .turn:last-of-type .assistant", { hasText: "Plan approved." }).count()) === 1, 10000);
    check((await page.evaluate(() => active.view.cwd)).endsWith("/beta"), "the fork isn't in beta's folder");
    await waitFor("claude resumed in the fork", async () => /resumed [0-9a-f-]{36}/.test(await termText()), 8000);
    await page.evaluate(() => closeTab(active));
    await waitFor("the fork to close", async () => (await tabCount()) === tabs);
    await toThread("beta");
  });

  await step("app view: a long thread draws only the turns near the viewport, and the minimap jumps between prompts", async () => {
    await sendPrompt("many");
    await answered("answer 45");
    const drawn = await page.evaluate(() => [threadView.querySelector(".lane").dataset.turns, threadView.querySelectorAll(".lane > .turn").length]);
    check(Number(drawn[0]) >= 40 && drawn[1] < 20, `drew ${drawn[1]} of ${drawn[0]} turns`);
    const [firstId, top] = await page.evaluate(() => {
      const shown = turnsOf(threadOf(active).items).slice(-Number(threadView.querySelector(".lane").dataset.turns));
      return [shown[0].id, shown.find((x) => x.user).user.text.split("\n")[0]];
    });
    await page.evaluate(() => (threadView.scrollTop = 0));
    await waitFor("the top turns", () => page.evaluate((id) => threadView.querySelector(".lane > .turn")?.dataset.turnId === id, firstId), 3000);
    await page.evaluate(() => (threadView.scrollTop = threadView.scrollHeight));
    await waitFor("the last turn", async () => (await page.locator("#thread .turn .bubble", { hasText: /^question 45$/ }).count()) === 1, 3000);
    if (touch) return `drew ${drawn[1]} of ${drawn[0]}; the minimap needs a fine pointer`;
    const ticks = await page.locator("#minimap .tick").count();
    check(ticks === Number(drawn[0]) && (await page.locator("#minimap").isVisible()), `the minimap shows ${ticks} ticks`);
    const r = await rect("#minimap .rail");
    await page.mouse.move(r.left + 6, r.top + 1);
    await waitFor("the preview", () => page.locator("#minimap .preview").isVisible());
    check((await text("#minimap .preview b")) === top, `the preview reads ${await text("#minimap .preview b")}, not ${top}`);
    await page.mouse.click(r.left + 6, r.top + 1);
    await waitFor("the first prompt on top", () => page.evaluate((top) => {
      const el = [...document.querySelectorAll("#thread .turn")].find((t) => t.querySelector(".bubble")?.textContent.trim() === top);
      return !!el && Math.abs(el.getBoundingClientRect().top - threadView.getBoundingClientRect().top - 24) < 4;
    }, top), 3000);
    await page.mouse.move(0, 400);
    await page.evaluate(() => (threadView.scrollTop = threadView.scrollHeight));
    return `drew ${drawn[1]} of ${drawn[0]}, ${ticks} ticks`;
  });

  await step("app view: resting on a PR link previews the pull request", async () => {
    await sendPrompt("prlink");
    await answered("Opened the pull request");
    if (!touch) {
      await gui('#thread .turn:last-of-type a[data-pr]').hover();
      await waitFor("the preview card", () => page.locator("#pr-card").isVisible(), 8000);
      const card = await page.evaluate(() => [...document.querySelectorAll("#pr-card .head > *, #pr-card .title, #pr-card .who > *")].map((el) => el.textContent.trim()).filter(Boolean).join(" | "));
      check(/^example\/alpha \| #42 \| · \| Open \| Add notes \| sweeper \| · \| opened .+ ago$/.test(card), `the card reads ${card}`);
      await page.mouse.move(5, 5);
      await waitFor("the card to close", async () => !(await page.locator("#pr-card").isVisible()), 2000);
    }
    // The steps after this block expect alpha.
    await toThread("alpha");
    if (touch) return "a touch screen has no hover";
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
    await press(gui("#panel-body .file > header .change"));
    const added = await page.locator("#panel-body .dl.add").count();
    check(added >= 2, `${added} added lines in the diff`);
    await press(gui('#panel-body [data-diff="split"]'));
    check((await page.locator("#panel-body .dl.split").count()) > 0, "split view shows no side-by-side rows");
    await press(gui('#panel-body [data-diff="unified"]'));
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    if (touch) await tapOrClick("#toggle-panel");
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
    check(/#41/.test(await text('#panel-tabs [data-panel="pr"].active')), `the PR tab reads ${await text('#panel-tabs [data-panel="pr"].active')} after clicking #41`);
    if (!touch) {
      const tabsBefore = await page.locator("#thread-list .thread-row").count();
      for (let n = await page.locator("#panel-tabs .ptab").count(); n > 0; n--) await page.keyboard.press("Meta+w");
      await waitFor("⌘W to close the panel", async () => !(await page.locator("#panel").isVisible()));
      check((await page.locator("#thread-list .thread-row").count()) === tabsBefore, "⌘W closed a thread while the panel was open");
      await tapOrClick("#toggle-panel");
    }
    if (touch) {
      await tapOrClick("#toggle-panel");
      await tapOrClick("#toggle-panel");
    }
    return shown;
  });

  await step("app view: the browser panel loads a URL", async () => {
    await press(gui("#panel-add"));
    await press(gui('#panel-menu button:has-text("Browser")'));
    await page.fill("#panel-body .url-bar input", `${HUB}/manifest.webmanifest`);
    await page.press("#panel-body .url-bar input", "Enter");
    await waitFor("the frame", async () => (await page.locator("#panel-body iframe").getAttribute("src"))?.endsWith("manifest.webmanifest"));
    const frame = page.frameLocator("#panel-body iframe");
    await waitFor("the page in the frame", async () => /pinned/.test((await frame.locator("body").textContent().catch(() => "")) ?? ""));
    await tapOrClick("#toggle-panel");
    check(!(await page.locator("#panel").isVisible()), "the panel didn't close");
    await sleep(200);
  });

  const openTab = (kind) => gui(`#panel-tabs .ptab.active[data-panel="${kind}"]`);
  const fromPlus = async (label) => {
    await press(gui("#panel-add"));
    await press(gui(`#panel-menu button:has-text("${label}")`));
  };
  const readLog = (name) => (fs.existsSync(path.join(HOME, name)) ? fs.readFileSync(path.join(HOME, name), "utf8") : "");
  const threadCount = () => page.locator("#thread-list .thread-row").count();

  await step("app view: panel tabs open from + and the launcher, close, and stay with their thread", async () => {
    await tapOrClick("#toggle-panel");
    await waitFor("the panel", () => page.locator("#panel").isVisible());
    check((await page.locator("#panel-tabs #toggle-panel").count()) === 1, "with the panel open, the layout controls stayed in the chat header");
    for (let n = await page.locator("#panel-tabs .ptab").count(); n > 0; n--) await press(gui("#panel-tabs .ptab .lead"));
    await waitFor("the last tab's × to close the panel", async () => !(await page.locator("#panel").isVisible()));
    check((await page.locator("#chat-head #toggle-panel").count()) === 1, "with the panel closed, the layout controls didn't return to the chat header");
    await tapOrClick("#toggle-panel");
    await waitFor("the launcher", async () => /Open a surface/.test(await text("#panel-body")));
    const rows = await page.locator("#panel-body .launcher button").allTextContents();
    check(rows.join("|") === "BrowserB|TerminalT|FilesF|DiffD|Pull requestP|Linked pull requestsL|DeviceM", `launcher rows: ${rows.join(" | ")}`);
    if (touch) await press(gui('#panel-body [data-launch="3"]'));
    else await page.keyboard.press("d");
    await waitFor("the Diff tab", () => openTab("diff").isVisible());
    if (touch) await tapOrClick("#panel-add");
    else await page.keyboard.press("Meta+t");
    await waitFor("the + menu", () => page.locator("#panel-menu").isVisible());
    check(!(await page.evaluate(() => pickerOpen())), "⌘T opened the project picker while the panel was open");
    const items = await page.locator("#panel-menu button .label").allTextContents();
    check(items.join("|") === "Browser|Terminal|Files|Diff|Pull request|Linked pull requests|Device", `+ menu: ${items.join(" | ")}`);
    await press(gui('#panel-menu button:has-text("Pull request")'));
    await waitFor("a PR tab", () => openTab("pr").isVisible());
    check((await page.locator("#panel-tabs .ptab").count()) === 2, `${await page.locator("#panel-tabs .ptab").count()} tabs, expected Diff and the PR`);
    if (!touch) {
      await page.click('#panel-tabs .ptab[data-panel="diff"]', { button: "right" });
      await press(gui('#panel-menu button:has-text("Close others")'));
      await waitFor("Close others", async () => (await page.locator("#panel-tabs .ptab").count()) === 1);
    }
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/beta"))));
    await waitFor("beta's own, closed panel", async () => !(await page.locator("#panel").isVisible()));
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
    await waitFor("alpha's panel back on its tab", () => openTab(touch ? "pr" : "diff").isVisible());
    if (!touch) {
      await tapOrClick("#panel-expand");
      check((await page.evaluate(() => document.body.dataset.panelMax)) === "true", "maximize didn't maximize");
      check((await page.getAttribute("#panel-expand", "title")) === "Restore panel size", `maximized, the button says ${await page.getAttribute("#panel-expand", "title")}`);
      check((await page.locator("#panel").evaluate((el) => getComputedStyle(el).borderLeftWidth)) === "0px", "the maximized panel kept its left border");
      await tapOrClick("#panel-expand");
      check((await page.getAttribute("#panel-expand", "title")) === "Maximize panel", "restore didn't restore");
      const threads = await threadCount();
      await page.focus("#prompt");
      await page.keyboard.press("Meta+w");
      await waitFor("⌘W to close the last panel tab and the panel", async () => !(await page.locator("#panel").isVisible()));
      check((await threadCount()) === threads, "⌘W closed a thread");
    } else {
      for (let n = await page.locator("#panel-tabs .ptab").count(); n > 0; n--) await press(gui("#panel-tabs .ptab .lead"));
    }
    return items.join(" | ");
  });

  await step("app view: diff scopes, hidden whitespace, file tree and highlighting", async () => {
    await sendPrompt("code");
    await waitFor("the code turn's changes", async () => (await page.locator("#thread .open-diff").count()) >= 2, 15000);
    await press(page.locator("#thread .open-diff").last());
    await waitFor("app.js in the diff", () => page.locator('#panel-body .file[data-path="app.js"]').isVisible());
    const open = await page.locator("#panel-body .file").evaluateAll((els) => els.filter((e) => !e.hasAttribute("data-collapsed")).map((e) => e.dataset.path));
    check(!open.length, `open by default: ${open.join(", ")}`);
    check(!(await page.locator('#panel-body .file[data-path="notes.md"]').count()), "notes.md, changed only in whitespace, shows while whitespace is hidden");
    await press(gui('#panel-body [data-diff="whitespace"]'));
    await waitFor("notes.md with whitespace shown", () => page.locator('#panel-body .file[data-path="notes.md"]').isVisible());
    await press(gui('#panel-body [data-diff="whitespace"]'));
    await press(gui('#panel-body .file[data-path="app.js"] > header .change'));
    // Shiki with pierre-dark, as T3 colors code: keywords, function names, and the variables hljs left plain.
    const colorOf = (word, at = 0) => page.locator('#panel-body .file[data-path="app.js"] .code span', { hasText: new RegExp(`^${word}$`) }).nth(at).evaluate((el) => getComputedStyle(el).color).catch(() => "");
    const colors = {};
    await waitFor("Shiki's colors", async () => {
      Object.assign(colors, { return: await colorOf("return"), greet: await colorOf("greet"), name: await colorOf("name", 1) });
      return colors.return === "rgb(255, 103, 141)" && colors.greet === "rgb(157, 106, 251)" && colors.name === "rgb(255, 163, 89)";
    }, 15000).catch(() => check(false, `colors: ${JSON.stringify(colors)}`));
    const scope = async (label) => {
      await press(gui("#diff-scope"));
      await press(gui(`#panel-menu button:has(.label:text-is("${label}"))`));
    };
    await press(gui("#diff-scope"));
    const options = await page.locator("#panel-menu button .label").allTextContents();
    await press(gui("#panel-menu button[data-sub]"));
    const turns = await page.locator("#panel-submenu button").allTextContents();
    check(options.join("|") === "Changes|Uncommitted|Latest turn|Turn" && turns.length >= 2 && turns.every((o) => /^Turn \d+\S+/.test(o)), `scopes: ${options.join(" | ")} / ${turns.join(" | ")}`);
    await page.keyboard.press("Escape");
    await press(gui('#panel-body .file[data-path="app.js"] > header .change'));
    await press(gui('#panel-body [data-diff="tree"]'));
    await waitFor("the file tree", () => page.locator("#panel-body .file-tree").isVisible());
    await press(gui('#panel-body .file-tree [data-tree-file="app.js"]'));
    await waitFor("the tree to open app.js", async () => (await page.locator('#panel-body .file[data-path="app.js"]').getAttribute("data-collapsed")) === null);
    await press(gui('#panel-body [data-diff="tree"]'));
    await scope("Uncommitted");
    await waitFor("the uncommitted changes", async () => (await page.locator('#panel-body .file[data-path="notes.md"]').count()) === 1 && (await page.locator('#panel-body .file[data-path="app.js"]').count()) === 1);
    check(await gui('#panel-body [data-diff="refresh"]').isVisible(), "the working-tree scope has no Refresh");
    await press(gui('#panel-body [data-diff="refresh"]'));
    await scope("Changes");
    await waitFor("the base scope's answer", async () => /no default branch/.test(await text("#panel-body")));
    await scope("Latest turn");
    await waitFor("the latest turn", () => page.locator('#panel-body .file[data-path="app.js"]').isVisible());
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    return options.join(" | ");
  });

  await step("app view: a comment on a diff line goes into the composer", async () => {
    if (await page.locator('#panel-body .file[data-path="app.js"]').getAttribute("data-collapsed") === "") await press(gui('#panel-body .file[data-path="app.js"] > header .change'));
    await press(gui('#panel-body .file[data-path="app.js"] .no[data-line="2"]'));
    await page.keyboard.down("Shift");
    await press(gui('#panel-body .file[data-path="app.js"] .no[data-line="3"]'));
    await page.keyboard.up("Shift");
    check((await page.locator("#panel-body .file[data-path='app.js'] .dl.sel").count()) === 2, `${await page.locator("#panel-body .dl.sel").count()} lines selected after a Shift-click`);
    const bar = await page.locator("#panel-body .file[data-path='app.js'] .dl.sel .no").first().evaluate((el) => getComputedStyle(el, "::before").backgroundColor);
    check(bar === "rgb(105, 177, 255)", `the selection's bar is ${bar}`);
    await page.hover('#panel-body .file[data-path="app.js"] .no[data-line="3"]');
    await press(gui('#panel-body .file[data-path="app.js"] .no[data-line="3"] [data-comment-line]'));
    await page.fill("#panel-body [data-comment-text]", "rename greet");
    if (touch) await press(gui("#panel-body [data-comment-save]"));
    else await page.press("#panel-body [data-comment-text]", "Meta+Enter");
    await waitFor("the saved comment", () => page.locator("#panel-body .dcomment:not(.draft)").isVisible());
    check((await page.inputValue("#prompt")) === "", `the prompt holds ${JSON.stringify(await page.inputValue("#prompt"))}`);
    check(/app\.js:L2 to L3 rename greet/.test(await text("#attachments .attachment")), `the composer's chip reads ${await text("#attachments")}`);
    const sent = await page.evaluate(() => threadOf(active).attachments.map((a) => a.text).join("\n"));
    check(sent.startsWith('<review_comment filePath="app.js" lines="L2 to L3">\nrename greet\n```diff\n+function greet(name) {\n+  return "hi " + name;\n```\n</review_comment>'), `what goes out: ${JSON.stringify(sent)}`);
    await press(gui("#panel-body [data-comment-delete]"));
    await waitFor("the comment to go", async () => !(await page.locator("#panel-body .dcomment").count()) && !(await page.locator("#attachments .attachment").count()));
  });

  await step("app view: local servers come from the session's listening ports", async () => {
    if (touch && (await page.locator("#panel").isVisible())) await tapOrClick("#toggle-panel");
    await sendPrompt("serve");
    await waitFor("the fake server", async () => !!readLog("serve.port"));
    const port = readLog("serve.port").trim();
    await waitFor("the answer", async () => (await page.locator("#thread .assistant", { hasText: "Started a web server" }).count()) > 0);
    const scraped = await page.evaluate((p) => threadOf(active).items.some((i) => `${i.text ?? ""}${i.detail ?? ""}`.includes(`:${p}`)), port);
    check(!scraped, "the transcript names the port, so this step can't tell discovery from scraping");
    if (!(await page.locator("#panel").isVisible())) await tapOrClick("#toggle-panel");
    await fromPlus("Browser");
    const card = `#panel-body .servers button[data-url="http://localhost:${port}"]`;
    await waitFor("the server's card", async () => (await page.locator(card).count()) > 0, 12000);
    check(/node/.test(await text(`${card} b`)), `the card is titled ${await text(`${card} b`)}`);
    await press(gui(card));
    const frame = page.frameLocator("#panel-body iframe");
    await waitFor("the server's page", async () => /served by fake claude/.test((await frame.locator("body").textContent().catch(() => "")) ?? ""), 10000);
    return `port ${port}`;
  });

  await step("app view: browser loading bar, tab title, device sizes, recently used, ⌘⇧J", async () => {
    const port = readLog("serve.port").trim();
    await page.fill("#panel-body .url-bar input", `http://localhost:${port}/slow`);
    await page.press("#panel-body .url-bar input", "Enter");
    await waitFor("the loading bar", () => page.locator("#panel-body .loading-bar").isVisible(), 2000);
    check((await page.getAttribute('#panel-body [data-nav="reload"]', "title")) === "Stop", "while loading, Reload isn't Stop");
    await waitFor("the page to load", async () => !(await page.locator("#panel-body .loading-bar").isVisible()), 10000);
    await waitFor("the page's title on its tab", async () => (await text("#panel-tabs .ptab.active")) === "Sweep server");
    const more = async (label) => {
      await press(gui('#panel-body [data-nav="more"]'));
      await press(gui(`#panel-menu button:has-text("${label}")`));
    };
    await press(gui('#panel-body [data-nav="more"]'));
    await press(gui('#panel-menu [data-zoom="0.1"]'));
    const scale = () => page.locator("#panel-body iframe").evaluate((f) => f.style.transform);
    check((await scale()) === "scale(1.1)", `zoomed in, the page is ${await scale()}`);
    check((await text("#panel-menu .zoom .value")) === "110%", `the menu's zoom reads ${await text("#panel-menu .zoom .value")}`);
    await press(gui('#panel-menu [data-zoom="0"]'));
    await page.keyboard.press("Escape");
    await more("Show device toolbar");
    await page.selectOption("#panel-body [data-device]", "iPhone SE");
    const width = () => page.locator("#panel-body iframe").evaluate((f) => Math.round(f.getBoundingClientRect().width));
    await waitFor("the iPhone SE size", async () => (await width()) === 375);
    await press(gui('#panel-body [data-nav="rotate"]'));
    await waitFor("the rotated size", async () => (await width()) === 667);
    await more("Hide device toolbar");
    await fromPlus("Browser");
    await waitFor("Recently used", async () => /Recently used/.test(await text("#panel-body .browser-empty")));
    check((await page.locator(`#panel-body .servers button[data-url="${HUB}/manifest.webmanifest"]`).count()) === 1, "the earlier page isn't among Recently used");
    if (!touch) {
      await page.focus("#prompt");
      await page.keyboard.press("Meta+Shift+j");
      await waitFor("⌘⇧J to close the preview", async () => !(await page.locator("#panel").isVisible()));
      await page.keyboard.press("Meta+Shift+j");
      await waitFor("⌘⇧J to open the preview", () => openTab("browser").isVisible());
    }
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
  });

  await step("app view: PR reviewers, labels, timeline, Viewed, copy, and merge after a confirm", async () => {
    await tapOrClick("#toggle-panel");
    await tapOrClick("#pr-chip");
    await waitFor("#42's details", async () => /Add notes/.test(await text("#panel-body .pr-head h3")), 15000);
    const meta = await text("#panel-body .meta");
    check(/monalisa/.test(meta) && /octocat/.test(meta) && /documentation/.test(meta), `reviewers and labels: ${meta}`);
    await press(gui('#panel-body [data-pr-tab="timeline"]'));
    const timeline = await text("#panel-body .pr-timeline");
    check(/opened this pull request/.test(timeline) && /Write the notes/.test(timeline) && /View commit/.test(timeline), `timeline: ${timeline.slice(0, 120)}`);
    await press(gui('#panel-body [data-pr-tab="code"]'));
    await press(gui("#panel-body label.viewed input"));
    await waitFor("the viewed file to fold", async () => (await page.locator('#panel-body .file[data-path="notes.md"]').getAttribute("data-collapsed")) === "");
    check(await page.isChecked("#panel-body label.viewed input"), "Viewed didn't stay checked");
    check(/notes\.md/.test(await page.evaluate(() => localStorage.getItem("gui-pr-viewed"))), "Viewed isn't saved");
    await press(gui("#panel-body [data-pr-more]"));
    const items = await page.locator("#panel-menu button .label").allTextContents();
    for (const want of ["Refresh", "Merge now", "Open on GitHub", "Copy link", "Copy PR number"]) check(items.includes(want), `More menu: ${items.join(" | ")}`);
    await press(gui('#panel-menu button:has-text("Copy PR number")'));
    if (engine === "chromium") {
      await waitFor("the number on the clipboard", async () => (await page.evaluate(() => navigator.clipboard.readText())) === "#42");
      if (!touch) {
        await page.evaluate(() => navigator.clipboard.writeText(""));
        await page.focus("#prompt");
        await page.keyboard.press("Meta+Shift+k");
        await waitFor("⌘⇧K to copy the number", async () => (await page.evaluate(() => navigator.clipboard.readText())) === "#42");
      }
    }
    await press(gui('#panel-body [data-pr-do="merge"]'));
    check(/Confirm merge\?/.test(await text("#panel-body [data-pr-do]")), `after one click the button reads ${await text("#panel-body [data-pr-do]")}`);
    check(!/pr merge/.test(readLog("gh.log")), "one click merged");
    await press(gui('#panel-body [data-pr-do="merge"]'));
    await waitFor("gh to merge", async () => /pr merge --merge -- https:\/\/github\.com\/example\/alpha\/pull\/42/.test(readLog("gh.log")));
    await waitFor("the Merged state", async () => /Merged/.test(await text("#panel-body .pr > .crumbs")), 15000);
    await tapOrClick("#toggle-panel");
    await tapOrClick('#thread .pr-link[data-pr$="/pull/41"]');
    await waitFor("#41's details", async () => /Draft the notes/.test(await text("#panel-body .pr-head h3")), 15000);
    check((await text('#panel-body [data-pr-do="ready"]')) === "Ready for review", `a draft's main button reads ${await text("#panel-body [data-pr-do]")}`);
    await press(gui('#panel-body [data-pr-do="ready"]'));
    await waitFor("gh to mark it ready", async () => /pr ready -- https:\/\/github\.com\/example\/alpha\/pull\/41/.test(readLog("gh.log")));
    await tapOrClick("#toggle-panel");
    return items.join(" | ");
  });

  await step("app view: the browser's Back and Forward step through a cross-origin page, and its tab shows the page's title and icon", async () => {
    const port = readLog("serve.port").trim();
    await tapOrClick("#toggle-panel");
    await fromPlus("Browser");
    await page.fill("#panel-body .url-bar input", `http://localhost:${port}/a`);
    await page.press("#panel-body .url-bar input", "Enter");
    const frame = page.frameLocator("#panel-body iframe");
    await waitFor("page A", async () => /page a/.test((await frame.locator("body").textContent().catch(() => "")) ?? ""), 10000);
    await waitFor("the tab to read the page's title", async () => (await text("#panel-tabs .ptab.active")) === "Page A", 10000);
    await waitFor("the page's icon on the tab", async () => /^data:image\/png/.test((await page.getAttribute("#panel-tabs .ptab.active img.favicon", "src").catch(() => "")) ?? ""));
    check(await page.isDisabled('#panel-body [data-nav="back"]'), "Back works before the page went anywhere");
    await press(frame.locator("a"));
    await waitFor("the page's own link to land", async () => /served by fake claude/.test((await frame.locator("body").textContent().catch(() => "")) ?? ""), 10000);
    await waitFor("Back to wake", async () => !(await page.isDisabled('#panel-body [data-nav="back"]')));
    await press(gui('#panel-body [data-nav="back"]'));
    await waitFor("Back to return to page A", async () => /page a/.test((await frame.locator("body").textContent().catch(() => "")) ?? ""), 10000);
    check(!(await page.isDisabled('#panel-body [data-nav="forward"]')), "Forward is off after going back");
    await press(gui('#panel-body [data-nav="forward"]'));
    await waitFor("Forward to the page's link again", async () => /served by fake claude/.test((await frame.locator("body").textContent().catch(() => "")) ?? ""), 10000);
    await press(gui('#panel-body [data-nav="more"]'));
    const items = await page.locator("#panel-menu button .label").allTextContents();
    check(items.join("|") === "Hard reload|Show device toolbar", `the web app's More menu: ${items.join(" | ")}`);
    await page.keyboard.press("Escape");
    const title = await text("#panel-tabs .ptab.active");
    await tapOrClick("#toggle-panel");
    return title;
  });

  const panelOpen = () => page.locator("#panel").isVisible();
  const openPanelTo = async (kind, label) => {
    if (!(await panelOpen())) await tapOrClick("#toggle-panel");
    await waitFor("the panel", panelOpen);
    if (!(await openTab(kind).count())) await fromPlus(label);
    await waitFor(`the ${label} tab`, () => openTab(kind).isVisible());
  };
  const closePanelIfOpen = async () => {
    if (await panelOpen()) await tapOrClick("#toggle-panel");
    await waitFor("the panel to close", async () => !(await panelOpen()));
  };
  const alphaGit = (...args) => require("child_process").execFileSync("git", ["-C", path.join(HOME, "work/alpha"), "-c", "user.email=s@example.test", "-c", "user.name=s", ...args]).toString().trim();
  const menuLabels = () => page.locator("#panel-menu button:not([hidden]) .label").allTextContents();
  const menuItem = (label) => gui(`#panel-menu button:has(.label:text-is("${label}"))`);

  await step("app view: the device toolbar locks its aspect, and the page floats over the chat and back", async () => {
    await openPanelTo("browser", "Browser");
    if (!(await page.locator("#panel-body iframe").count())) {
      await page.fill("#panel-body .url-bar input", `http://localhost:${readLog("serve.port").trim()}/a`);
      await page.press("#panel-body .url-bar input", "Enter");
    }
    await press(gui('#panel-body [data-nav="more"]'));
    await press(menuItem("Show device toolbar"));
    await page.selectOption("#panel-body [data-device]", "iPhone SE");
    await press(gui('#panel-body [data-nav="lock"]'));
    check((await page.getAttribute('#panel-body [data-nav="lock"]', "title")) === "Unlock aspect ratio", "the lock didn't lock");
    await page.fill('#panel-body [data-size="w"]', "750");
    await page.locator('#panel-body [data-size="w"]').dispatchEvent("change");
    await waitFor("the locked height", async () => (await page.inputValue('#panel-body [data-size="h"]')) === "1334");
    await press(gui('#panel-body [data-nav="more"]'));
    await press(menuItem("Hide device toolbar"));
    await press(gui('#panel-body [data-nav="pip"]'));
    await waitFor("the page to float", async () => /floating over the chat/.test(await text("#panel-body .viewport.floating")));
    if (touch) await closePanelIfOpen();
    await waitFor("the floating page", () => page.locator("#preview-pip iframe").isVisible());
    const box = await rect("#preview-pip");
    check(box.width >= 300 && box.height >= 200, `the floating page is ${box.width}×${box.height}`);
    await press(gui('#preview-pip [data-pip="panel"]'));
    await waitFor("the page back in the panel", async () => (await page.locator("#panel-body .viewport iframe").count()) === 1 && !(await page.locator("#preview-pip").isVisible()));
    await closePanelIfOpen();
  });

  await step("app view: the diff picks a turn from its menu, compares with a branch or its remote copy, and keeps that per thread", async () => {
    const head = alphaGit("symbolic-ref", "--short", "HEAD");
    alphaGit("commit", "-q", "--allow-empty", "-m", "base");
    alphaGit("branch", "feature-base");
    alphaGit("update-ref", "refs/remotes/origin/feature-base", "HEAD");
    try {
      await openPanelTo("diff", "Diff");
      await press(gui("#diff-scope"));
      await press(menuItem("Changes"));
      await waitFor("the base picker", () => gui("#panel-body [data-base-pick]").isVisible(), 10000);
      check((await text("#panel-body .refs .head")) === head, `the head reads ${await text("#panel-body .refs .head")}`);
      await press(gui("#panel-body [data-base-pick]"));
      await waitFor("the refs", () => page.locator("#panel-menu .menu-search").isVisible());
      const refs = await menuLabels();
      check(refs[0] === "Automatic" && refs.includes("feature-base") && refs.includes(head), `refs: ${refs.join(" | ")}`);
      check((await page.locator('#panel-menu button:has(.label:text-is("feature-base")) .switch').count()) === 1, "feature-base has no switch to its remote copy");
      await page.fill("#panel-menu .menu-search", "feat");
      check((await menuLabels()).join("|") === "feature-base", `filtered: ${(await menuLabels()).join(" | ")}`);
      await press(menuItem("feature-base"));
      await waitFor("the base to read feature-base", async () => (await text("#panel-body .base-pick")) === "feature-base", 10000);
      await waitFor("the files against feature-base", async () => (await page.locator("#panel-body .file").count()) > 0, 10000);
      await press(gui("#panel-body [data-base-pick]"));
      await press(gui('#panel-menu button:has(.label:text-is("feature-base")) .switch'));
      await waitFor("the remote copy", async () => (await text("#panel-body .base-pick")) === "origin/feature-base", 10000);
      const saved = JSON.parse(await page.evaluate(() => load(`gui-diff:${threadKey(active)}`)));
      check(saved.scope === "base" && saved.base === "origin/feature-base", `saved: ${JSON.stringify(saved)}`);
      await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/beta"))));
      await waitFor("beta", async () => /beta/.test(await text("#chat-head .crumb")));
      check(!(await page.evaluate(() => load(`gui-diff:${threadKey(active)}`))), "beta took alpha's diff scope");
      await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
      await openPanelTo("diff", "Diff");
      await waitFor("alpha's scope back", async () => (await text("#diff-scope")) === "Changes" && (await text("#panel-body .base-pick")) === "origin/feature-base", 10000);
      await press(gui("#diff-scope"));
      await press(gui("#panel-menu button[data-sub]"));
      const turns = await page.locator("#panel-submenu button").allTextContents();
      await press(gui("#panel-submenu button >> nth=0"));
      await waitFor("the first turn", async () => (await text("#diff-scope")) === "Turn 1");
      check(turns.length >= 2, `turns: ${turns.join(" | ")}`);
      await press(gui("#diff-scope"));
      await press(menuItem("Latest turn"));
      await waitFor("the latest turn", async () => (await text("#diff-scope")) === "Latest turn");
    } finally {
      alphaGit("update-ref", "-d", "refs/remotes/origin/feature-base");
      alphaGit("branch", "-D", "feature-base");
      alphaGit("update-ref", "-d", `refs/heads/${head}`);
    }
    await closePanelIfOpen();
  });

  await step("app view: the diff opens unchanged lines 100 at a time and draws a long diff as it scrolls", async () => {
    const file = path.join(HOME, "work/alpha/long.txt");
    const rows = Array.from({ length: 600 }, (_, i) => `line ${i + 1}`);
    fs.writeFileSync(file, `${rows.join("\n")}\n`);
    fs.mkdirSync(path.join(HOME, "work/alpha/nested/inner"), { recursive: true });
    fs.writeFileSync(path.join(HOME, "work/alpha/nested/inner/file.js"), "export const deep = 1;\n");
    const head = alphaGit("symbolic-ref", "--short", "HEAD");
    const first = [];
    try {
      await openPanelTo("diff", "Diff");
      await press(gui("#diff-scope"));
      await press(menuItem("Uncommitted"));
      await waitFor("long.txt in the diff", () => page.locator('#panel-body .file[data-path="long.txt"]').isVisible(), 10000);
      const bar = await page.locator("#panel-body .diff-bar").evaluate((el) => [getComputedStyle(el).height, getComputedStyle(el).borderBottomColor]);
      const sheet = await page.evaluate(() => matchMedia("(max-width: 980px)").matches);
      check(sheet ? bar[0] === (touch ? "48px" : "40px") && bar[1] !== "rgba(0, 0, 0, 0)" : bar[0] === "28px" && bar[1] === "rgba(0, 0, 0, 0)", `the diff's header row is ${bar.join(", ")}`);
      if ((await page.locator('#panel-body .file[data-path="long.txt"]').getAttribute("data-collapsed")) === "") await press(gui('#panel-body .file[data-path="long.txt"] > header .change'));
      check((await page.locator('#panel-body .file[data-path="long.txt"][data-fresh] .hunks').evaluate((el) => getComputedStyle(el).animationName)) === "file-in", "an opened file doesn't fade in");
      check((await page.locator("#panel-body .rows-later").count()) > 0, "a 600-line file drew every row up front");
      await press(gui('#panel-body [data-diff="tree"]'));
      await waitFor("the folders in the tree", () => page.locator('#panel-body .file-tree [data-tree-dir="nested/inner"]').isVisible());
      const jsColor = await page.locator('#panel-body .file-tree [data-tree-file="nested/inner/file.js"] .ficon').evaluate((el) => getComputedStyle(el).color);
      check(jsColor === "rgb(240, 219, 79)", `a .js file's icon is ${jsColor}`);
      check((await page.getAttribute("#panel-body [data-tree-all]", "title")) === "Collapse all folders", "no Collapse all folders");
      await press(gui("#panel-body [data-tree-all]"));
      await waitFor("the folders to close", async () => !(await page.locator('#panel-body .file-tree [data-tree-file="nested/inner/file.js"]').count()));
      await press(gui("#panel-body [data-tree-all]"));
      await waitFor("the folders to open", () => page.locator('#panel-body .file-tree [data-tree-file="nested/inner/file.js"]').isVisible());
      await press(gui('#panel-body [data-diff="tree"]'));
      const line = (n) => page.locator('#panel-body .file[data-path="long.txt"] .dl .code', { hasText: new RegExp(`^line ${n}( changed)?$`) }).count();
      await waitFor("the last line once scrolled to", async () => {
        await page.evaluate(() => (panelBody.scrollTop = panelBody.scrollHeight));
        return (await line(600)) === 1;
      }, 10000);
      check(!(await page.locator("#panel-body .rows-later").count()), "rows stayed undrawn at the end");
      alphaGit("add", "long.txt");
      alphaGit("commit", "-q", "-m", "long");
      rows[9] = "line 10 changed";
      rows[299] = "line 300 changed";
      fs.writeFileSync(file, `${rows.join("\n")}\n`);
      await press(gui('#panel-body [data-diff="refresh"]'));
      await waitFor("the two changes", async () => (await page.locator('#panel-body .file[data-path="long.txt"] > header .del').textContent().catch(() => "")) === "-2", 10000);
      if ((await page.locator('#panel-body .file[data-path="long.txt"]').getAttribute("data-collapsed")) === "") await press(gui('#panel-body .file[data-path="long.txt"] > header .change'));
      const gaps = () => page.locator('#panel-body .file[data-path="long.txt"] .gap .label').allTextContents();
      await waitFor("the unchanged stretches", async () => (await gaps()).length === 3);
      first.push(...(await gaps()));
      check(first[0] === "6 unmodified lines" && first[1] === "283 unmodified lines" && first[2] === "More unchanged context may be available", `gaps: ${first.join(" | ")}`);
      await press(gui('#panel-body .file[data-path="long.txt"] .gap[data-gap="1"] [data-expand="top"]'));
      await waitFor("100 more lines", async () => (await gaps())[1] === "183 unmodified lines", 10000);
      check((await line(113)) === 1 && (await line(114)) === 0, "lines 14 to 113 didn't open");
      if (!touch) {
        await page.hover('#panel-body .file[data-path="long.txt"] .gap[data-gap="1"] .label');
        const lit = await page.locator('#panel-body .file[data-path="long.txt"] .gap[data-gap="1"] .label').evaluate((el) => getComputedStyle(el).color);
        check(lit === "rgb(245, 245, 245)", `a hovered separator reads ${lit}`);
      }
      await press(gui('#panel-body .file[data-path="long.txt"] .gap[data-gap="1"] [data-expand="all"]'));
      await waitFor("the whole middle", async () => (await gaps()).length === 2);
      check((await gaps())[1] === "297 unmodified lines", `once read, the end reads ${(await gaps())[1]}`);
      await press(gui('#panel-body .file[data-path="long.txt"] .gap[data-gap="2"] [data-expand="all"]'));
      await waitFor("the end of the file", async () => {
        await page.evaluate(() => (panelBody.scrollTop = panelBody.scrollHeight));
        return (await line(600)) === 1;
      });
      await press(gui("#diff-scope"));
      await press(menuItem("Latest turn"));
    } finally {
      alphaGit("update-ref", "-d", `refs/heads/${head}`);
      alphaGit("update-index", "--force-remove", "long.txt");
      fs.rmSync(file, { force: true });
      fs.rmSync(path.join(HOME, "work/alpha/nested"), { recursive: true, force: true });
    }
    await closePanelIfOpen();
    return first.join(" | ");
  });

  await step("app view: Files lists the folder, a File tab shows and saves a file, and ⌘P and ⌘⇧F find files and text", async () => {
    await openPanelTo("files", "Files");
    await waitFor("the folder's files", () => page.locator('#panel-body [data-files-open="app.js"]').isVisible());
    await page.fill("#panel-body [data-files-filter]", "notes");
    await waitFor("the filter", async () => (await page.locator("#panel-body [data-files-open]").count()) === 1);
    await press(gui('#panel-body [data-files-open="notes.md"]'));
    await waitFor("notes.md in a File tab", async () => (await text('#panel-tabs .ptab.active[data-panel="file"]')) === "notes.md");
    await waitFor("its markdown preview", () => page.locator("#panel-body .file-markdown").isVisible());
    await press(gui("#panel-body [data-file-rendered]"));
    await waitFor("its source", () => page.locator("#panel-body .code-view").isVisible());
    if (touch) await page.evaluate(() => openPicker("files"));
    else {
      await page.focus("#prompt");
      await page.keyboard.press("Meta+p");
    }
    await waitFor("the file picker", () => page.locator("#file-picker .query").isVisible());
    await page.fill("#file-picker .query", "apjs");
    await waitFor("app.js picked", async () => (await text("#file-picker button.on .title")) === "app.js");
    await page.press("#file-picker .query", "Enter");
    await waitFor("app.js in a File tab", async () => (await text('#panel-tabs .ptab.active[data-panel="file"]')) === "app.js");
    await waitFor("its lines", async () => (await page.locator("#panel-body .code-view .fl").count()) === 4);
    const source = fs.readFileSync(path.join(HOME, "work/alpha/app.js"), "utf8");
    await page.locator("#panel-body .code-edit").fill(`${source}// saved from the panel\n`);
    await waitFor("the unsaved dot", () => page.locator('#panel-tabs .ptab.active .pending').isVisible());
    if (touch) await press(gui("#panel-body [data-file-save]"));
    else await page.keyboard.press("Meta+s");
    await waitFor("the saved file", async () => fs.readFileSync(path.join(HOME, "work/alpha/app.js"), "utf8").endsWith("// saved from the panel\n"));
    await waitFor("the dot to go", async () => !(await page.locator('#panel-tabs .ptab.active .pending').count()));
    fs.writeFileSync(path.join(HOME, "work/alpha/app.js"), source);
    if (touch) await page.evaluate(() => openPicker("content"));
    else await page.keyboard.press("Meta+Shift+f");
    await waitFor("the search", async () => (await page.getAttribute("#file-picker .query", "placeholder")) === "Search project contents…");
    await page.fill("#file-picker .query", "greet(");
    await waitFor("a match in app.js", async () => /app\.js/.test(await text("#file-picker .results .group")) && (await text("#file-picker .results button.on .line")) === "2", 10000);
    await page.press("#file-picker .query", "Enter");
    await waitFor("app.js at line 2", () => page.locator('#panel-body .code-view .fl.revealed[data-n="2"]').isVisible());
    await closePanelIfOpen();
  });

  await step("app view: file paths in messages are chips that open the file at its line", async () => {
    await sendPrompt("paths");
    await waitFor("the chips", async () => (await page.locator("#thread .file-chip").count()) === 2, 10000);
    const chips = await page.locator("#thread .file-chip").allTextContents();
    check(chips.join("|") === "app.js:2|the notes", `chips: ${chips.join(" | ")}`);
    const plain = await page.locator("#thread .assistant code").allTextContents();
    check(["v1.2.3", "example.com/a.js", "npm"].every((c) => plain.includes(c)), `left as code: ${plain.join(" | ")}`);
    await press(page.locator("#thread .file-chip").first());
    await waitFor("app.js at line 2", () => page.locator('#panel-body .code-view .fl.revealed[data-n="2"]').isVisible());
    await closePanelIfOpen();
    return chips.join(" | ");
  });

  await step("app view: a Terminal tab runs a shell in the thread's folder, and its links open in the preview", async () => {
    await openPanelTo("terminal", "Terminal");
    const screen = () => page.locator("#panel-body .xterm-rows").textContent();
    await waitFor("the shell", async () => (await page.locator("#panel-body .xterm-rows").count()) === 1, 10000);
    await page.locator("#panel-body .xterm-helper-textarea").focus();
    const port = readLog("serve.port").trim();
    await page.keyboard.type(`pwd; echo http://localhost:${port}/a\n`);
    await waitFor("the folder and the link", async () => /work\/alpha/.test(await screen()) && new RegExp(`localhost:${port}/a`).test(await screen()), 10000);
    const at = await page.evaluate((url) => {
      for (const row of [...document.querySelectorAll("#panel-body .xterm-rows > div")].reverse()) {
        const i = row.textContent.indexOf(url);
        if (i < 0 || row.textContent.trim() !== url) continue;
        const range = document.createRange();
        const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
        let seen = 0;
        for (let n = walker.nextNode(); n; n = walker.nextNode()) {
          if (seen + n.length > i + 4) {
            range.setStart(n, i + 4 - seen);
            range.setEnd(n, i + 5 - seen);
            const r = range.getBoundingClientRect();
            return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
          }
          seen += n.length;
        }
      }
      return null;
    }, `http://localhost:${port}/a`);
    check(at, "the link isn't on screen");
    await page.mouse.move(at.x, at.y);
    await sleep(300);
    await page.mouse.click(at.x, at.y);
    await waitFor("the link in the preview", () => openTab("browser").isVisible(), 10000);
    await waitFor("page A", async () => /page a/.test((await page.frameLocator("#panel-body iframe").locator("body").textContent().catch(() => "")) ?? ""), 10000);
    await page.click('#panel-tabs .ptab[data-panel="terminal"]');
    await press(gui('#panel-tabs .ptab[data-panel="terminal"] .lead'));
    await closePanelIfOpen();
  });

  await step("app view: Linked pull requests lists the thread's pull requests and links another", async () => {
    await openPanelTo("prs", "Linked pull requests");
    await waitFor("the thread's two", async () => (await page.locator("#panel-body .pr-row").count()) === 2 && /Draft the notes/.test(await text("#panel-body .prs-list")), 15000);
    check(/1 open · 2 linked/.test(await text("#panel-body .prs footer")), `footer: ${await text("#panel-body .prs footer")}`);
    await press(gui("#panel-body [data-prs-link]"));
    await page.fill("#link-pr input", "https://github.com/example/alpha/pull/43");
    await page.press("#link-pr input", "Enter");
    await waitFor("#43 linked", async () => (await page.locator("#panel-body .pr-row").count()) === 3, 10000);
    await press(gui('#panel-body [data-prs-more$="/pull/43"]'));
    await press(menuItem("Unlink from thread"));
    await waitFor("#43 unlinked", async () => (await page.locator("#panel-body .pr-row").count()) === 2);
    await press(gui('#panel-body .pr-row[data-prs-open$="/pull/41"] .what'));
    await waitFor("#41 opened", async () => /#41/.test(await text('#panel-tabs .ptab.active[data-panel="pr"]')));
    await closePanelIfOpen();
  });

  await step("app view: the Device tab starts the device hub and shows its dashboard", async () => {
    await openPanelTo("device", "Device");
    await waitFor("the hub's dashboard", async () => /Expo Device Hub dashboard/.test((await page.frameLocator("#panel-body iframe").locator("body").textContent().catch(() => "")) ?? ""), 20000);
    check(/expo-device-hub@0\.15\.3 --host 127\.0\.0\.1 --port \d+ --frame-ancestor http/.test(readLog("npx.log")), `npx ran: ${readLog("npx.log")}`);
    await page.dblclick('#panel-tabs .ptab[data-panel="device"] span:last-child').catch(() => {});
    if (!touch) {
      await page.fill("#panel-tabs .ptab .rename", "Sim");
      await page.press("#panel-tabs .ptab .rename", "Enter");
      await waitFor("the renamed tab", async () => (await text('#panel-tabs .ptab[data-panel="device"]')) === "Sim");
    }
    await press(gui('#panel-tabs .ptab[data-panel="device"] .lead'));
    await closePanelIfOpen();
  });

  await step("app view: a pull request's title and description edit, its menu closes, reopens, reverts and auto-merges, and its checks, stack, threads and crumbs show", async () => {
    await tapOrClick('#thread .pr-link[data-pr$="/pull/41"]');
    await waitFor("#41", async () => /Draft the notes/.test(await text("#panel-body .pr-head h3")), 15000);
    await press(gui("#panel-body [data-edit=title]"));
    await page.fill('#panel-body [data-pr-edit="title"]', "Draft the better notes");
    await page.press('#panel-body [data-pr-edit="title"]', "Enter");
    await waitFor("the new title", async () => /Draft the better notes/.test(await text("#panel-body .pr-head h3")), 10000);
    await press(gui("#panel-body [data-edit=body]"));
    await page.fill('#panel-body [data-pr-edit="body"]', "Adds better notes.");
    await press(gui('#panel-body [data-edit-save="body"]'));
    await waitFor("the new description", async () => /Adds better notes\./.test(await text("#panel-body .pr-body .md")), 10000);
    await press(gui("#panel-body [data-checks-pop]"));
    check(/Checks · 1 passed \/ 1 failed/.test(await text("#panel-menu .group")) && (await menuLabels()).join("|") === "build|lint", `checks popover: ${await text("#panel-menu")}`);
    await page.keyboard.press("Escape");
    await press(gui('#panel-body details[data-fold="checks"] > summary'));
    await press(gui('#panel-body .checks li.fail [data-check-rerun]'));
    await waitFor("gh to re-run the job", async () => /^run rerun --job 3$/m.test(readLog("gh.log")));
    await press(gui("#panel-body [data-pr-more]"));
    await press(menuItem("Ask a question"));
    check((await page.inputValue("#prompt")).startsWith("About pull request #41 (https://github.com/example/alpha/pull/41): "), `Ask a question wrote ${await page.inputValue("#prompt")}`);
    await page.fill("#prompt", "");
    await press(gui('#panel-body .checks li.fail [data-check-fix]'));
    await waitFor("claude asked to fix lint", async () => /The check "lint" fails on pull request #41/.test(await text("#thread")), 10000);
    check(await gui('#panel-body .review-outcome[aria-label="Review required"]').isVisible(), "no review outcome icon");
    await press(gui("#panel-body [data-pr-more]"));
    const items = await menuLabels();
    for (const want of ["Ask a question", "Explain this PR", "Fix findings", "Write guide", "Convert to draft", "Merge now", "Enable auto-merge", "Unlink from this thread", "Close pull request"]) check(items.includes(want), `More menu: ${items.join(" | ")}`);
    await press(menuItem("Enable auto-merge"));
    check(await menuItem("Confirm auto-merge?").isVisible(), "auto-merge didn't ask again");
    await press(menuItem("Confirm auto-merge?"));
    await waitFor("auto-merge on", async () => /Auto-merge \(merge\)/.test(await text("#panel-body .pr > .crumbs")), 10000);
    await press(gui("#panel-body [data-pr-more]"));
    await press(menuItem("Close pull request"));
    await press(menuItem("Confirm close?"));
    await waitFor("#41 closed", async () => /Closed/.test(await text("#panel-body .pr > .crumbs")), 10000);
    await press(gui("#panel-body [data-pr-more]"));
    await press(menuItem("Reopen pull request"));
    await waitFor("#41 reopened", async () => /pr reopen -- https:\/\/github\.com\/example\/alpha\/pull\/41/.test(readLog("gh.log")) && !/Closed/.test(await text("#panel-body .pr > .crumbs")), 10000);
    await tapOrClick('#thread .pr-link[data-pr$="/pull/42"]');
    await waitFor("#42", async () => /Merged/.test(await text("#panel-body .pr > .crumbs")), 15000);
    await press(gui('#panel-body [data-pr-tab="summary"]'));
    await waitFor("its stack", () => gui("#panel-body [data-pr-stack]").isVisible(), 10000);
    await press(gui("#panel-body [data-pr-stack]"));
    check((await menuLabels()).some((l) => /#43 Build on the notes/.test(l)), `stack: ${(await menuLabels()).join(" | ")}`);
    await page.keyboard.press("Escape");
    check((await page.getAttribute("#panel-body [data-pr-threads]", "aria-label")) === "Linked from 1 thread", "no thread links");
    await press(gui("#panel-body [data-pr-more]"));
    await press(menuItem("Revert changes"));
    await press(menuItem("Confirm revert?"));
    await waitFor("gh to revert", async () => /pr revert -- https:\/\/github\.com\/example\/alpha\/pull\/42/.test(readLog("gh.log")), 10000);
    await page.evaluate(() => (panelBody.scrollTop = 0));
    check(!(await page.locator("#panel-body .pr > .crumbs.condensed").count()), "the crumbs start condensed");
    await page.evaluate(() => (panelBody.style.maxHeight = "240px"));
    await page.evaluate(() => {
      panelBody.scrollTop = 200;
      panelBody.dispatchEvent(new Event("scroll"));
    });
    await waitFor("the crumbs to condense", () => page.locator("#panel-body .pr > .crumbs.condensed .condensed-title").isVisible());
    check((await text("#panel-body .pr > .crumbs .condensed-title")) === "Add notes", "the condensed crumbs miss the title");
    await page.evaluate(() => (panelBody.style.maxHeight = ""));
    await closePanelIfOpen();
    return items.join(" | ");
  });

  await step("app view: pull request comments filter, sort, reply and comment, and the Code tab submits a review with line comments", async () => {
    await tapOrClick('#thread .pr-link[data-pr$="/pull/42"]');
    await waitFor("#42", async () => /#42/.test(await text("#panel-body .pr > .crumbs")), 15000);
    await press(gui('#panel-body [data-pr-tab="summary"]'));
    await waitFor("#42's comments", async () => /Why a second line\?/.test(await text("#panel-body .conversation")), 15000);
    check(!/Bumped a dependency/.test(await text("#panel-body .conversation")) && /1 hidden/.test(await text("#panel-body [data-comment-filter]")), "a bot's comment shows by default");
    await press(gui("#panel-body [data-comment-filter]"));
    await press(menuItem("Bot comments"));
    await waitFor("the bot's comment", async () => /Bumped a dependency/.test(await text("#panel-body .conversation")));
    const order = async () => (await page.locator("#panel-body .conversation > li").allTextContents()).map((t) => t.trim().slice(0, 12));
    const newest = await order();
    await press(gui("#panel-body [data-comment-order]"));
    await waitFor("oldest first", async () => (await text("#panel-body [data-comment-order]")) === "Oldest first");
    check(JSON.stringify(await order()) === JSON.stringify([...newest].reverse()), `order: ${JSON.stringify(newest)} then ${JSON.stringify(await order())}`);
    await press(gui('#panel-body [data-reply="T1"]'));
    await page.fill('#panel-body [data-pr-text="reply"]', "Because two is clearer.");
    await press(gui('#panel-body [data-pr-send="reply"]'));
    await waitFor("the reply", async () => /Because two is clearer\./.test(await text("#panel-body .conversation .thread")), 10000);
    await page.fill('#panel-body [data-pr-text="comment"]', "Ready when you are.");
    await press(gui('#panel-body [data-pr-send="comment"]'));
    await waitFor("the comment", async () => /Ready when you are\./.test(await text("#panel-body .conversation")), 10000);
    await press(gui('#panel-body [data-pr-tab="code"]'));
    if ((await page.locator('#panel-body .file[data-path="notes.md"]').getAttribute("data-collapsed")) === "") await press(gui('#panel-body .file[data-path="notes.md"] > header .change'));
    await page.hover('#panel-body .file[data-path="notes.md"] .no[data-line="2"]');
    await press(gui('#panel-body .file[data-path="notes.md"] .no[data-line="2"] [data-comment-line]'));
    await page.fill("#panel-body [data-comment-text]", "Say why");
    await press(gui("#panel-body [data-comment-save]"));
    await waitFor("the pending comment", async () => (await text("#panel-body [data-review-open]")) === "Finish review (1)");
    check(!/review /.test(readLog("gh.log")), "a pending comment went out on its own");
    await press(gui("#panel-body [data-review-open]"));
    await page.fill("#panel-body [data-review-body]", "Two small notes.");
    await page.check('#panel-body [data-review-form] input[value="REQUEST_CHANGES"]');
    await press(gui("#panel-body [data-review-submit]"));
    const review = () => JSON.parse(readLog("gh.log").split("\n").find((l) => l.startsWith("review "))?.slice(7) ?? "null");
    await waitFor("the review", async () => !!review(), 10000);
    check(JSON.stringify(review()) === JSON.stringify({ body: "Two small notes.", comments: [{ body: "Say why", line: 2, path: "notes.md", side: "RIGHT" }], event: "REQUEST_CHANGES" }), `review: ${JSON.stringify(review())}`);
    await press(gui('#panel-body [data-pr-tab="summary"]'));
    await closePanelIfOpen();
  });

  await step("app view: the Guide tab has claude write a review guide and walks its stops", async () => {
    await tapOrClick('#thread .pr-link[data-pr$="/pull/42"]');
    await waitFor("#42", async () => /#42/.test(await text("#panel-body .pr > .crumbs")), 15000);
    await press(gui('#panel-body [data-pr-tab="guide"]'));
    await waitFor("the empty guide", () => gui("#panel-body [data-guide-write]").isVisible());
    await press(gui("#panel-body [data-guide-write]"));
    await waitFor("claude writing", async () => /Writing a review guide/.test(await text("#panel-body .pr-body")));
    await waitFor("the guide", async () => /The notes file arrives/.test(await text("#panel-body .stop h4")), 20000);
    check(/-p --output-format json --json-schema --allowedTools Read,Grep,Glob/.test(readLog("claude-p.log")) && /change request #42/.test(readLog("claude-p.log")), `claude ran: ${readLog("claude-p.log")}`);
    check(/Product Should notes live at the root\?/.test(await text("#panel-body .stop .attention")), `attention: ${await text("#panel-body .stop .attention")}`);
    check((await page.locator("#panel-body .stop .excerpt .dl").count()) === 2, "the excerpt doesn't show the two new lines");
    await press(gui('#panel-body .stop [data-guide-stop="1"]'));
    await waitFor("stop 2", async () => (await text("#panel-body .stop .step")) === "Stop 2 of 2");
    await press(gui('#panel-body [data-pr-tab="summary"]'));
    await closePanelIfOpen();
  });

  await step("app view: panel tabs scroll with chevrons, PR links open on GitHub with ⌘ or a middle click, and the panel can animate", async () => {
    await page.evaluate(() => (window.open = (url) => (window.__opened = url)));
    if (!touch) {
      await showSidebar();
      await page.click('#thread-list .thread-row [data-pr]', { modifiers: ["Meta"] });
      await waitFor("the badge to open GitHub", async () => /github\.com\/example\/alpha\/pull\/4\d/.test((await page.evaluate(() => window.__opened)) ?? ""));
      await page.evaluate(() => (window.__opened = ""));
      await page.click('#thread .pr-link[data-pr$="/pull/41"]', { button: "middle" });
      await waitFor("a middle click to open GitHub", async () => /pull\/41$/.test((await page.evaluate(() => window.__opened)) ?? ""));
    }
    await openPanelTo("browser", "Browser");
    for (let i = 0; i < 6; i++) await fromPlus("Browser");
    // On touch the strip scrolls under a finger, so only a pointer gets chevrons.
    if (!touch) {
      await waitFor("the left chevron, the new tab shown at the end", () => page.locator("#panel-tabs .tab-scroll.left").isVisible());
      await page.evaluate(() => (document.querySelector("#panel-tabs .strip").scrollLeft = 0));
      await waitFor("the right chevron", () => page.locator("#panel-tabs .tab-scroll.right").isVisible());
      await press(gui("#panel-tabs .tab-scroll.right"));
      await waitFor("the left chevron", () => page.locator("#panel-tabs .tab-scroll.left").isVisible());
    }
    if (!touch) {
      await page.click('#panel-tabs .ptab.active', { button: "right" });
      await press(menuItem("Close others"));
    } else {
      for (let n = await page.locator("#panel-tabs .ptab").count(); n > 1; n--) await press(gui("#panel-tabs .ptab:not(.active) .lead"));
    }
    await closePanelIfOpen();
    await page.evaluate(() => toggleSettings());
    await tapOrClick('#settings-nav [data-page="general"]');
    await page.selectOption("#panel-animation", "200");
    await page.evaluate(() => toggleSettings());
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
    await tapOrClick("#toggle-panel");
    check(await page.evaluate(() => document.body.classList.contains("panel-animating")), "opening didn't animate");
    await waitFor("the animation to end", async () => !(await page.evaluate(() => document.body.classList.contains("panel-animating"))));
    await page.evaluate(() => save("gui-panel-animation", null));
    await closePanelIfOpen();
  });

  await step("app view: Open in an editor from the header", async () => {
    await waitFor("the Open button", () => page.locator("#open-in").isVisible());
    await tapOrClick("#open-in .pick");
    const editors = await page.locator("#panel-menu button .label").allTextContents();
    check(editors.includes("Zed") && editors.includes("Finder"), `editors: ${editors.join(" | ")}`);
    await press(gui('#panel-menu button:has-text("Zed")'));
    await sleep(300);
    check(!readLog("open.log"), "picking an editor opened it");
    await tapOrClick("#open-in .main");
    const opened = new RegExp(`^-a .*/home/Applications/Zed\\.app ${HOME.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}/work/alpha$`, "m");
    await waitFor("the folder to open in Zed", async () => opened.test(readLog("open.log")));
    if (!touch) {
      await page.focus("#prompt");
      await page.keyboard.press("Meta+o");
      await waitFor("⌘O to open it again", async () => readLog("open.log").trim().split("\n").length === 2);
    }
    await tapOrClick("#toggle-panel");
    if (!(await openTab("diff").count())) await fromPlus("Diff");
    await waitFor("app.js in the diff", () => page.locator('#panel-body .file[data-path="app.js"] .path').isVisible());
    await press(gui('#panel-body .file[data-path="app.js"] > header .path'));
    await waitFor("the path to open a File tab", async () => (await text('#panel-tabs .ptab.active[data-panel="file"]')) === "app.js");
    await press(gui("#panel-body [data-file-open-in]"));
    await waitFor("the file to open in Zed", async () => /Zed\.app .*\/work\/alpha\/app\.js$/m.test(readLog("open.log")));
    await tapOrClick("#toggle-panel");
    return editors.join(" | ");
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
    await page.fill("#prompt", "sent while scrolled up");
    await page.evaluate(() => (threadView.scrollTop = 0));
    await tapOrClick("#send");
    await waitTerm(/echo: sent while scrolled up/);
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
    // The timeline's steps filled beta's thread, so this step and the next run in a new one.
    await page.evaluate(() => newThread(true));
    await waitFor("a new thread in beta", async () => (await page.locator("#thread-list .thread-row").count()) === 3, 10000);
    await waitTerm(/fake claude in .*work\/beta/);
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
    git("add", "-A");
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
    await page.evaluate(() => closeTab(active));
    await waitFor("two threads", async () => (await page.locator("#thread-list .thread-row").count()) === 2);
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

  // The sidebar's organization, as T3 Code's: rows by project with icons, pins, shelves, the row
  // menu, search, new-thread shortcuts, reopen and resize. Every step leaves alpha open.
  const keyFor = (name) => page.evaluate((n) => tabs.find((t) => projectName(t) === n)?.key, name);
  const tone = (selector) => page.locator(selector).first().evaluate((el) => getComputedStyle(el).color);
  const rowSel = (key) => `#thread-list .thread-row[data-tab="${key}"]`;
  const sideOrder = () => page.evaluate(() => [...document.querySelectorAll("#thread-list .thread-row")].map((r) => projectName(tabByKey(r.dataset.tab))));
  const crumbProject = () => text("#chat-head .crumb .project > span");
  const openRow = async (key) => {
    await showSidebar();
    await press(page.locator(rowSel(key)));
    await waitFor("the thread to open", () => page.evaluate((k) => active?.key === k, key));
  };
  const rowMenu = async (selector) => {
    await showSidebar();
    const row = page.locator(selector).first();
    if (touch) {
      const box = await row.boundingBox();
      await row.dispatchEvent("contextmenu", { clientX: box.x + 24, clientY: box.y + 18 });
    } else await row.click({ button: "right" });
    await waitFor("the row menu", () => page.locator("#side-menu").isVisible());
  };
  const menuPick = (label) => press(page.locator("#side-menu button", { hasText: label }).first());
  const rowAct = async (key, act) => {
    await page.locator(rowSel(key)).hover();
    await press(page.locator(`${rowSel(key)} [data-act="${act}"]`));
  };

  await step("app view: rows carry project icons, ⌘1-9 follow the visible rows", async () => {
    await showSidebar();
    const marks = await page.locator("#thread-list .thread-row .monogram").allTextContents();
    check(marks.length === 2 && marks.includes("AA") && marks.includes("BA"), `monograms: ${marks.join(", ")}`);
    check((await page.locator("#chat-head .crumb .monogram").count()) === 1 && (await crumbProject()) === "alpha", "the header's project has no icon");
    const order = await sideOrder();
    if (touch) return order.join(", ");
    await page.keyboard.down("Meta");
    await waitFor("the jump hints", () => page.evaluate(() => "jumpHints" in document.body.dataset), 2000);
    const hints = await page.locator("#thread-list .thread-row").evaluateAll((rows) => rows.map((r) => getComputedStyle(r, "::after").content));
    check(hints.join(" ") === `"⌘1" "⌘2"`, `hints: ${hints.join(" ")}`);
    for (const n of [1, 2]) {
      await page.keyboard.press(String(n));
      await waitFor(`⌘${n} to open ${order[n - 1]}`, async () => (await crumbProject()) === order[n - 1]);
    }
    await page.keyboard.up("Meta");
    check(!(await page.evaluate(() => "jumpHints" in document.body.dataset)), "the hints stayed after ⌘ was let go");
    await page.fill("#thread-search", order[1]);
    await page.keyboard.press("Meta+1");
    await waitFor("⌘1 to open the only row the search left", async () => (await crumbProject()) === order[1]);
    await page.fill("#thread-search", "");
    await openRow(await keyFor("alpha"));
    return order.join(", ");
  });

  await step("app view: pin and settle, with undo, hover controls and shelves", async () => {
    const alpha = await keyFor("alpha");
    const beta = await keyFor("beta");
    const pinned = async () => (await page.locator(`${rowSel(alpha)} .pin`).count()) === 1;
    await openRow(alpha);
    await showSidebar();
    if (touch) {
      await rowMenu(rowSel(alpha));
      await menuPick("Pin thread");
    } else await page.keyboard.press("Meta+Shift+P");
    await waitFor("alpha pinned on top", async () => (await pinned()) && (await sideOrder())[0] === "alpha");
    await press(page.locator(`${rowSel(alpha)} .pin`));
    await waitFor("the pin to come off", async () => !(await pinned()));
    check(/Unpinned 1 thread/.test(await text("#side-notice")), `notice: ${await text("#side-notice")}`);
    if (touch) await press(gui("#side-notice [data-undo]"));
    else {
      await page.focus("#prompt");
      await page.keyboard.press("Meta+z");
    }
    await waitFor("the undo to pin it back", pinned);
    await press(page.locator(`${rowSel(alpha)} .pin`));
    await waitFor("unpinned", async () => !(await pinned()));

    if (touch) {
      await rowMenu(rowSel(beta));
      await menuPick("Settle thread");
    } else await rowAct(beta, "settle");
    await waitFor("the settled shelf", async () => (await text('#thread-list .settled-head[data-shelf="settled"]')) === "Settled (1)");
    check(!(await page.locator(rowSel(beta)).count()), "beta shows while the shelf is collapsed");
    await press(gui('#thread-list .settled-head[data-shelf="settled"]'));
    await waitFor("beta's slim row", () => page.locator(`${rowSel(beta)}.slim`).isVisible());
    if (touch) {
      await rowMenu(rowSel(beta));
      await menuPick("Un-settle thread");
    } else await rowAct(beta, "unsettle");
    await waitFor("beta back among the cards", () => page.locator(`${rowSel(beta)}.tall`).isVisible());

    if (touch) {
      await rowMenu(rowSel(alpha));
      await menuPick("Settle thread");
    } else await page.keyboard.press("Meta+Shift+S");
    await waitFor("settling alpha to open beta", async () => (await crumbProject()) === "beta");
    await showSidebar();
    await waitFor("alpha in the settled shelf", () => page.locator(`${rowSel(alpha)}.slim`).isVisible());
    await rowMenu(rowSel(alpha));
    await menuPick("Un-settle thread");
    await waitFor("alpha back among the cards", () => page.locator(`${rowSel(alpha)}.tall`).isVisible());
    await openRow(alpha);
  });

  await step("app view: the row menu renames, snoozes, marks unread and copies", async () => {
    const alpha = await keyFor("alpha");
    const beta = await keyFor("beta");
    await rowMenu(rowSel(alpha));
    const labels = await page.locator("#side-menu button .label").allTextContents();
    const want = ["Pin thread", "Settle thread", "Snooze", "Rename thread", "Regenerate title", "Mark unread", "Filter by alpha", "Auto-settle behavior", "Copy", "Project settings", "Archive thread", "Delete"];
    check(want.every((l) => labels.includes(l)), `menu: ${labels.join(" | ")}`);
    await menuPick("Rename thread");
    await page.fill(`${rowSel(alpha)} input.rename`, "Renamed in the sidebar");
    await page.press(`${rowSel(alpha)} input.rename`, "Enter");
    await waitFor("the new title in the row and the header", async () =>
      (await text(`${rowSel(alpha)} .l2`)) === "Renamed in the sidebar" && (await text("#chat-head .crumb .title")) === "Renamed in the sidebar");
    if (!touch) {
      await page.dblclick("#chat-head .crumb .title");
      await page.fill("#chat-head input.rename", "");
      await page.press("#chat-head input.rename", "Enter");
      check(/cannot be empty/.test(await text("#side-notice")), `an empty title gave: ${await text("#side-notice")}`);
      await page.fill("#chat-head input.rename", "Renamed in the header");
      await page.press("#chat-head input.rename", "Enter");
      await waitFor("the header's title in the row", async () => (await text(`${rowSel(alpha)} .l2`)) === "Renamed in the header");
    }

    await rowMenu(rowSel(beta));
    await menuPick("Snooze");
    await menuPick("In 1 hour");
    await waitFor("the snoozed shelf", async () => (await text('#thread-list .settled-head[data-shelf="snoozed"]')) === "Snoozed (1)");
    await press(gui('#thread-list .settled-head[data-shelf="snoozed"]'));
    await waitFor("beta's wake time", async () => /^\d+m$/.test(await text(`${rowSel(beta)} .wake-at`)));
    const wakeAt = await text(`${rowSel(beta)} .wake-at`);
    if (touch) {
      await rowMenu(rowSel(beta));
      await menuPick("Wake thread");
    } else await rowAct(beta, "wake");
    await waitFor("beta awake", () => page.locator(`${rowSel(beta)}.tall`).isVisible());
    await page.evaluate((k) => snooze(tabByKey(k), Date.now() / 1000 + 1.5), beta);
    await waitFor("Woke when the snooze ran out", async () => /Woke/.test(await text(`${rowSel(beta)} .stamp`)), 6000);
    await press(page.locator(`${rowSel(beta)} [data-act="dismiss"]`));
    await waitFor("Woke dismissed", async () => !/Woke/.test(await text(`${rowSel(beta)} .slot`)));

    await openRow(beta);
    await rowMenu(rowSel(alpha));
    await menuPick("Mark unread");
    await waitFor("alpha's time marked unread", () => page.locator(`${rowSel(alpha)} .stamp.when.unread`).count());
    check((await tone(`${rowSel(alpha)} .stamp.when`)) === "rgb(254, 154, 0)", `the unread time is ${await tone(`${rowSel(alpha)} .stamp.when`)}`);
    await rowMenu(rowSel(alpha));
    await menuPick("Copy");
    await menuPick("Path");
    await waitFor("the copy notice", async () => /Copied path|Couldn't copy path/.test(await text("#side-notice")));
    const cwd = await page.evaluate((k) => tabByKey(k).view.cwd, alpha);
    if (engine === "chromium") check((await page.evaluate(() => navigator.clipboard.readText())) === cwd, "the clipboard lacks alpha's path");
    await openRow(alpha);
    if (!touch) {
      await page.keyboard.press("Meta+Shift+C");
      await waitFor("the PR link notice", async () => /Copied PR link/.test(await text("#side-notice")));
      if (engine === "chromium") check((await page.evaluate(() => navigator.clipboard.readText())) === "https://github.com/example/alpha/pull/42", "⌘⇧C didn't copy the thread's PR link");
      await openRow(beta);
      await page.keyboard.press("Meta+Shift+C");
      await waitFor("the thread ID notice", async () => /Copied thread ID/.test(await text("#side-notice")));
      const session = await page.evaluate(() => threadOf(active).session);
      if (engine === "chromium") check((await page.evaluate(() => navigator.clipboard.readText())) === session, "⌘⇧C didn't copy beta's thread ID");
      await openRow(alpha);
    }
    return `snoozed for ${wakeAt}`;
  });

  await step("app view: search finds message text, steps with arrows, clears", async () => {
    await showSidebar();
    await page.fill("#thread-search", "hello from the app");
    await waitFor("a message match", async () => (await page.locator("#thread-list .thread-row.result mark").count()) === 1);
    check((await text("#thread-list .thread-row.result mark")) === "hello from the app", `marked: ${await text("#thread-list .thread-row.result mark")}`);
    check(await page.locator("#search-clear").isVisible(), "no clear button while searching");
    await tapOrClick("#search-clear");
    check((await page.inputValue("#thread-search")) === "" && !(await page.locator("#search-clear").isVisible()), "the clear button left the search");
    check((await page.locator("#thread-list .thread-row.tall").count()) === 2, "the rows didn't come back");
    if (touch) return "touch";
    await page.fill("#thread-search", "a");
    await waitFor("two results", async () => (await page.locator("#thread-list .thread-row.result").count()) === 2);
    const second = await page.evaluate(() => projectName(tabByKey(document.querySelectorAll("#thread-list .thread-row.result")[1].dataset.tab)));
    await page.press("#thread-search", "ArrowDown");
    check(await page.locator("#thread-list .thread-row.result").nth(1).evaluate((r) => r.classList.contains("hit")), "ArrowDown didn't move to the second result");
    await page.press("#thread-search", "Enter");
    await waitFor("Enter to open it", async () => (await crumbProject()) === second && (await page.inputValue("#thread-search")) === "");
    await openRow(await keyFor("alpha"));
  });

  await step("app view: new thread here, the header's project, reopen with ⌘⇧T, add project", async () => {
    const rows = () => page.locator("#thread-list .thread-row").count();
    await openRow(await keyFor("alpha"));
    if (touch) {
      await rowMenu(rowSel(await keyFor("alpha")));
      await menuPick("New thread");
    } else await page.click("#new-thread", { modifiers: ["Shift"] });
    await waitFor("a third thread", async () => (await rows()) === 3, 10000);
    check(await page.locator("#projects").isHidden(), "the picker opened");
    const header = async () => `${await crumbProject()} / ${await text("#chat-head .crumb .title")}`;
    await waitFor("the new thread in alpha", async () => (await header()) === "alpha / New thread").catch(async (e) => check(false, `${e.message}: the header reads ${await header()}`));
    await waitTerm(/fake claude in .*work\/alpha/);
    await sendPrompt("before closing");
    await waitFor("its answer", async () => (await page.locator("#thread .assistant", { hasText: "echo: before closing" }).count()) > 0);
    const session = await page.evaluate(() => threadOf(active).session);
    if (touch) {
      await rowMenu("#thread-list .thread-row.active");
      await menuPick("Delete");
      await press(gui("#side-dialog .destructive"));
    } else await page.keyboard.press("Meta+w");
    await waitFor("two threads", async () => (await rows()) === 2);
    if (touch) return "touch: ⌘⇧T and the header's project need a keyboard and a wide screen";
    await page.keyboard.press("Meta+Shift+T");
    await waitFor("the reopened thread", async () => (await rows()) === 3, 10000);
    await waitTerm(new RegExp(`resumed ${session}`));
    await waitFor("its conversation", async () => (await page.locator("#thread .bubble", { hasText: "before closing" }).count()) > 0);
    await page.keyboard.press("Meta+w");
    await waitFor("two threads", async () => (await rows()) === 2);

    await openRow(await keyFor("alpha"));
    await page.click("#chat-head .crumb [data-new-here]");
    await waitFor("a thread from the header", async () => (await rows()) === 3, 10000);
    await waitFor("the header's new thread to open", async () => (await text("#chat-head .crumb .title")) === "New thread", 10000);
    check((await crumbProject()) === "alpha", `the header opened a thread in ${await crumbProject()}`);
    await page.keyboard.press("Meta+w");
    await waitFor("two threads", async () => (await rows()) === 2);

    await tapOrClick("#add-project");
    await waitFor("the path field", () => page.locator("#projects .browse input").isVisible());
    await page.keyboard.press("Escape");
    await page.keyboard.press("Escape");
    await waitFor("the picker to close", () => page.locator("#projects").isHidden());
    await openRow(await keyFor("alpha"));
  });

  await step("app view: drop a file on a row, resize the sidebar", async () => {
    const beta = await keyFor("beta");
    await showSidebar();
    await page.evaluate((k) => {
      const row = document.querySelector(`#thread-list .thread-row[data-tab="${k}"]`);
      const data = new DataTransfer();
      data.items.add(new File(["dropped"], "dropped-on-row.txt", { type: "text/plain" }));
      for (const type of ["dragover", "drop"]) row.dispatchEvent(new DragEvent(type, { dataTransfer: data, bubbles: true, cancelable: true }));
    }, beta);
    await waitFor("beta to open", async () => (await crumbProject()) === "beta");
    await waitFor("the file's chip in the composer", async () => (await page.locator("#attachments .name", { hasText: "dropped-on-row.txt" }).count()) === 1, 10000);
    await openRow(beta);
    await press(page.locator("#attachments .remove"));
    await openRow(await keyFor("alpha"));
    if (touch) return "touch: the sidebar is a full-width sheet";
    await showSidebar();
    const rail = await rect("#sidebar-resize");
    const left = (await rect("#sidebar")).left;
    await page.mouse.move(rail.left + rail.width / 2, 300);
    await page.mouse.down();
    await page.mouse.move(rail.left + rail.width / 2 + 100, 300, { steps: 8 });
    await page.mouse.up();
    const want = Math.round(rail.left + rail.width / 2 + 100 - left);
    const width = Math.round((await rect("#sidebar")).width);
    check(width === want, `the sidebar is ${width}px after a drag to ${want}`);
    check((await page.evaluate(() => load("gui-sidebar-width"))) === String(want), "the width wasn't saved");
    await page.dblclick("#sidebar-resize");
    check(Math.round((await rect("#sidebar")).width) === 256, "double-click didn't reset the width");
    return `${want}px, then 256px`;
  });

  // T3 Code's sidebar, round 2: PR state everywhere, auto-settle and its settings, the Working shelf.
  const attr = (selector, name) => page.evaluate(([sel, n]) => document.querySelector(sel)?.getAttribute(n) ?? null, [selector, name]);
  const prState = (selector) => attr(selector, "data-state");
  const sideSetting = async (id, on) => {
    await page.evaluate(() => document.body.dataset.page === "settings" || toggleSettings());
    await tapOrClick('#settings-nav [data-page="general"]');
    if (on !== undefined) await page.locator(`#side-${id}`).setChecked(on);
  };
  const closeSettings = () => page.evaluate(() => document.body.dataset.page === "settings" && toggleSettings());
  const sectionIn = (key, days) =>
    page.evaluate(([k, d]) => {
      const real = Date.now;
      Date.now = () => real() + d * 86400e3;
      try {
        return sectionOf(tabByKey(k));
      } finally {
        Date.now = real;
      }
    }, [key, days]);

  await step("app view: PR badges take their state's color, with T3's hover cards", async () => {
    const alpha = await keyFor("alpha");
    await openRow(alpha);
    await showSidebar();
    const badge = `${rowSel(alpha)} .pr`;
    await waitFor("alpha's badge to read merged", async () => (await prState(badge)) === "merged", 10000);
    check((await tone(badge)) === "rgba(196, 181, 253, 0.9)", `the merged badge is ${await tone(badge)}`);
    check((await attr(badge, "aria-label")) === "PR #42 merged", `aria-label: ${await attr(badge, "aria-label")}`);
    await waitFor("the composer's badge to read merged", async () => (await prState("#composer-footer .pr")) === "merged");
    check((await tone("#composer-footer .pr")) === "rgba(196, 181, 253, 0.9)", `the composer's badge is ${await tone("#composer-footer .pr")}`);
    check((await attr(rowSel(alpha), "title")) === null, "the row still has a native tooltip");
    if (touch) return "touch: hover cards need a pointer";
    await page.hover(`${rowSel(alpha)} .l2`);
    await waitFor("the row's hover card", () => page.locator("#hover-card").isVisible(), 3000);
    const card = await text("#hover-card");
    const title = await text(`${rowSel(alpha)} .l2`);
    check(card.startsWith(title) && /alpha/.test(card) && /sweep-node/.test(card) && /#42/.test(card), `row card: ${card}`);
    const [row, box] = [await rect(rowSel(alpha)), await rect("#hover-card")];
    check(Math.round(box.left) === Math.round(row.right + 4) && box.width <= 320, `the card sits at ${box.left},${box.top} (${box.width}px), the row ends at ${row.right}`);
    await page.hover(badge);
    await waitFor("the PR's hover card", async () => /#42\s*Add notes/.test(await text("#hover-card")), 3000);
    check((await rect("#hover-card")).width === 320, "the PR card isn't 320px wide");
    await page.click('#hover-card [data-card-pr$="/pull/42"]');
    await waitFor("#42 in the PR panel", async () => /#42/.test(await text('#panel-tabs [data-panel="pr"].active')), 15000);
    check(!(await page.locator("#hover-card").isVisible()), "the card stayed after its link was clicked");
    await tapOrClick("#toggle-panel");
    await page.hover("#composer-footer .pr");
    await waitFor("the composer badge's card", async () => /#42\s*Add notes/.test(await text("#hover-card")), 3000);
    await page.mouse.move(0, 0);
    await waitFor("the card to close", async () => !(await page.locator("#hover-card").isVisible()), 2000);
    return card;
  });

  await step("app view: a draft's pen leads its row and Discard draft clears it; the empty search state", async () => {
    const alpha = await keyFor("alpha");
    const beta = await keyFor("beta");
    await openRow(alpha);
    await page.fill("#prompt", "unsent words");
    await openRow(beta);
    await showSidebar();
    const lead = await page.locator(`${rowSel(alpha)} .l1 > *`).first().getAttribute("class");
    check(lead === "draft-pen", `line 1 starts with ${lead}`);
    check(!(await page.locator(`${rowSel(alpha)} .l2 .draft-pen`).count()), "the pen is still on the title line");
    if (!touch) {
      await rowAct(alpha, "discard");
      await waitFor("the draft gone", async () => !(await page.locator(`${rowSel(alpha)} .draft-pen`).count()));
      await openRow(alpha);
      check((await page.inputValue("#prompt")) === "", `the discarded draft came back: ${await page.inputValue("#prompt")}`);
    } else {
      await openRow(alpha);
      await page.fill("#prompt", "");
    }
    await showSidebar();
    await page.fill("#thread-search", "nothing matches this");
    const empty = await page.locator("#thread-list .empty").evaluate((el) => [el.textContent, getComputedStyle(el).fontSize, getComputedStyle(el).textAlign, getComputedStyle(el).paddingTop].join("|"));
    check(empty === "No threads found|12px|center|24px", `empty state: ${empty}`);
    await page.fill("#thread-search", "");
    await openRow(alpha);
  });

  await step("app view: a project's favicon replaces its monogram, found where T3 looks", async () => {
    const beta = await keyFor("beta");
    const file = path.join(HOME, "work/beta/favicon.svg");
    fs.writeFileSync(file, '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" fill="#e11d48"/></svg>');
    await page.evaluate(() => dispatchEvent(new Event("focus")));
    await showSidebar();
    await waitFor("beta's favicon", async () => ((await attr(`${rowSel(beta)} img.favicon`, "src")) ?? "").startsWith("data:image/svg+xml;base64,"), 10000);
    check(!(await page.locator(`${rowSel(beta)} .monogram`).count()), "beta kept its monogram");
    check((await page.locator(`${rowSel(await keyFor("alpha"))} .monogram`).count()) === 1, "alpha, with no favicon, lost its monogram");
    const size = await page.locator(`${rowSel(beta)} img.favicon`).evaluate((el) => `${el.getBoundingClientRect().width}x${el.getBoundingClientRect().height}`);
    check(size === "16x16", `the favicon is ${size}`);
    fs.unlinkSync(file);
    await page.evaluate(() => dispatchEvent(new Event("focus")));
    await waitFor("the monogram back", async () => (await page.locator(`${rowSel(beta)} .monogram`).count()) === 1, 10000);
  });

  await step("app view: the project filter is T3's searchable combobox, each project with its settings gear", async () => {
    await showSidebar();
    check((await text("#project-filter .label")) === "All projects", `the filter reads ${await text("#project-filter .label")}`);
    await tapOrClick("#project-filter");
    await waitFor("the project menu", () => page.locator("#project-menu").isVisible());
    check((await attr("#project-menu input", "placeholder")) === "Search projects...", "the menu has no project search");
    const items = await page.locator("#project-menu .item .label").allTextContents();
    check(items.join("|") === "All projects|alpha|beta", `items: ${items.join("|")}`);
    const [menu, search] = [await rect("#project-menu"), await rect("#sidebar .search")];
    check(Math.round(menu.left) === Math.round(search.left) && menu.width <= 288, `the menu sits at ${menu.left} (${menu.width}px), the search at ${search.left}`);
    await page.fill("#project-menu input", "nothing like it");
    check((await text("#project-menu .empty")) === "No matching projects.", `empty: ${await text("#project-menu .empty")}`);
    await page.fill("#project-menu input", "bet");
    if (touch) await press(gui('#project-menu .item[data-project="beta"]'));
    else await page.press("#project-menu input", "Enter");
    await waitFor("only beta's rows", async () => (await sideOrder()).every((p) => p === "beta") && (await text("#project-filter .label")) === "beta");
    check((await page.evaluate(() => load("gui-project"))) === "beta", "the filter wasn't saved");
    await tapOrClick("#project-filter");
    await press(gui('#project-menu [data-gear="beta"]'));
    await waitFor("beta's project page", async () => (await text("#settings-page h2")) === "beta", 10000);
    await closeSettings();
    await showSidebar();
    await tapOrClick("#project-filter");
    await press(gui('#project-menu .item[data-project=""]'));
    await waitFor("all projects again", async () => (await sideOrder()).length === 2 && (await text("#project-filter .label")) === "All projects");
    await openRow(await keyFor("alpha"));
  });

  await step("app view: the row menu regenerates a title, snoozes for a duration, opens project settings, archives and deletes", async () => {
    const alpha = await keyFor("alpha");
    const beta = await keyFor("beta");
    const rows = () => page.locator("#thread-list .thread-row").count();
    await rowMenu(rowSel(alpha));
    await menuPick("Regenerate title");
    await waitFor("claude's new title", async () => /^Titled /.test(await text(`${rowSel(alpha)} .l2`)), 10000);

    await rowMenu(rowSel(beta));
    await menuPick("Snooze");
    await menuPick("Custom…");
    await waitFor("the custom snooze dialog", () => page.locator("#side-dialog").isVisible());
    check((await text("#side-dialog h3")) === "Custom snooze", `dialog: ${await text("#side-dialog h3")}`);
    await press(gui('#side-dialog [data-mode="duration"]'));
    await page.fill('#side-dialog [name="amount"]', "0");
    await press(gui("#side-dialog .primary"));
    check((await text("#side-dialog .error")) === "Enter a positive duration.", `error: ${await text("#side-dialog .error")}`);
    await page.fill('#side-dialog [name="amount"]', "3");
    await press(gui("#side-dialog .primary"));
    await waitFor("beta snoozed for 3 hours", async () => (await text(`${rowSel(beta)} .wake-at`)) === "3h");
    await rowMenu(rowSel(beta));
    await menuPick("Wake thread");
    await waitFor("beta awake", async () => !(await page.locator(`${rowSel(beta)} .wake-at`).count()));

    await rowMenu(rowSel(alpha));
    await menuPick("Project settings");
    await waitFor("alpha's project page", async () => (await text("#settings-page h2")) === "alpha", 10000);
    await closeSettings();

    await rowMenu(rowSel(alpha));
    await menuPick("New thread on");
    await waitFor("a third thread", async () => (await rows()) === 3, 10000);
    await waitFor("it to open", async () => (await text("#chat-head .crumb .title")) === "New thread", 10000);
    const third = await page.evaluate(() => active.key);
    await sendPrompt("to archive");
    await answered("echo: to archive");
    await rowMenu(rowSel(third));
    await menuPick("Archive thread");
    await waitFor("the archived thread gone", async () => (await rows()) === 2);
    await sideSetting("days");
    await waitFor("it under Archived threads", async () => /to archive/.test(await text("#side-settings .archived")));
    await press(gui("#side-settings [data-unarchive]"));
    await waitFor("Unarchive to bring it back", async () => (await rows()) === 3 && (await text("#chat-head .crumb .title")) === "to archive", 15000);
    await waitFor("its conversation", async () => (await page.locator("#thread .bubble", { hasText: "to archive" }).count()) > 0, 10000);
    await rowMenu("#thread-list .thread-row.active");
    await menuPick("Delete");
    await waitFor("the delete dialog", async () => (await text("#side-dialog h3")) === 'Delete thread "to archive"?');
    await press(gui("#side-dialog .destructive"));
    await waitFor("the deleted thread gone", async () => (await rows()) === 2);
    await sideSetting("days");
    check(/No archived threads/.test(await text("#side-settings")), "the archive isn't empty");
    await closeSettings();
    await openRow(alpha);
  });

  await step("app view: ⌘-click and Shift-click select rows for T3's bulk menu", async () => {
    if (touch) return "touch: selecting takes a modifier key";
    const alpha = await keyFor("alpha");
    const beta = await keyFor("beta");
    await openRow(alpha);
    await showSidebar();
    const picked = () => page.locator("#thread-list .thread-row.selected").count();
    await page.click(`${rowSel(beta)} .l2`, { modifiers: ["Meta"] });
    await page.click(`${rowSel(alpha)} .l2`, { modifiers: ["Meta"] });
    check((await picked()) === 2 && (await crumbProject()) === "alpha", `${await picked()} selected, ${await crumbProject()} open`);
    await page.click(rowSel(alpha), { button: "right" });
    const labels = await page.locator("#side-menu button .label").allTextContents();
    for (const want of ["Settle (2)", "Snooze (2)", "Regenerate titles (2)", "Mark unread (2)", "Delete (2)"]) check(labels.includes(want), `bulk menu: ${labels.join(" | ")}`);
    await menuPick("Settle (2)");
    await waitFor("both settled", async () => (await page.locator("#thread-list .thread-row.slim").count()) === 2);
    check((await picked()) === 0, "the selection stayed after the bulk action");
    check(/Settled 2 threads/.test(await text("#side-notice")), `notice: ${await text("#side-notice")}`);
    await page.keyboard.press("Meta+z");
    await waitFor("both back", async () => (await page.locator("#thread-list .thread-row.tall").count()) === 2);
    await page.click(`${rowSel(alpha)} .l2`);
    await page.click(`${rowSel(beta)} .l2`, { modifiers: ["Shift"] });
    check((await picked()) === 2, `Shift-click selected ${await picked()}`);
    await page.keyboard.press("Escape");
    await waitFor("Esc to clear the selection", async () => (await picked()) === 0);
    return labels.join(" | ");
  });

  await step("app view: ⌘⇧T reopens a closed panel tab, as T3's closed views", async () => {
    if (touch) return "touch: ⌘⇧T needs a keyboard";
    await openRow(await keyFor("alpha"));
    await tapOrClick("#pr-chip");
    await waitFor("the PR tab", () => page.locator('#panel-tabs [data-panel="pr"].active').isVisible(), 10000);
    const label = await text('#panel-tabs [data-panel="pr"].active');
    await page.keyboard.press("Meta+w");
    await waitFor("the PR tab closed", async () => !(await page.locator('#panel-tabs [data-panel="pr"]', { hasText: label }).filter({ visible: true }).count()));
    await page.keyboard.press("Meta+Shift+T");
    await waitFor("⌘⇧T to bring it back", async () => (await page.locator('#panel-tabs [data-panel="pr"].active').isVisible()) && (await text('#panel-tabs [data-panel="pr"].active')) === label, 5000);
    await tapOrClick("#toggle-panel");
    return label;
  });

  await step("app view: settings: auto-settle inactive threads after the set days, the Working section", async () => {
    const beta = await keyFor("beta");
    await sideSetting("days");
    check((await page.inputValue("#side-days")) === "3", `days: ${await page.inputValue("#side-days")}`);
    check((await sectionIn(beta, 2)) === "active" && (await sectionIn(beta, 4)) === "settled", "3 days don't settle beta after 4");
    await page.fill("#side-days", "5");
    check((await sectionIn(beta, 4)) === "active" && (await sectionIn(beta, 6)) === "settled", "5 days still settle beta after 4");
    await page.fill("#side-days", "91");
    check(JSON.parse(await page.evaluate(() => load("gui-sidebar-settings"))).days === 5, "91 days was taken");
    await page.locator("#side-inactive").setChecked(false);
    await waitFor("the days row to go", async () => !(await page.locator("#side-days").count()) && !(await page.locator("#side-keepPrs").count()));
    check((await sectionIn(beta, 100)) === "active", "inactivity still settles with the switch off");
    await page.locator("#side-inactive").setChecked(true);
    await waitFor("3 days again", async () => (await page.locator("#side-days").count()) && (await page.inputValue("#side-days")) === "3");

    await page.locator("#side-working").setChecked(true);
    await closeSettings();
    const alpha = await keyFor("alpha");
    await openRow(alpha);
    await sendPrompt("think");
    await waitFor("the Working shelf", async () => (await text('#thread-list .settled-head[data-shelf="working"]')) === "Working (1)", 8000);
    await showSidebar();
    check(await page.locator(`#thread-list .settled-head[data-shelf="working"] ~ .thread-row[data-tab="${alpha}"].tall`).isVisible(), "the open thread isn't under the Working shelf");
    await answered("Done thinking.");
    await waitFor("alpha back on top of the inbox", async () => (await sideOrder())[0] === "alpha" && !(await page.locator('.settled-head[data-shelf="working"]').count()), 10000);
    await sideSetting("working", false);
    await closeSettings();
  });

  await step("app view: a thread settles when its pull request closes, or merges with on-merge on", async () => {
    const beta = await keyFor("beta");
    await openRow(beta);
    await sendPrompt("open-pr");
    await answered("I opened a pull request.");
    const badge = `${rowSel(beta)} .pr`;
    await waitFor("beta's open #43", async () => (await prState(badge)) === "open", 10000);
    check((await tone(badge)) === "rgba(110, 231, 183, 0.9)", `the open badge is ${await tone(badge)}`);
    check((await sectionIn(beta, 30)) === "active", "an open PR let inactivity settle beta");
    fs.writeFileSync(path.join(HOME, ".fake-gh-43"), `closed ${new Date().toISOString()}`);
    await page.evaluate(() => dispatchEvent(new Event("focus")));
    await showSidebar();
    await waitFor("beta settled by the closed PR", () => page.locator(`${rowSel(beta)}.slim`).isVisible(), 10000);
    check((await prState(badge)) === "closed", "the settled row's badge isn't closed");
    check((await crumbProject()) === "beta", "auto-settle moved off the open thread");
    await rowMenu(rowSel(beta));
    await menuPick("Un-settle thread");
    await waitFor("beta back among the cards", () => page.locator(`${rowSel(beta)}.tall`).isVisible());

    await sideSetting("onMerge", false);
    await closeSettings();
    fs.writeFileSync(path.join(HOME, ".fake-gh-43"), `merged ${new Date().toISOString()}`);
    await page.evaluate(() => dispatchEvent(new Event("focus")));
    await waitFor("beta's merged badge", async () => (await prState(badge)) === "merged", 10000);
    await showSidebar();
    check(await page.locator(`${rowSel(beta)}.tall`).isVisible(), "a merge settled beta with on-merge off");
    await sideSetting("keepPrs", true);
    check((await sectionIn(beta, 10)) === "active", "Keep threads with a PR let inactivity settle beta");
    await page.locator("#side-keepPrs").setChecked(false);
    check((await sectionIn(beta, 10)) === "settled", "inactivity didn't settle beta without Keep threads with a PR");
    await page.locator("#side-onMerge").setChecked(true);
    await closeSettings();
    await showSidebar();
    await waitFor("on-merge to settle beta", () => page.locator(`${rowSel(beta)}.slim`).isVisible(), 5000);
    await rowMenu(rowSel(beta));
    await menuPick("Un-settle thread");
    await waitFor("beta back among the cards", () => page.locator(`${rowSel(beta)}.tall`).isVisible());
    await openRow(await keyFor("alpha"));
  });

  await step("app view: dragging rows reorders, pins, unpins, settles and un-settles, and Settle sweeps over rows", async () => {
    if (touch) return "touch: dragging rows takes a mouse";
    await openRow(await keyFor("alpha"));
    await showSidebar();
    const read = (selector) => page.evaluate((sel) => document.querySelector(sel)?.textContent.trim() ?? "", selector);
    const mid = async (selector) => {
      const r = await rect(selector);
      return { x: r.left + r.width / 2, y: r.top + r.height / 2, top: r.top, bottom: r.bottom };
    };
    const drag = async (selector, targetY) => {
      const from = await mid(selector);
      await page.mouse.move(from.x, from.y);
      await page.mouse.down();
      await page.mouse.move(from.x, from.y - 8, { steps: 2 });
      await page.mouse.move(from.x, await targetY(), { steps: 6 });
      const verb = await read("#thread-list .thread-row.dragging .drop-verb");
      await page.mouse.up();
      return verb;
    };
    const keys = () => page.evaluate(() => [...document.querySelectorAll("#thread-list .thread-row")].map((r) => r.dataset.tab));
    const pinned = (key) => page.locator(`${rowSel(key)} .pin`).count();
    const [first, second] = await keys();
    let verb = await drag(rowSel(second), async () => (await mid('.drag-mark[data-mark="pinned"]')).y + 4);
    check(verb === "Pin", `dragging onto Pinned reads ${verb}`);
    await waitFor("the dragged row pinned on top", async () => (await pinned(second)) === 1 && (await keys())[0] === second);
    verb = await drag(rowSel(second), async () => (await mid(rowSel(first))).bottom - 4);
    check(verb === "Unpin", `dragging below Active reads ${verb}`);
    await waitFor("unpinned below", async () => !(await pinned(second)) && (await keys()).join() === [first, second].join());
    verb = await drag(rowSel(first), async () => (await mid(rowSel(second))).bottom - 4);
    check(verb === "", `a reorder shows ${verb}`);
    await waitFor("the rows reordered", async () => (await keys()).join() === [second, first].join());
    verb = await drag(rowSel(first), async () => (await mid('.drag-hint[data-mark="settled"]')).y);
    check(verb === "Settle", `dragging onto Settled reads ${verb}`);
    await waitFor("the dragged row settled", () => page.evaluate((k) => sectionOf(tabByKey(k)) === "settled", first));
    if (!(await page.locator(`${rowSel(first)}.slim`).count())) await press(gui('#thread-list .settled-head[data-shelf="settled"]'));
    verb = await drag(rowSel(first), async () => (await mid(rowSel(second))).top + 4);
    check(verb === "Un-settle", `dragging a settled row up reads ${verb}`);
    await waitFor("it back among the cards", () => page.locator(`${rowSel(first)}.tall`).isVisible());

    const [top, below] = await keys();
    await page.hover(`${rowSel(top)} .l2`);
    const settleButton = await mid(`${rowSel(top)} [data-act="settle"]`);
    await page.mouse.move(settleButton.x, settleButton.y);
    await page.mouse.down();
    await page.mouse.move(settleButton.x, settleButton.y + 12, { steps: 2 });
    await page.mouse.move(settleButton.x, (await mid(rowSel(below))).y, { steps: 6 });
    const swept = await page.locator("#thread-list .thread-row.swept .drop-verb").allTextContents();
    await page.mouse.up();
    check(swept.join() === "Settle,Settle", `the sweep marked ${swept.join() || "nothing"}`);
    await waitFor("both settled", async () => (await page.locator("#thread-list .thread-row.slim").count()) === 2);
    // The drag's settle a moment ago joins the same undo, as repeats of one action do.
    check(/Settled [23] threads/.test(await text("#side-notice")), `notice: ${await text("#side-notice")}`);
    await page.keyboard.press("Meta+z");
    await waitFor("the sweep undone", async () => (await page.locator("#thread-list .thread-row.tall").count()) === 2);
    await openRow(await keyFor("alpha"));
  });

  await step("app view: controls are big enough to tap", async () => {
    if (!touch) return "desktop";
    const small = await smallTargets("#chat-head button, #composer button, #composer-footer button");
    touchCheck(!small.length, `small: ${small.join(", ")}`);
  });

  const paletteGroups = () => page.locator("#palette .group .label").allTextContents();
  const paletteItems = () => page.locator("#palette .item .name").allTextContents();
  await step("shell: ⌘K opens the command palette, finds threads, nests pages", async () => {
    if (touch) return "touch: no keyboard";
    const alpha = await keyFor("alpha");
    await openRow(alpha);
    await page.keyboard.press("Meta+k");
    await waitFor("the palette", () => page.locator("#palette").isVisible());
    check(await page.evaluate(() => document.activeElement === paletteInput), "the palette's input has no focus");
    check((await page.getAttribute("#palette input", "placeholder")) === "Search commands, projects, and threads...", "wrong placeholder");
    const groups = await paletteGroups();
    check(groups.join() === "Actions,Recent Threads", `groups: ${groups.join()}`);
    const items = await paletteItems();
    check(items[0] === "New thread in alpha", `first item: ${items[0]}`);
    check(items.includes("Open settings") && items.includes("Find in current thread"), `actions: ${items.join(" | ")}`);
    check(await page.locator("#palette .item.hi").first().evaluate((el) => el.dataset.i === "0"), "the first row isn't highlighted");
    const pop = await rect("#palette .popup");
    check(pop.width <= 576 && pop.height <= 420, `the palette is ${pop.width}x${pop.height}`);
    await page.keyboard.type("beta");
    await waitFor("the Threads group", async () => (await paletteGroups()).includes("Threads"));
    check(!(await paletteGroups()).includes("Recent Threads"), "a query still shows Recent Threads");
    await page.keyboard.press("Enter");
    await waitFor("beta to open from the palette", async () => (await crumbProject()) === "beta");
    check(!(await page.locator("#palette").isVisible()), "the palette stayed open");
    await page.keyboard.press("Meta+k");
    await page.keyboard.type(">sett");
    await waitFor("actions only", async () => (await paletteGroups()).join() === "Actions");
    check((await paletteItems()).join() === "Open settings,Project settings", `> results: ${(await paletteItems()).join(" | ")}`);
    await page.fill("#palette input", "new thread in");
    await waitFor("the sub-page item", async () => (await paletteItems())[0] === "New thread in...");
    await page.keyboard.press("Enter");
    await waitFor("the projects sub-page", async () => (await page.getAttribute("#palette input", "placeholder")) === "Search...");
    const nested = await paletteItems();
    check(nested[0] === "No project" && nested.includes("alpha") && nested.includes("beta"), `sub-page: ${nested.join(" | ")}`);
    check((await text("#palette .item[data-i='1'] .hint")) === "⌘2", "the second row has no ⌘2 hint");
    await page.keyboard.press("Backspace");
    await waitFor("the root again", async () => (await page.inputValue("#palette input")) === "new thread in");
    await page.fill("#palette input", "zzqx");
    await waitFor("the empty state", async () => (await text("#palette .empty")) === "No matching commands, projects, or threads.");
    await page.keyboard.press("Escape");
    check(!(await page.locator("#palette").isVisible()), "Escape left the palette open");
    await page.keyboard.press("Meta+k");
    await page.keyboard.type("find in current");
    await page.keyboard.press("Enter");
    await waitFor("⌘K's Find in current thread", () => page.locator("#thread-find").isVisible());
    await page.keyboard.press("Escape");
    await page.keyboard.press("Meta+k");
    await page.keyboard.press("Meta+k");
    check(!(await page.locator("#palette").isVisible()), "a second ⌘K didn't close the palette");
    return `${items.length} actions and threads`;
  });

  await step("shell: ⌘[ and ⌘] go back and forward between threads", async () => {
    if (touch) return "touch: no keyboard";
    const [alpha, beta] = [await keyFor("alpha"), await keyFor("beta")];
    await openRow(alpha);
    await openRow(beta);
    await page.keyboard.press("Meta+[");
    await waitFor("⌘[ to go back to alpha", async () => (await crumbProject()) === "alpha");
    await page.keyboard.press("Meta+]");
    await waitFor("⌘] to go forward to beta", async () => (await crumbProject()) === "beta");
    await page.keyboard.press("Meta+[");
    await waitFor("alpha again", async () => (await crumbProject()) === "alpha");
    return "alpha, beta, alpha";
  });

  // Priced usage the transcripts recorded: opus today and three days ago, and a model the price list lacks.
  const seedUsage = () => {
    const at = (hoursAgo) => new Date(Date.now() - hoursAgo * 3600 * 1000).toISOString();
    const line = (id, session, hoursAgo, model, usage) => JSON.stringify({ type: "assistant", timestamp: at(hoursAgo), sessionId: session, requestId: `req-${id}`, message: { id, model, role: "assistant", content: [{ type: "text", text: "spent" }], usage } });
    const dir = path.join(HOME, ".claude", "projects", "-sweep-usage");
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, "u1.jsonl"), [
      line("m1", "u1", 2, "claude-opus-5-5", { input_tokens: 1000, cache_read_input_tokens: 2000000, cache_creation_input_tokens: 100000, output_tokens: 50000, speed: "standard" }),
      line("m1", "u1", 2, "claude-opus-5-5", { input_tokens: 1000, cache_read_input_tokens: 2000000, cache_creation_input_tokens: 100000, output_tokens: 50000, speed: "standard" }),
      line("m2", "u1", 2, "claude-mystery-1", { input_tokens: 0, output_tokens: 1000 }),
    ].join("\n") + "\n");
    fs.writeFileSync(path.join(dir, "u2.jsonl"), line("m3", "u2", 72, "claude-opus-5-5", { input_tokens: 500, output_tokens: 10000 }) + "\n");
    fs.writeFileSync(path.join(DIR, "node", "usage-model-rates.json"), JSON.stringify({
      "claude-opus-5-5": { input_cost_per_token: 5e-6, output_cost_per_token: 25e-6, cache_read_input_token_cost: 0.5e-6, cache_creation_input_token_cost: 6.25e-6 },
    }));
  };
  const pageSidebar = async () => {
    if (await page.locator("#sidebar").isVisible()) return;
    await tapOrClick("#page-view .side-toggle");
    await waitFor("the sidebar", () => page.locator("#sidebar").isVisible());
  };
  const usagePick = async (seg, value) => {
    if (touch) await page.selectOption(`#page-view select.seg-select[data-seg="${seg}"]`, value);
    else await page.click(`#page-view .seg[data-seg="${seg}"] button[data-value="${value}"]`);
  };
  const tile = (label) => text(`#page-view .tiles div:has(dt:text-is("${label}")) dd`);
  await step("shell: the Usage page shows limits, cost and tokens from the transcripts", async () => {
    seedUsage();
    await page.evaluate(() => save("gui-usage-page", null));
    await showSidebar();
    await tapOrClick('#sidebar footer [data-open-page="usage"]');
    await waitFor("the Usage page", () => page.locator("#page-view h1", { hasText: "Usage" }).isVisible());
    check(!(await page.locator("#chat").isVisible()), "the thread still shows beside Usage");
    await pageSidebar();
    check(await page.locator("#sidebar footer .back").isVisible(), "the footer has no Back button");
    await waitFor("two limit windows", async () => (await page.locator("#page-view .limit-card").count()) === 2);
    const left = await page.locator("#page-view .limit-card .figure b").allTextContents();
    check(left.join() === "77%,26%", `limits left: ${left.join()}`);
    check(/↻ \d+[hm]/.test(await text('#page-view .limit-card[data-window="five_hour"] .chip')), "the session window has no reset time");
    if (touch) await usagePick("metric", "cost");
    else await page.keyboard.press("c");
    await waitFor("the total cost", async () => (await text("#page-view .total")) === "$3.13", 15000);
    check((await text("#page-view .lead > p")) === "2 sessions · API estimate", `under the total: ${await text("#page-view .lead > p")}`);
    check(/33\.3% unpriced/.test(await page.getAttribute("#page-view .info", "title")), "the unpriced share is missing");
    check((await page.locator("#page-view .x-axis span").count()) === 3, "the chart has no three x labels");
    check((await page.locator("#page-view .plot path.line").getAttribute("d")).startsWith("M"), "the chart has no line");
    const tiles = [await tile("Processed tokens"), await tile("Cached input"), await tile("Uncached input"), await tile("Output"), await tile("Cache savings")];
    check(tiles.join() === "2.16M,2M,1.50K,61K,$9.00", `totals: ${tiles.join()}`);
    const models = await page.locator("#page-view table tbody tr td.model span").allTextContents();
    check(models.join() === "claude-opus-5-5,claude-mystery-1", `models: ${models.join()}`);
    check((await text('#page-view tr[data-model="claude-mystery-1"] td:nth-child(3)')) === "Unpriced", "the unknown model isn't Unpriced");
    if (!touch) {
      const plot = await rect("#page-view .plot");
      await page.mouse.move(plot.left + plot.width - 4, plot.top + plot.height / 2);
      await waitFor("the chart's tooltip", async () => /Total/.test(await text("#page-view .tip")));
      await page.keyboard.press("Meta+Shift+1");
    } else await usagePick("days", "1");
    await waitFor("the past 24 hours", async () => (await text("#page-view .total")) === "$2.88");
    check((await text("#page-view .chart-col h2")) === "Hourly cost", "the chart isn't hourly");
    if (touch) await usagePick("metric", "tokens");
    else await page.keyboard.press("t");
    await waitFor("tokens", async () => (await text("#page-view .total")) === "2.15M");
    await usagePick("breakdown", "day");
    check((await page.locator("#page-view table th").first().textContent()) === "Hour", "the breakdown isn't by hour");
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    check(JSON.parse(await page.evaluate(() => load("gui-usage-page"))).days === 1, "the period wasn't saved");
    if (touch) {
      await pageSidebar();
      await tapOrClick("#sidebar footer .back");
    } else await page.keyboard.press("Escape");
    await waitFor("the thread again", () => page.locator("#chat").isVisible());
    return "limits 77%/26%, $3.13 in 30 days, $2.88 and 2.15M in 24h";
  });

  await step("shell: ⌘U opens Usage, and ⌘[ comes back", async () => {
    if (touch) return "touch: no keyboard";
    const before = await crumbProject();
    await page.keyboard.press("Meta+u");
    await waitFor("⌘U to open Usage", () => page.locator("#page-view h1", { hasText: "Usage" }).isVisible());
    await page.keyboard.press("Meta+[");
    await waitFor("⌘[ to leave Usage", () => page.locator("#chat").isVisible());
    check((await crumbProject()) === before, `back on ${await crumbProject()}, not ${before}`);
    await page.keyboard.press("Meta+]");
    await waitFor("⌘] to return to Usage", () => page.locator("#page-view h1", { hasText: "Usage" }).isVisible());
    await page.keyboard.press("Escape");
    await waitFor("Escape to leave Usage", () => page.locator("#chat").isVisible());
    return before;
  });

  await step("shell: a model's details on Usage, and a price set for an unpriced one", async () => {
    await showSidebar();
    await tapOrClick('#sidebar footer [data-open-page="usage"]');
    await waitFor("the Usage page", () => page.locator("#page-view h1", { hasText: "Usage" }).isVisible());
    await usagePick("metric", "cost");
    await usagePick("days", "30");
    await usagePick("breakdown", "model");
    await waitFor("the 30-day cost", async () => (await text("#page-view .total")) === "$3.13", 15000);
    await press(page.locator('#page-view tr[data-model="claude-mystery-1"]'));
    await waitFor("the model's details", () => page.locator("#shell-dialog .dialog-head", { hasText: "claude-mystery-1" }).isVisible());
    const stat = (label) => text(`#shell-dialog .stats div:has(dt:text-is("${label}")) dd`);
    check((await stat("Cost")) === "Unpriced" && (await stat("Tokens")) === "1K", `stats: ${await stat("Cost")} ${await stat("Tokens")}`);
    check((await text("#shell-dialog .dialog-foot .muted")) === "1K tokens have no known price", "no unpriced note");
    await press(page.locator("#shell-dialog [data-set-price]"));
    await waitFor("the prices dialog", () => page.locator("#shell-dialog table.prices").isVisible());
    check((await page.inputValue('#shell-dialog tbody tr:last-child [data-field="model"]')) === "claude-mystery-1", "the model isn't filled in");
    await page.fill('#shell-dialog tbody tr:last-child [data-field="input"]', "2");
    await page.fill('#shell-dialog tbody tr:last-child [data-field="output"]', "10");
    await press(page.locator("#shell-dialog [data-save]"));
    await waitFor("the priced total", async () => (await text("#page-view .total")) === "$3.14", 15000);
    check(!(await page.locator("#page-view .info").count()), "the unpriced note stayed");
    await press(page.locator('#page-view tr[data-model="claude-mystery-1"]'));
    await waitFor("the priced model", async () => (await stat("Cost")) === "$0.01");
    if (touch) await press(page.locator("#shell-dialog .close"));
    else await page.keyboard.press("Escape");
    check(!(await page.locator("#shell-dialog").isVisible()), "the details stayed open");
    check(await page.locator("#page-view h1", { hasText: "Usage" }).isVisible(), "closing the details left Usage");
    await press(page.locator("#page-view .machine-filter"));
    await press(page.locator("#side-menu button", { hasText: "Model prices" }));
    await press(page.locator("#shell-dialog tbody tr:first-child [data-remove]"));
    await press(page.locator("#shell-dialog [data-save]"));
    await waitFor("the list price again", async () => (await text("#page-view .total")) === "$3.13", 15000);
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    if (touch) {
      await pageSidebar();
      await tapOrClick("#sidebar footer .back");
    } else await page.keyboard.press("Escape");
    await waitFor("the thread again", () => page.locator("#chat").isVisible());
    return "Unpriced, then $0.01 at $2/$10 per 1M";
  });

  const prNumbers = (group) => page.locator(`#page-view ${group ? `.pr-group[data-group="${group}"] ` : ""}.pr-row[data-url] .num`).allTextContents();
  const prFilter = async (kind, label) => {
    await tapOrClick('#page-view [data-pr="filters"]');
    await press(page.locator("#side-menu button", { hasText: kind }));
    await press(page.locator("#side-menu button", { hasText: label }));
  };
  await step("shell: the Pull requests page lists the open projects' pull requests", async () => {
    fs.writeFileSync(path.join(HOME, "gh.log"), "");
    await showSidebar();
    await tapOrClick('#sidebar footer [data-open-page="pull-requests"]');
    await waitFor("the Pull requests page", () => page.locator("#page-view h1", { hasText: "Pull Requests" }).isVisible());
    await waitFor("three groups", async () => (await page.locator("#page-view .pr-group").count()) === 3, 15000);
    const groups = await page.locator("#page-view .pr-group h3 > span:nth-child(2)").allTextContents();
    check(groups.join() === "Authored,Review requested,Others", `groups: ${groups.join()}`);
    const rows = [await prNumbers("authored"), await prNumbers("reviewing"), await prNumbers("others")].map((n) => n.join());
    check(rows.join(" | ") === "#42 | #41 | #43", `rows: ${rows.join(" | ")}`);
    const search = readLog("gh.log").match(/ -f (q=.*)$/m)?.[1];
    check(/^q=is:pr is:open sort:updated-desc repo:example\/\w+ repo:example\/\w+$/.test(search ?? ""), `gh searched: ${search}`);
    const busy = '#page-view .pr-row[data-url$="/pull/43"]';
    check(await page.locator(`${busy} .conflict`).isVisible(), "#43's conflict isn't marked");
    check((await page.getAttribute(`${busy} .checks`, "title")) === "Checks failing", "#43's failing checks aren't shown");
    check((await text(`${busy} .stat`)) === "+120 -30", `#43's size: ${await text(`${busy} .stat`)}`);
    check((await page.locator(`${busy} .label`).count()) === 3 && (await text(`${busy} .more`)) === "+1", "#43 doesn't show three labels and +1");
    check((await page.getAttribute('#page-view .pr-row[data-url$="/pull/42"] .review', "title")) === "Approved", "#42 isn't approved");
    await prFilter("State", "Merged");
    await waitFor("the merged pull request", async () => (await prNumbers()).join() === "#40");
    await prFilter("State", "Open");
    await waitFor("open again", async () => (await prNumbers()).length === 3);
    await prFilter("Involvement", "Authored");
    await waitFor("only the viewer's", async () => (await prNumbers()).join() === "#42" && !(await page.locator("#page-view .pr-group").count()));
    await prFilter("Involvement", "All");
    await page.fill("#page-view .pr-search input", "beta build");
    await waitFor("the search", async () => (await prNumbers()).join() === "#43");
    await page.fill("#page-view .pr-search input", "zzqx");
    await waitFor("nothing matches", async () => (await text("#page-view .pr-empty h2")) === "Nothing matches “zzqx”");
    await press(page.locator('#page-view [data-pr="clear"]'));
    await waitFor("every row again", async () => (await prNumbers()).length === 3);
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    if (!touch) {
      await page.keyboard.down("Shift");
      await page.hover('#page-view .pr-row[data-url$="/pull/41"]');
      await press(page.locator('#page-view .pr-row[data-url$="/pull/41"] [data-quick="ready"]'));
      await page.keyboard.up("Shift");
      await waitFor("gh to mark #41 ready", () => /pr ready -- https:\/\/github.com\/example\/alpha\/pull\/41/.test(readLog("gh.log")));
    }
    await press(page.locator('#page-view .pr-row[data-url$="/pull/42"]'));
    await waitFor("#42 in the side panel", () => gui('#panel-tabs .ptab.active[data-panel="pr"]').isVisible());
    await waitFor("#42's details", async () => (await page.locator("#panel-body", { hasText: "Add notes" }).count()) > 0, 10000);
    if (touch) {
      await page.evaluate(() => closePanel());
      await pageSidebar();
      await tapOrClick("#sidebar footer .back");
    } else {
      await page.keyboard.press("Escape");
      if (!(await page.locator("#chat").isVisible())) await page.keyboard.press("Escape");
    }
    await waitFor("the thread again", () => page.locator("#chat").isVisible());
    if (await page.evaluate(() => document.body.dataset.panel === "open")) await tapOrClick("#toggle-panel");
    return "#42 authored, #41 to review, #43 others";
  });

  await step("shell: ⌘K opens the Pull requests and Usage pages", async () => {
    if (touch) return "touch: no keyboard";
    await page.keyboard.press("Meta+k");
    await page.keyboard.type("open pull requests");
    await page.keyboard.press("Enter");
    await waitFor("the Pull requests page", () => page.locator("#page-view h1", { hasText: "Pull Requests" }).isVisible());
    await page.keyboard.press("Meta+k");
    await page.keyboard.type("open usage");
    await page.keyboard.press("Enter");
    await waitFor("the Usage page", () => page.locator("#page-view h1", { hasText: "Usage" }).isVisible());
    await page.keyboard.press("Escape");
    await waitFor("back on Pull requests", () => page.locator("#page-view h1", { hasText: "Pull Requests" }).isVisible());
    await page.keyboard.press("Escape");
    await waitFor("the thread again", () => page.locator("#chat").isVisible());
  });

  await step("shell: Pull requests load more, condense the header, copy a link and list checks", async () => {
    fs.writeFileSync(path.join(HOME, "gh.log"), "");
    await showSidebar();
    await tapOrClick('#sidebar footer [data-open-page="pull-requests"]');
    await waitFor("the rows", async () => (await prNumbers()).length === 3, 15000);
    await page.fill("#page-view .pr-search input", "label:many");
    await waitFor("GitHub's first page", async () => (await prNumbers()).length === 100, 15000);
    await press(page.locator('#page-view [data-pr="more"]'));
    await waitFor("the second page", async () => (await prNumbers()).length === 150, 15000);
    check(!(await page.locator('#page-view [data-pr="more"]').count()), "Load more stayed after the last page");
    check(/-f after=page-2/.test(readLog("gh.log")), "gh never asked for the second page");
    await page.locator("#page-view .page-body").evaluate((el) => (el.scrollTop = 2000));
    await waitFor("the condensed header", () => page.locator("#page-view .page-head .condensed").isVisible());
    check((await page.locator("#page-view .page-head .condensed .crumb-menu").allTextContents()).join() === "Open,All", "the condensed filters are wrong");
    await press(page.locator('#page-view .page-head [data-pr="search-top"]'));
    await waitFor("the search, scrolled back", async () => (await page.evaluate(() => document.activeElement?.closest?.(".pr-search") !== null)) && !(await page.locator("#page-view .page-head .condensed").isVisible()));
    await page.fill("#page-view .pr-search input", "");
    await waitFor("every row again", async () => (await prNumbers()).length === 3, 15000);
    const num = page.locator('#page-view .pr-row[data-url$="/pull/42"] .num');
    if (touch) {
      const box = await num.boundingBox();
      await num.dispatchEvent("contextmenu", { clientX: box.x + 4, clientY: box.y + 4 });
    } else await num.click({ button: "right" });
    await waitFor("the number's menu", () => page.locator("#side-menu").isVisible());
    const items = await page.locator("#side-menu button .label").allTextContents();
    check(items.join() === "Copy link,Copy number,Open on GitHub", `menu: ${items.join()}`);
    await press(page.locator("#side-menu button", { hasText: "Copy link" }));
    const copied = await clipboard();
    check(copied === null || copied === "https://github.com/example/alpha/pull/42", `copied ${copied}`);
    await press(page.locator('#page-view .pr-row[data-url$="/pull/43"] .checks'));
    await waitFor("the checks", () => page.locator("#side-menu button", { hasText: "build" }).isVisible(), 10000);
    check((await text('#side-menu button:has-text("build") .hint')) === "success", "the check's result is missing");
    if (touch) await page.evaluate(() => closeSideMenu());
    else await page.keyboard.press("Escape");
    if (!touch) {
      await press(page.locator('#page-view .pr-row[data-url$="/pull/42"]'));
      await waitFor("#42 selected", () => page.locator('#page-view .pr-row.selected[data-url$="/pull/42"]').isVisible());
      await page.evaluate(() => navigator.clipboard?.writeText("").catch(() => {}));
      await page.keyboard.press("Meta+Shift+c");
      const link = await clipboard();
      check(link === null || link === "https://github.com/example/alpha/pull/42", `⌘⇧C copied ${link}`);
      await page.evaluate(() => closePanel());
      await page.keyboard.press("Escape");
    } else {
      await pageSidebar();
      await tapOrClick("#sidebar footer .back");
    }
    await waitFor("the thread again", () => page.locator("#chat").isVisible());
    return "100, then 150 rows";
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
