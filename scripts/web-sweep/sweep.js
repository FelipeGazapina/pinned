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
    vercel: { key_hint: "…fake" },
    connectors: [],
  };
  ws.send(JSON.stringify({ type: "call", id: 1, call: { cmd: "import", store, linear_key: "lin_api_fake", slack_key: "xoxb-fake", github_key: "ghp_fake", vercel_key: "vck_fake" } }));
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
  // It runs in every frame too, where a page that failed to load has no storage to read.
  await context.addInitScript(() => {
    try {
      localStorage.getItem("mode") || localStorage.setItem("mode", "terminal");
    } catch {}
  });
  let page = await context.newPage();

  let seen = [];
  page.on("console", (m) => ["error", "warning"].includes(m.type()) && seen.push(`console.${m.type()}: ${m.text()}`));
  page.on("pageerror", (e) => {
    // The first frame in pinned's own code says where a page error came from, or the top frames when it has none.
    const frames = (e.stack ?? "").split("\n").filter((l) => / at /.test(l)).map((l) => l.trim());
    const at = frames.find((l) => !/\/vendor\//.test(l)) ?? frames.slice(0, 4).join(" < ");
    seen.push(`pageerror: ${e.message}${at ? ` (${at})` : ""}`);
  });
  page.on("requestfailed", (r) => seen.push(`requestfailed: ${r.method()} ${r.url()} ${r.failure()?.errorText}`));
  page.on("response", (r) => r.status() >= 400 && seen.push(`HTTP ${r.status()}: ${r.request().method()} ${r.url()}`));

  // On a phone the chat header folds these into its Options menu, so a hidden one is reached there,
  // and the sidebar's sheet covers the header, so it closes first.
  const HEAD_MENU = { "#toggle-panel": /side panel$/, "#toggle-terminal": /terminal$/, "#toggle-details": /^Thread details$/ };
  const tapOrClick = async (selector) => {
    if (HEAD_MENU[selector] && touch) await hideSidebar();
    if (HEAD_MENU[selector] && !(await page.locator(selector).isVisible())) {
      await tapOrClick("#head-more");
      return press(page.locator("#panel-menu button", { hasText: HEAD_MENU[selector] }));
    }
    return touch ? page.tap(selector) : page.click(selector);
  };
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
    await settingsNav(name);
  };
  const settingsNav = (name) => tapOrClick(`#settings-nav [data-page="${name}"]`);
  // The theme toast's bold title and its description, as "title · description".
  const toastText = () => page.evaluate(() => [...document.querySelectorAll("#g-toast:not([hidden]) :is(b, p)")].map((el) => el.textContent).join(" · "));
  const pickOption = async (selector, value) => {
    await press(page.locator(selector));
    await press(page.locator(`#g-select-popup [data-value="${value}"]`));
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
    // While claude asks a question or shows its plan, T3 Code's pills take Send's place.
    if (touch) await page.tap((await page.locator("#pill-actions .pill").first().isVisible()) ? "#pill-actions .pill >> nth=0" : "#send");
    else {
      // Enter in the `/` menu picks its highlighted command, so a word sent as typed closes it first.
      // The menu lists the machine's commands, which a new thread has yet to ask for, so it can open late.
      if (/^\//.test(words)) await waitFor("the / menu", () => page.locator('#composer-menu[data-kind="slash"]').isVisible());
      if (await page.locator('#composer-menu:is([data-kind="slash"], [data-kind="mention"])').isVisible()) await page.press("#prompt", "Escape");
      await page.press("#prompt", "Enter");
    }
  };
  // A phone's composer stays open after a tap on one of its controls until a tap lands outside it, here the thread's gutter.
  const tapOutsideComposer = async () => {
    const r = await rect("#thread");
    await page.touchscreen.tap(r.left + 4, r.top + r.height / 2);
  };
  // A phone's composer collapses to one row whenever the prompt loses focus, hiding its controls until a tap on the
  // prompt reopens it. It collapses a moment after Send's tap, so the tap doesn't wait on the collapsed state.
  const openComposer = async () => {
    if (touch) await tapOrClick("#prompt");
  };
  // The phone's sheet is still visible while it slides out, so the body's state says whether it is open.
  const sidebarShown = async () => (await page.evaluate(() => document.body.dataset.sidebar === "open")) && (await page.locator("#sidebar").isVisible());
  const hideSidebar = async () => {
    if (!(await sidebarShown())) return;
    await tapOrClick("#sidebar-toggle");
    await waitFor("the sidebar to close", async () => !(await page.locator("#sidebar").isVisible()));
  };
  // A page's header carries its own toggle while the chat's is hidden under it.
  const showSidebar = async () => {
    if (await sidebarShown()) return;
    await tapOrClick((await page.locator("#chat-sidebar-toggle").isVisible()) ? "#chat-sidebar-toggle" : "#page-view .side-toggle");
    await waitFor("the sidebar", sidebarShown);
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
  /** The thread's PR #n line, which sits in an older turn: the thread draws only the turns near the
   * screen, so it scrolls up to it as a reader would. */
  const threadPrLink = async (n) => {
    const link = `#thread .pr-link[data-pr$="/pull/${n}"]`;
    for (let i = 0; i < 30 && !(await page.locator(link).count()); i++) {
      await page.evaluate(() => (threadView.scrollTop -= threadView.clientHeight / 2));
      await sleep(100);
    }
    return link;
  };

  await step("app view: tool runs group under a summary, with thinking and highlighted code", async () => {
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/beta"))));
    await waitFor("beta", async () => /beta/.test(await text("#chat-head .crumb")));
    await sendPrompt("rich");
    await waitFor("the code block", async () => (await page.locator("#thread .answer .codeblock").count()) > 0, 10000);
    const fold = lastTurn(".work > summary");
    // A turn whose lines all landed in the same millisecond took no time, and reads just Worked.
    check(/^Worked( for .+)?$/.test(await fold.locator(".label").textContent()), `the fold reads ${await fold.textContent()}`);
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
    check(thought === "Planning the read I'll look at the notes first. Then run the tests.", `the thinking row reads ${thought}`);
    await press(lastTurn(".group .tool.thought > summary"));
    await waitFor("the open thought to read Thought", async () => (await text("#thread .turn:last-of-type .group .tool.thought .label")) === "Thought", 3000);
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
    await tapOrClick("#image-zoom .media-close");
    await waitFor("the zoom to close", async () => !(await page.locator("#image-zoom").isVisible()));
  });

  await step("app view: errors, stopped turns and answered questions stay in sight", async () => {
    await sendPrompt("oops");
    await waitFor("the API error", async () => (await page.locator("#thread .error-row", { hasText: "API Error: 529 Overloaded" }).count()) === 1);
    check(/service is busy/.test(await text("#thread .error-row .message")), "the error's detail is missing");
    await sendPrompt("fail");
    await waitFor("the failed build", async () => (await page.locator("#thread .error-row", { hasText: "npm run build" }).count()) === 1);
    check(await gui("#thread .error-row:has-text('npm run build') .message").isVisible(), "the failure's output is not in sight");
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

  await step("app view: a published artifact shows as a card that opens it", async () => {
    await sendPrompt("artifact");
    await answered("Published the page.");
    const cards = page.locator("#thread .turn:last-of-type .artifact-card");
    check((await cards.count()) === 1, `${await cards.count()} artifact cards, but only one Artifact call published`);
    check((await cards.locator("h3").textContent()) === "payouts.html", `the card is titled ${await cards.locator("h3").textContent()}`);
    check((await cards.locator(".about").textContent()) === "Follows an invoice to the bank.", `the card says ${await cards.locator(".about").textContent()}`);
    // window.open stands in for the browser while the card opens, then comes back for the steps after.
    await page.evaluate(() => {
      window.realOpen = window.open;
      window.open = (url) => (window.openedUrl = url);
    });
    try {
      await press(cards.locator("a[href]"));
    } finally {
      await page.evaluate(() => (window.open = window.realOpen));
    }
    const opened = await page.evaluate(() => window.openedUrl);
    check(opened === "https://claude.ai/artifact/PgGFrq1LMW2iDhYZThVQcD", `Open went to ${opened}`);
    return opened;
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
    await waitFor("the compaction divider", async () => (await page.locator("#thread .compaction", { hasText: "Context compacted 259K" }).count()) > 0, 10000);
  });

  await step("app view: Stop ends the background agents before the turn, and the turn they wake", async () => {
    await sendPrompt("build");
    await waitFor("the running command", async () => (await text("#thread .live-tool .label")) === "Running npm", 6000);
    await tapOrClick("#stop");
    await waitTerm(/\[ctrl-x\]\[ctrl-k\]\[ctrl-x\]\[ctrl-k\]\[esc\]/, 3000);
    await waitTerm(/\[ctrl-x\]\[ctrl-k\]\[ctrl-x\]\[ctrl-k\]\[esc\][^[]*\[esc\]/, 4000);
    await answered("Built.");
  });

  // The fake claude logs each turn's length after its Stop hook returns, and $0.42 a turn.
  await step("app view: the header and the row read the thread's active time and cost, ticking while it works", async () => {
    const readouts = () =>
      page.evaluate(() => {
        const row = document.querySelector("#thread-list .thread-row.active");
        return { head: document.querySelector("#chat-head .crumb .stats")?.textContent ?? "", row: row?.querySelector(".stats")?.textContent ?? "", status: active.view.status };
      });
    const parse = (s) => {
      const m = /^(\d+)s · \$(\d+\.\d\d)$/.exec(s);
      return m && { secs: Number(m[1]), cost: m[2] };
    };
    await waitFor("the last turn to end", async () => (await readouts()).status === "done" && parse((await readouts()).head));
    await sleep(1000);
    const before = await readouts();
    check(before.row === before.head, `the row reads "${before.row}", the header "${before.head}"`);
    if (!touch) {
      await page.hover("#thread-list .thread-row.active .l2");
      await waitFor("the row's hover card", () => page.locator("#hover-card").isVisible(), 3000);
      const card = await text("#hover-card .card-lines .stats");
      await page.mouse.move(0, 0);
      check(card === before.head, `the row's hover card reads "${card}", the header "${before.head}"`);
    }
    const placed = await page.evaluate(() =>
      [["#chat-head .crumb .stats", "#chat-head .crumb"], ["#thread-list .thread-row.active .stats", "#thread-list .thread-row.active .l3"]].map(([inner, outer]) => {
        const box = document.querySelector(inner).getBoundingClientRect();
        if (!box.width) return "hidden";
        return box.right <= document.querySelector(outer).getBoundingClientRect().right + 0.5 ? "inside" : "spills";
      }));
    check(placed[0] === (touch ? "hidden" : "inside"), `the header readout is ${placed[0]}`);
    check(placed[1] !== "spills", "the row readout spills out of its line");
    if (!touch) {
      const sidebarWidth = await page.evaluate(() => document.getElementById("sidebar").getBoundingClientRect().width);
      const role = await page.evaluate(() => active.view.role ?? "");
      await page.evaluate(() => {
        setSidebarWidth(1);
        active.view.role = "orchestrator";
        renderSidebar();
      });
      const actsFit = () => page.evaluate(() => {
        const row = document.querySelector("#thread-list .thread-row.active");
        const side = document.getElementById("sidebar").getBoundingClientRect();
        const clipped = [...row.querySelectorAll(".acts [data-act]")].filter((button) => {
          const box = button.getBoundingClientRect();
          const hit = box.width > 0 ? document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2) : null;
          const inside = box.width > 0 && box.left >= side.left - 0.5 && box.right <= side.right + 0.5 && box.top >= side.top - 0.5 && box.bottom <= side.bottom + 0.5;
          return !(inside && hit && (hit === button || button.contains(hit)));
        }).map((button) => button.dataset.act);
        return { clipped, count: row.querySelectorAll(".acts [data-act]").length, focus: row.matches(":focus-visible") };
      });
      await page.hover("#thread-list .thread-row.active .l2");
      const hovered = await actsFit();
      await page.evaluate(() => document.querySelector("#thread-list .thread-row.active").focus({ focusVisible: true }));
      const focused = await actsFit();
      await page.evaluate(({ sidebarWidth, role }) => {
        active.view.role = role;
        renderSidebar();
        setSidebarWidth(sidebarWidth);
      }, { sidebarWidth, role });
      await page.mouse.move(0, 0);
      check(hovered.count > 0 && hovered.clipped.length === 0, `hover hides ${hovered.clipped.join(", ") || "the actions"}`);
      check(focused.focus && focused.clipped.length === 0, `focus hides ${focused.clipped.join(", ") || "the actions"}`);
    }
    await sendPrompt("think");
    await waitFor("the turn to start", async () => (await readouts()).status === "working");
    await sleep(2500);
    const during = await readouts();
    check(parse(during.head)?.secs >= parse(before.head).secs + 2, `while working it read "${during.head}" after "${before.head}"`);
    const cost = (Number(parse(before.head).cost) + 0.42).toFixed(2);
    await waitFor(`$${cost}`, async () => parse((await readouts()).head)?.cost === cost, 8000);
    const after = await readouts();
    check(after.status === "done" && parse(after.head).secs >= parse(before.head).secs + 4, `after the turn it read "${after.head}", status ${after.status}`);
    check(after.row === after.head, `the row reads "${after.row}", the header "${after.head}"`);
    return `${before.head} then ${during.head} then ${after.head}`;
  });

  await step("app view: spawned agents show as live cards, and the thread reads in progress until they finish", async () => {
    const beta = await page.evaluate(() => active.key);
    const card = (name) => page.locator("#thread .turn:last-of-type .agents .agent-card", { hasText: name });
    const stepLine = (name) => card(name).locator(".agent-step .label").textContent();
    const rowStatus = () => page.locator(`#thread-list .thread-row[data-tab="${beta}"] .stamp.status`).textContent().catch(() => "");
    await sendPrompt("spawn agents");
    await waitFor("two agent cards", async () => (await page.locator("#thread .turn:last-of-type .agents .agent-card").count()) === 2, 6000);
    check((await card("Map the parser").locator(".chip").textContent()) === "Explore · haiku", `the first card's chip reads ${await card("Map the parser").locator(".chip").textContent()}`);
    check((await card("Check the tests").getAttribute("data-state")) === "running", "the second card is not running");
    const first = await stepLine("Check the tests");
    await waitFor("the card's step to move on", async () => (await stepLine("Check the tests")) !== first, 5000);
    const later = await stepLine("Check the tests");
    check(/^cargo test batch_\d$/.test(later), `the step line reads ${later}`);
    await waitFor("claude's turn to end", () => page.evaluate(() => statusOf(active) === "done"), 5000);
    check((await page.evaluate(() => phaseOf(active))) === "delegating", `the phase is ${await page.evaluate(() => phaseOf(active))}`);
    check(/^Worked( for .+)?$/.test(await lastTurn(".work > summary .label").textContent()), `the turn's own work did not settle: ${await lastTurn(".work > summary").textContent()}`);
    const footer = (await text("#thread .agents-working")).replace(/\s+/g, " ");
    check(/^2 agents working \d/.test(footer), `the footer reads ${footer}`);
    check(/2 agents working/.test(await rowStatus()), `the sidebar row reads ${await rowStatus()}`);
    check((await page.locator("#composer").getAttribute("data-working")) === "false", "the composer acts as if claude's turn were running");
    await card("Check the tests").screenshot({ path: path.join(shots, "agent-card-running.png") });
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
    await waitFor("one agent left", async () => /1 agent working/.test(await rowStatus()), 8000);
    check(!(await page.evaluate((k) => unseenDone(tabByKey(k)), beta)), "the row read Done while an agent still worked");
    await waitFor("the row to show an unread finish", () => page.evaluate((k) => unseenDone(tabByKey(k)), beta), 12000);
    check((await page.locator(`#thread-list .thread-row[data-tab="${beta}"] .stamp.status`).count()) === 0, "the finished row still shows a status");
    await page.evaluate((k) => activate(tabByKey(k)), beta);
    await waitFor("both cards done", async () => (await page.locator('#thread .agent-card[data-state="done"]').count()) === 2, 4000);
    check((await page.locator("#thread .agents-working").count()) === 0, "the agents footer outlived the agents");
    const done = await stepLine("Check the tests");
    check(done === "Check the tests: all clear.", `the finished card's step reads ${done}`);
    await press(card("Check the tests").locator(".agent-head"));
    const rows = await card("Check the tests").locator(".agent-steps .tool").count();
    check(rows === 5, `the open card shows ${rows} tool rows`);
    await card("Check the tests").screenshot({ path: path.join(shots, "agent-card-done.png") });
    await press(card("Check the tests").locator(".agent-head"));
    return `${first} → ${later}`;
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
    const pressable = touch ? "head-more" : "toggle-panel";
    const r = await rect(`#${pressable}`);
    await page.mouse.move(r.left + r.width / 2, r.top + r.height / 2);
    await page.mouse.down();
    await sleep(200);
    const pressed = await page.evaluate((id) => getComputedStyle(document.getElementById(id)).scale, pressable);
    await page.mouse.move(0, 0);
    await page.mouse.up();
    check(pressed === "0.97", `a pressed button scales to ${pressed}`);
    await page.evaluate(() => toggleSettings());
    await settingsNav("appearance");
    await pickOption("#look-chat-width", "wide");
    await pickOption("#look-diff-colors", "blue-orange");
    await pickOption("#look-code-size", "16px");
    // The thread draws only the turns on screen, so a count and a code block are drawn here to read their styles.
    const looks = await page.evaluate(() => {
      const probe = Object.assign(document.createElement("div"), { innerHTML: '<span class="add">+1</span><div class="md"><pre><code>x</code></pre></div>' });
      threadView.append(probe);
      const read = [getComputedStyle(document.documentElement).getPropertyValue("--lane").trim(), getComputedStyle(probe.querySelector(".add")).color, getComputedStyle(probe.querySelector("code")).fontSize];
      probe.remove();
      return read;
    });
    check(looks.join("|") === "1152px|rgb(81, 162, 255)|16px", `looks: ${looks.join(" | ")}`);
    await pickOption("#look-chat-width", "full");
    check((await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--lane").trim())) === "100%", "Full doesn't widen the lane");
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    for (const name of ["chat-width", "diff-colors", "code-size"]) await pickOption(`#look-${name}`, "");
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
    // The reply's text can draw a moment before the turn ends, while its fold still reads Working.
    const folds = () => page.locator("#thread .turn .work > summary .label").allTextContents();
    const settled = (f) => /^Continued( for |$)/.test(f.at(-1)) && /^Worked( for |$)/.test(f.at(-2));
    await waitFor("the turn to end", async () => !/^Working/.test((await folds()).at(-1)), 3000).catch(() => {});
    check(settled(await folds()), `the folds read ${(await folds()).slice(-2).join(" | ")}`);
    await sendPrompt("Continue from where you left off.");
    await answered("echo: Continue from where you left off.");
    check((await text("#thread .turn:last-of-type .sent-by")) === "Sent by Pinned", "pinned's own prompt isn't marked");
    check((await page.locator("#thread .turn:nth-last-of-type(2) .sent-by").count()) === 0, "a typed prompt is marked as pinned's");
    // A tabs event, such as the turn's stats arriving, replaces the view, so the faked connector rides on each one.
    await page.evaluate(() => {
      const plain = updateView;
      window.updateView = (tab, view) => plain(tab, tab === active ? { ...view, connector: "sweep-workflow" } : view);
      window.updateView.plain = plain;
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
      window.updateView = updateView.plain;
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
    const bubbles = await page.locator("#thread .bubble").count();
    await press(answer.locator("[data-run-code]"));
    const shellText = () => page.evaluate(() => {
      const b = scriptShells.at(-1)?.term.buffer.active;
      return b ? Array.from({ length: b.length }, (_, i) => b.getLine(i).translateToString(true)).join("\n") : "";
    });
    await waitFor("the block's output in its own terminal", async () => /^ran-from-thread$/m.test(await shellText()), 10000);
    check(await page.evaluate(() => document.body.dataset.drawer === "open" && scriptShells.at(-1).name === "Chat code block"), "the block didn't run as T3's Chat code block script in the drawer");
    check((await page.locator("#thread .bubble").count()) === bubbles && !/ran-from-thread/.test(await termText()), "running the block went through the conversation");
    await page.evaluate(() => toggleDrawer(false));
    check((await page.locator("#thread .codeblock .lang", { hasText: "rust" }).count()) === 0 || (await page.locator("#thread .codeblock:has(.lang:text-is('rust')) [data-run-code]").count()) === 0, "a Rust block offers to run");
    await page.evaluate(() => wantLang("cobol"));
    await waitFor("a rare language's grammar", () => page.evaluate(() => shikiLangs.has("cobol")), 8000);
  });

  await step("app view: typography sets the interface, prompt, code and terminal fonts, and the lane keeps 48px gutters down to 640px", async () => {
    // The typed fields, without the font list Chromium could offer once a field takes focus.
    await page.evaluate(() => {
      Object.assign(installedFonts, { status: "unavailable", families: [] });
      toggleSettings();
    });
    await settingsNav("appearance");
    const resetTypography = () =>
      page.evaluate(() => {
        for (const name of ["ui-size", "prompt-size", "terminal-size", "sans-font", "mono-font", "advanced", "smoothing", "word-wrap"]) localStorage.removeItem(`look-${name}`);
        applyLooks();
        if (settingsOpen()) toggleSettings();
      });
    // Settings covers the thread, which draws its turns again once it shows.
    const inThread = async (read) => {
      await page.evaluate(() => toggleSettings());
      await waitFor("the thread's code", () => page.evaluate(() => !!document.querySelector("#thread .codeblock:has([data-run-code]) pre code")));
      const got = await page.evaluate(read);
      await page.evaluate(() => toggleSettings());
      await page.locator("#settings-page #typography").waitFor();
      return got;
    };
    try {
      check((await page.locator("#look-terminal-size").count()) === 0, "the terminal row shows without Advanced");
      check((await text("#code-font h4")) === "Monospace font", `the simple code row reads ${await text("#code-font h4")}`);
      await pickOption("#look-ui-size", "18px");
      const zoom = await page.evaluate(() => getComputedStyle(document.getElementById("chat")).zoom);
      check(zoom === "1.125", `the interface zooms ${zoom}`);
      await page.fill("#look-sans-font", "Georgia");
      await page.press("#look-sans-font", "Enter");
      await page.fill("#look-mono-font", "Courier New");
      await page.press("#look-mono-font", "Enter");
      const fonts = await inThread(() => [getComputedStyle(document.body).fontFamily, getComputedStyle(document.querySelector("#thread .md pre code")).fontFamily, active.term.options.fontFamily, active.term.options.fontSize].map((f) => String(f).split(",")[0].replaceAll('"', "")));
      check(fonts.join("|") === "Georgia|Courier New|Courier New|12", `fonts: ${fonts.join(" | ")}`);
      await page.locator("#look-advanced").setChecked(true);
      await waitFor("the advanced rows", () => page.locator("#look-terminal-size").isVisible());
      await pickOption("#look-terminal-size", "15px");
      await pickOption("#look-prompt-size", "18px");
      const sizes = await page.evaluate(() => [active.term.options.fontSize, getComputedStyle(prompt).fontSize, active.term.options.fontFamily.split(",")[0]]);
      check(sizes.join("|") === "15|18px|ui-monospace", `advanced sizes: ${sizes.join(" | ")}`);
      if (await page.evaluate(() => IS_MAC)) {
        check((await page.evaluate(() => getComputedStyle(document.body).webkitFontSmoothing)) === "antialiased" || engine !== "chromium", "text isn't antialiased by default");
        await page.locator("#look-smoothing").setChecked(false);
        check((await page.evaluate(() => document.documentElement.dataset.smoothing)) === "false", "smoothing didn't turn off");
        await page.locator("#look-smoothing").setChecked(true);
      }
      await page.locator("#look-word-wrap").setChecked(false);
      check((await inThread(() => document.querySelector("#thread .codeblock:has([data-run-code])")?.dataset.wrap)) === "false", "Word wrap off still wraps code");
      await page.locator("#look-word-wrap").setChecked(true);
      for (const id of ["#look-terminal-size", "#look-prompt-size", "#look-ui-size"]) await pickOption(id, "");
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
   * Syntax tokens keep pierre-light's own colors, as T3's do, so code is left out, as is a row's
   * branch, which T3 paints at muted/40 (about 1.7 in light) to sit behind the title. */
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
        if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true }) || el.closest("[aria-hidden=true], [disabled], [aria-disabled=true], .xterm, iframe, .wireframe, pre code, .dl .code, .fl .code, .thread-row .l3 .branch")) continue;
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
        seen.push(`${await page.evaluate(() => themeState().mode)}:${await appearance()}:${await toastText()}`);
      }
      check(seen.join(" ") === "light:light|:Appearance: Light dark:dark|:Appearance: Dark system:dark|:Appearance: System", `cycle: ${seen.join(" ")}`);
      await themeKey("A");
      await waitFor("the Change theme list", async () => (await page.locator("#palette .group .label").first().textContent()) === "Change theme");
      const names = await page.locator("#palette .item .name").allTextContents();
      check(names.join(",") === "T3 Code,T3 Chat,Grove,Ocean,Ember,Iris", `themes: ${names.join(", ")}`);
      check((await page.locator("#palette .item").first().locator(".current").textContent()) === "Current", "T3 Code isn't marked current");
      await press(page.locator("#palette .item", { hasText: "Grove" }));
      await waitFor("Grove", async () => (await appearance()) === "dark|grove");
      const canvas = await page.evaluate(() => {
        const probe = Object.assign(document.createElement("div"), { style: `background: color-mix(in srgb, ${colorsFor(themeById("grove"), "dark").surfaceRaised} 80%, transparent)` });
        document.body.append(probe);
        const read = [getComputedStyle(document.querySelector("#composer .box")).backgroundColor, getComputedStyle(probe).backgroundColor];
        probe.remove();
        return read;
      });
      check(canvas[0] === canvas[1], `the composer is ${canvas[0]}, Grove's dark raised surface at 80% ${canvas[1]}`);
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
    await settingsNav("appearance");
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
      check(/Harbor created/.test(await toastText()), `the toast reads ${await toastText()}`);
      await page.evaluate(() => toggleSettings());
      await settingsNav("appearance");
      const card = page.locator('#theme-section .theme-card[data-theme-card="harbor"]');
      check((await card.locator("[data-theme-action]").count()) === 4, "Harbor's card lacks Duplicate, Edit, Export, Remove");
      await press(card.locator('[data-theme-action="edit"]'));
      await waitFor("the edit panel", async () => (await text("#theme-editor .te-title b")) === "Edit theme");
      await page.evaluate(() => toggleSettings());
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
      check((await page.locator("#theme-editor .te-label").allTextContents()).join(",") === "Muted text,Border,Sidebar background,Sidebar controls,Sidebar selection", "the filter doesn't find the sidebar's families and roles");
      await page.fill('#theme-editor .te-value[data-role="sidebar"]', "#000000");
      await press(page.locator('#theme-editor [data-te="save"]'));
      await waitFor("the saved edit", async () => (await page.evaluate(() => colorHex(customThemes[0].colors.sidebar))) === "#000000");
      check((await appearance()) === "dark|harbor", `after the edit: ${await appearance()}`);
      const file = await page.evaluate(() => JSON.parse(themeFile(customThemes[0])));
      check(file.version === 1 && file.name === "Harbor" && file.appearance === "dark" && /^oklch\(/.test(file.colors.canvas), `the export: ${JSON.stringify(file).slice(0, 80)}`);
      await page.evaluate(() => toggleSettings());
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
      await settingsNav("appearance");
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

  /** A paint as [luminance, alpha], through a canvas so oklab() and color-mix() results compare. */
  const paintOf = (selector, prop) =>
    page.evaluate(([selector, prop]) => {
      const ctx = Object.assign(document.createElement("canvas"), { width: 1, height: 1 }).getContext("2d", { willReadFrequently: true });
      ctx.fillStyle = getComputedStyle(document.querySelector(selector))[prop];
      ctx.fillRect(0, 0, 1, 1);
      const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data;
      const lin = (c) => ((c /= 255) <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
      return [+(0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)).toFixed(4), a];
    }, [selector, prop]);

  await step("app view: Settings' Contrast slider runs 50 to 200% by 5, sharpens or fades text and borders in both appearances, and resets, as appearanceContrast.ts", async () => {
    await resetTheme();
    await page.evaluate(() => toggleSettings());
    await settingsNav("appearance");
    const slider = page.locator("#appearance-contrast");
    const shown = () => page.evaluate(() => [document.getElementById("appearance-contrast").value, document.querySelector("#setting-appearance-contrast output").textContent, document.querySelector('[data-reset="contrast"]').hidden, document.documentElement.dataset.contrast ?? "", localStorage.getItem("gui-contrast") ?? ""].join("|"));
    const looks = async () => ({ side: (await paintOf("#sidebar", "color"))[0], meta: (await paintOf("#setting-appearance-contrast .row-text p", "color"))[0], title: (await paintOf("#settings-nav .links button:not(.active)", "color"))[0], border: await paintOf("#theme-section .theme-card", "borderTopColor") });
    try {
      check((await text("#setting-appearance-contrast h4")) === "Contrast", `the row is ${await text("#setting-appearance-contrast h4")}`);
      const range = await slider.evaluate((i) => [i.min, i.max, i.step].join("|"));
      check(range === "50|200|5", `the range is ${range}`);
      check((await shown()) === "100|100%|true||", `at rest: ${await shown()}`);
      const base = await looks();
      await slider.focus();
      for (let i = 0; i < 12; i++) await page.keyboard.press("ArrowRight");
      check((await shown()) === "160|160%|false|160|160", `after twelve steps up: ${await shown()}`);
      const sharp = await looks();
      check(sharp.side > base.side && sharp.meta > base.meta && sharp.title > base.title && sharp.border[1] > base.border[1], `160% in dark: ${JSON.stringify(sharp)} against ${JSON.stringify(base)}`);
      await page.keyboard.press("Home");
      check((await shown()) === "50|50%|false|50|50", `Home: ${await shown()}`);
      const faint = await looks();
      check(faint.side < base.side && faint.meta < base.meta && faint.title < base.title && faint.border[1] < base.border[1], `50% in dark: ${JSON.stringify(faint)} against ${JSON.stringify(base)}`);
      await page.emulateMedia({ colorScheme: "light" });
      await waitFor("the light appearance", async () => (await appearance()) === "light|");
      await page.keyboard.press("End");
      const lightSharp = await looks();
      await press(page.locator('[data-reset="contrast"]'));
      check((await shown()) === "100|100%|true||", `after Reset: ${await shown()}`);
      const lightBase = await looks();
      check(lightSharp.side < lightBase.side && lightSharp.meta < lightBase.meta && lightSharp.title < lightBase.title && lightSharp.border[0] < lightBase.border[0], `200% in light: ${JSON.stringify(lightSharp)} against ${JSON.stringify(lightBase)}`);
    } finally {
      await page.emulateMedia({ colorScheme: "dark" });
      await page.evaluate(() => setContrast(CONTRAST.standard));
      await resetTheme();
      await page.evaluate(() => settingsOpen() && toggleSettings());
    }
  });

  await step("app view: a theme color's swatch opens T3's picker (plane, hue, HEX, RGB) that paints live, and its label spotlights every use with a count", async () => {
    await resetTheme();
    const sub = () => text("#theme-editor .te-title span");
    const field = (role) => page.locator(`#theme-editor .te-value[data-role="${role}"]`);
    try {
      await themeKey("T", true);
      await waitFor("the editor", () => page.locator("#theme-editor").isVisible());
      const labels = await page.locator("#theme-editor .te-label").evaluateAll((els) => els.map((e) => e.getAttribute("aria-label")));
      check(labels.join(",") === "Show Background usage,Show Accent usage", `labels: ${labels.join(", ")}`);
      await press(page.locator('#theme-editor [data-te-usage="canvas"]'));
      const uses = await page.evaluate(() => [...document.querySelectorAll("[data-theme-inspector-match]")].map((e) => e.id));
      check(uses.includes("expanded") && (await sub()) === `Background · ${uses.length} ${uses.length === 1 ? "use" : "uses"}`, `Background: ${await sub()}, marked ${uses.join(" ")}`);
      check((await page.locator("#theme-inspector-spotlight mask rect").count()) > 1, "no spotlight cut out");
      check((await page.locator('#theme-editor [data-te-usage="canvas"]').getAttribute("aria-pressed")) === "true", "the label isn't pressed");
      await press(page.locator('#theme-editor [data-te-usage="canvas"]'));
      check((await page.locator("#theme-inspector-spotlight, [data-theme-inspector-match]").count()) === 0 && (await sub()) === "Select a color below", `a second press leaves ${await sub()}`);
      const before = await field("accent").inputValue();
      await press(page.locator('#theme-editor [data-te-pick="accent"]'));
      await waitFor("the picker", () => page.locator("#theme-color-picker").isVisible());
      const head = [await text("#theme-color-picker .cp-title"), await text("#theme-color-picker .cp-sub"), await page.inputValue('#theme-color-picker [data-cp="hex"]'), await page.inputValue('#theme-color-picker [data-cp="rgb"]')];
      const rgb = [1, 3, 5].map((i) => parseInt(before.slice(i, i + 2), 16)).join(", ");
      check(head.join("|") === `Accent|Choose a color|${before.slice(0, 7)}|${rgb}`, `the picker opens as ${head.join(" | ")}`);
      check(/^Accent color · \d+ uses?$/.test(await sub()) && (await page.evaluate(() => document.querySelector("#send").hasAttribute("data-theme-inspector-match"))), `the swatch selects Accent: ${await sub()}`);
      const plane = await rect("#theme-color-picker .cp-plane");
      const corner = { x: plane.right - 2, y: plane.top + 2 };
      if (touch) await page.touchscreen.tap(corner.x, corner.y);
      else await page.mouse.click(corner.x, corner.y);
      await sleep(100);
      const vivid = await page.inputValue('#theme-color-picker [data-cp="hex"]');
      const hsv = await page.evaluate(() => colorPicker.hsv);
      check(hsv.s > 0.97 && hsv.v > 0.97 && vivid !== before.slice(0, 7), `the plane's corner gives ${vivid} (${JSON.stringify(hsv)})`);
      check((await field("accent").inputValue()) === (await page.evaluate(() => colorHex(themeEditor.colors.dark.accent))) && (await field("accent").inputValue()) !== before, "the field didn't take the picked color");
      await page.fill('#theme-color-picker [data-cp="rgb"]', "224, 86, 154");
      check((await page.inputValue('#theme-color-picker [data-cp="hex"]')) === "#e0569a", `RGB 224, 86, 154 shows hex ${await page.inputValue('#theme-color-picker [data-cp="hex"]')}`);
      const painted = await page.evaluate(() => [colorHex(themeEditor.colors.dark.accent), document.documentElement.dataset.themeId]);
      check(painted[1] === "__preview" && painted[0] !== before, `the preview: ${painted.join(" ")}`);
      await page.keyboard.press("Escape");
      check((await page.locator("#theme-color-picker").count()) === 0 && (await page.locator("#theme-editor").isVisible()), "Escape didn't close just the picker");
      await press(page.locator('#theme-editor [data-te="inspect"]'));
      check((await page.locator("#theme-inspector-spotlight").count()) === 0, "inspecting kept the usage spotlight");
    } finally {
      await page.evaluate(() => themeEditor && closeThemeEditor());
      await resetTheme();
    }
  });

  /** An Open VSX extension served from page.route: a search hit, its details, manifest, checksum and
   * a real .vsix (deflated zip) whose themes are a light and dark pair and a third that includes one. */
  const openVsxFixture = () => {
    const zlib = require("zlib");
    const crypto = require("crypto");
    const manifest = { publisher: "fixture", name: "aurora-themes", version: "1.0.0", license: "MIT", contributes: { themes: [{ label: "Aurora Light", uiTheme: "vs", path: "./themes/light.json" }, { label: "Aurora Dark", uiTheme: "vs-dark", path: "./themes/dark.json" }, { label: "Aurora Midnight", uiTheme: "vs-dark", path: "./themes/midnight.json" }] } };
    const files = {
      "extension/package.json": JSON.stringify(manifest),
      "extension/themes/light.json": JSON.stringify({ colors: { "editor.background": "#fbf7f0", "editor.foreground": "#2b2b2b", "button.background": "#c2410c" } }),
      "extension/themes/dark.json": JSON.stringify({ colors: { "editor.background": "#1a1b26", "editor.foreground": "#c0caf5", "focusBorder": "#7aa2f7", "sideBar.background": "#16161e" } }),
      "extension/themes/midnight.json": '{\n  // darker\n  "include": "./dark.json",\n  "colors": { "editor.background": "#0b0c14", },\n}',
    };
    const locals = [];
    const central = [];
    let at = 0;
    for (const [name, text] of Object.entries(files)) {
      const raw = Buffer.from(text);
      const data = zlib.deflateRawSync(raw);
      const head = Buffer.alloc(30);
      head.writeUInt32LE(0x04034b50, 0); head.writeUInt16LE(20, 4); head.writeUInt16LE(8, 8); head.writeUInt32LE(data.length, 18); head.writeUInt32LE(raw.length, 22); head.writeUInt16LE(name.length, 26);
      const dir = Buffer.alloc(46);
      dir.writeUInt32LE(0x02014b50, 0); dir.writeUInt16LE(20, 6); dir.writeUInt16LE(8, 10); dir.writeUInt32LE(data.length, 20); dir.writeUInt32LE(raw.length, 24); dir.writeUInt16LE(name.length, 28); dir.writeUInt32LE(at, 42);
      locals.push(head, Buffer.from(name), data);
      central.push(dir, Buffer.from(name));
      at += 30 + name.length + data.length;
    }
    const dirBytes = Buffer.concat(central);
    const end = Buffer.alloc(22);
    end.writeUInt32LE(0x06054b50, 0); end.writeUInt16LE(Object.keys(files).length, 8); end.writeUInt16LE(Object.keys(files).length, 10); end.writeUInt32LE(dirBytes.length, 12); end.writeUInt32LE(at, 16);
    const vsix = Buffer.concat([...locals, dirBytes, end]);
    const base = "https://open-vsx.org/api/fixture/aurora-themes/1.0.0/file";
    const detail = { namespace: "fixture", name: "aurora-themes", displayName: "Aurora Themes", version: "1.0.0", license: "MIT", description: "Light, dark and midnight.", downloadCount: 1234, repository: "https://github.com/example/aurora-themes", files: { manifest: `${base}/package.json`, sha256: `${base}/fixture.aurora-themes-1.0.0.sha256`, download: `${base}/fixture.aurora-themes-1.0.0.vsix` } };
    const closed = { ...detail, name: "closed-themes", displayName: "Closed Themes", license: "Proprietary" };
    const routes = {
      "/api/fixture/aurora-themes": JSON.stringify(detail),
      "/api/fixture/closed-themes": JSON.stringify(closed),
      "/api/fixture/aurora-themes/1.0.0/file/package.json": JSON.stringify(manifest),
      "/api/fixture/aurora-themes/1.0.0/file/fixture.aurora-themes-1.0.0.sha256": `${crypto.createHash("sha256").update(vsix).digest("hex")}  fixture.aurora-themes-1.0.0.vsix\n`,
      "/api/fixture/aurora-themes/1.0.0/file/fixture.aurora-themes-1.0.0.vsix": vsix,
    };
    const hits = [];
    const handler = (route) => {
      const url = new URL(route.request().url());
      hits.push(`${route.request().method()} ${url.pathname}`);
      const headers = { "access-control-allow-origin": "*" };
      if (url.pathname === "/api/-/search") return route.fulfill({ headers, contentType: "application/json", body: JSON.stringify({ extensions: url.searchParams.get("query") === "aurora" ? [{ namespace: "fixture", name: "aurora-themes" }, { namespace: "fixture", name: "closed-themes" }] : [] }) });
      const body = routes[url.pathname];
      return body === undefined ? route.fulfill({ status: 404, headers, body: "" }) : route.fulfill({ headers: { ...headers, "content-length": String(body.length) }, body: route.request().method() === "HEAD" ? "" : body });
    };
    return { handler, hits };
  };
  /** WebKit sends a service-worker-controlled page's fetches past page.route, so a step that serves
   * a site from the sweep runs in a second page of the same account with service workers blocked. */
  const withRoutablePage = async (run) => {
    const main = page;
    const ctx = await browser.newContext({ ...profile, colorScheme: "dark", serviceWorkers: "block", storageState: await context.storageState() });
    if (touch) await ctx.addInitScript(stubVisualViewport);
    page = await ctx.newPage();
    page.on("console", (m) => ["error", "warning"].includes(m.type()) && seen.push(`console.${m.type()}: ${m.text()}`));
    page.on("pageerror", (e) => seen.push(`pageerror: ${e.message}`));
    try {
      await page.goto(HUB);
      await page.waitForFunction(() => typeof hub !== "undefined" && document.body.dataset.mode === "gui", null, { timeout: 20000 });
      return await run();
    } finally {
      page = main;
      await ctx.close();
    }
  };
  const openAddTheme = async () => {
    if (!(await page.evaluate(() => settingsOpen()))) await page.evaluate(() => toggleSettings());
    await settingsNav("appearance");
    await press(page.locator('#theme-section [data-theme-action="import"]'));
    await waitFor("the Add a theme dialog", async () => (await text("#shell-dialog .dialog-head h2")) === "Add a theme");
  };

  await step("app view: Add theme searches Open VSX, installs an extension as one collection card whose variants fan out per appearance, updates it, and removes picked variants", async () => {
    await withRoutablePage(async () => {
      await resetTheme();
      const fixture = openVsxFixture();
      await page.route("https://open-vsx.org/**", fixture.handler);
      const collection = page.locator('#theme-section .theme-card.collection[data-theme-collection="open-vsx:fixture.aurora-themes"]');
      try {
        await openAddTheme();
        check((await text("#theme-search-heading")) === "Search community themes", `the search reads ${await text("#theme-search-heading")}`);
        const chips = await page.locator("#shell-dialog .ti-chip").allTextContents();
        check(chips.join(",") === "Dracula,Catppuccin,Nord,Tokyo Night", `popular: ${chips.join(", ")}`);
        await page.fill('#shell-dialog input[aria-label="Search Open VSX themes"]', "aurora");
        await waitFor("the results", async () => (await page.locator("#shell-dialog .ti-card").count()) > 0, 10000);
        const cards = await page.locator("#shell-dialog .ti-card h4").allTextContents();
        check(cards.join(",") === "Aurora Themes", `results (the proprietary one left out): ${cards.join(", ")}`);
        check(/^fixture · 1\.2K downloads$/i.test(await text("#shell-dialog .ti-card .muted")), `the byline is ${await text("#shell-dialog .ti-card .muted")}`);
        await press(page.locator("#ti-sort"));
        check((await page.locator("#g-select-popup [role=option]").allTextContents()).join(",") === "Most downloaded,Best rated,Newest,Most relevant", "the Sort menu is missing");
        await press(page.locator('#g-select-popup [data-value="downloadCount"]'));
        check((await page.locator('#shell-dialog .ti-source[href="https://github.com/example/aurora-themes"] .github').count()) === 1, "no GitHub source link");
        await press(page.locator('#shell-dialog [data-ti-install="fixture.aurora-themes"]'));
        await waitFor("the install", async () => (await page.locator("#shell-dialog").isHidden()) && (await collection.count()) === 1, 10000);
        check(/^2 themes added · Aurora, Aurora Midnight$/.test(await toastText()), `the toast reads ${await toastText()}`);
        check(fixture.hits.some((h) => h.endsWith(".vsix") && h.startsWith("GET")) && fixture.hits.some((h) => h.endsWith(".sha256")), `fetched: ${fixture.hits.join(", ")}`);
        const installed = await page.evaluate(() => customThemes.map((t) => `${t.label}:${modesOf(t).join("+")}:${colorHex(colorsFor(t, modesOf(t).at(-1)).canvas)}:${t.collection.label}`));
        check(installed.join(" ") === "Aurora:light+dark:#1a1b26:Aurora Themes Aurora Midnight:dark:#0b0c14:Aurora Themes", `installed: ${installed.join(" ")}`);
        check((await collection.locator(".name").textContent()) === "Aurora Themes", "the card isn't named for the extension");
        const roots = await collection.locator(".pick.root").evaluateAll((els) => els.map((e) => e.getAttribute("aria-label")));
        check(roots.join(" | ") === "Use light variant, currently Aurora | Choose dark variant, 2 options, currently Aurora", `roots: ${roots.join(" | ")}`);
        const dark = collection.locator('.variant-mode[data-mode="dark"]');
        await dark.locator(".pick.root").focus();
        await waitFor("the dark variants to fan out", async () => (await dark.locator(".variant-option").evaluateAll((els) => els.filter((e) => getComputedStyle(e).opacity === "1").length)) === 2);
        await press(dark.locator('.variant-option[aria-label^="Use Aurora Midnight"]'));
        const midnight = await page.evaluate(() => customThemes.find((t) => t.label === "Aurora Midnight").id);
        await page.evaluate(() => setAppearanceMode("dark"));
        check((await appearance()) === `dark|${midnight}`, `picking Midnight for dark gives ${await appearance()}`);
        await press(collection.locator(".name"));
        check((await page.evaluate(() => JSON.stringify(Object.values(themeState().halves).map((id) => themeById(id).label)))) === '["Aurora","Aurora"]', "the card's name didn't pick the first variants");
        await openAddTheme();
        await page.fill('#shell-dialog input[aria-label="Search Open VSX themes"]', "aurora");
        await waitFor("the Update button", async () => (await page.locator('#shell-dialog [data-ti-install="fixture.aurora-themes"]').textContent().catch(() => "")) === "Update", 10000);
        await press(page.locator('#shell-dialog [data-ti-install="fixture.aurora-themes"]'));
        check((await text("#ti-update-title")) === "Update “Aurora Themes”?", `the confirm reads ${await text("#ti-update-title")}`);
        await press(page.locator("#shell-dialog [data-ti-update]"));
        await waitFor("the update", async () => /^2 themes updated/.test(await toastText()), 10000);
        await press(collection.locator('[data-theme-action="remove"]'));
        check((await text("#shell-dialog .dialog-head h2")) === "Remove themes from “Aurora Themes”?", `the removal asks ${await text("#shell-dialog .dialog-head h2")}`);
        const remove = page.locator("#shell-dialog [data-dialog-remove]");
        check(await remove.isDisabled(), "Remove selected is enabled with nothing picked");
        await press(page.locator('#shell-dialog .theme-remove-option', { hasText: "Aurora Midnight" }));
        check((await remove.textContent()) === "Remove selected (1)", `the button reads ${await remove.textContent()}`);
        await press(remove);
        const left = await page.evaluate(() => customThemes.map((t) => t.label).join(","));
        check(left === "Aurora" && (await collection.count()) === 0 && (await page.locator('#theme-section .theme-card[data-theme-card] .name', { hasText: /^Aurora$/ }).count()) === 1, `after removing Midnight: ${left}`);
      } finally {
        await page.unroute("https://open-vsx.org/**", fixture.handler);
        await resetTheme();
      }
    });
  });

  await step("app view: Add theme's JSON box converts a pasted VS Code theme, offers Update or Keep both for one already installed, and pairs dropped Light and Dark files", async () => {
    await resetTheme();
    const owl = { name: "night-owl-test", type: "dark", colors: { "editor.background": "#011627", "editor.foreground": "#d6deeb", "button.background": "#7e57c2", "focusBorder": "#122d42", "sideBar.background": "#011627" }, tokenColors: [] };
    try {
      await openAddTheme();
      const spill = await overflowing();
      check(!spill.length, `the dialog is wider than the screen: ${spill.join(", ")}`);
      await page.fill("#theme-json-editor", JSON.stringify(owl, null, 2));
      check((await page.locator("#shell-dialog .ti-editor pre .key").count()) > 3, "the JSON isn't highlighted");
      await press(page.locator("#shell-dialog [data-ti-add]"));
      await waitFor("the pasted theme", async () => (await appearance()) === "dark|night-owl-test");
      check((await toastText()) === "Night Owl Test added · It’s now your dark theme.", `the toast reads ${await toastText()}`);
      const owlColors = await page.evaluate(() => ["canvas", "messageAction", "sidebar"].map((r) => colorHex(themeById("night-owl-test").colors[r])).join(" "));
      check(owlColors === "#011627 #7e57c2 #011627", `converted: ${owlColors}`);
      await openAddTheme();
      await page.fill("#theme-json-editor", JSON.stringify(owl));
      await press(page.locator("#shell-dialog [data-ti-add]"));
      const conflict = (await page.locator("#shell-dialog .ti-conflict").innerText()).replace(/\s+/g, " ");
      check(conflict === "Already installed Night Owl Test", `the conflict reads ${conflict}`);
      await press(page.locator('#shell-dialog [data-ti-resolve="copy"]'));
      await waitFor("the copy", async () => (await page.evaluate(() => customThemes.map((t) => t.label).join(","))) === "Night Owl Test,Night Owl Test (1)");
      await openAddTheme();
      await page.fill("#theme-json-editor", JSON.stringify({ version: 1, name: "Broken", appearance: "dark", colors: { canvas: "teal-ish" } }));
      await press(page.locator("#shell-dialog [data-ti-add]"));
      check(/^The color for "canvas" must be a literal CSS color/.test(await text("#shell-dialog .ti-error")), `the error reads ${await text("#shell-dialog .ti-error")}`);
      const ayu = (type, bg) => ({ name: `Ayu ${type === "light" ? "Light" : "Dark"}`, type, colors: { "editor.background": bg, "editor.foreground": type === "light" ? "#5c6166" : "#bfbdb6" } });
      await page.setInputFiles('#shell-dialog .ti-file input[type="file"]', [
        { name: "ayu-light.json", mimeType: "application/json", buffer: Buffer.from(JSON.stringify(ayu("light", "#fcfcfc"))) },
        { name: "ayu-dark.json", mimeType: "application/json", buffer: Buffer.from(JSON.stringify(ayu("dark", "#0d1017"))) },
      ]);
      await waitFor("the paired files", async () => (await page.locator("#shell-dialog").isHidden()) && (await page.evaluate(() => !!themeById("ayu"))));
      const pair = await page.evaluate(() => `${modesOf(themeById("ayu")).join("+")} ${colorHex(colorsFor(themeById("ayu"), "light").canvas)} ${colorHex(colorsFor(themeById("ayu"), "dark").canvas)}`);
      check(pair === "light+dark #fcfcfc #0d1017", `Ayu: ${pair}`);
      check((await toastText()) === "Ayu added · Ayu", `the toast reads ${await toastText()}`);
    } finally {
      await page.evaluate(() => (themeImport ? endThemeImport() : dialogOpen() && closeDialog()));
      await resetTheme();
      await page.evaluate(() => settingsOpen() && toggleSettings());
    }
  });

  const openSettingsFromSidebar = async () => {
    if (touch) await page.evaluate(() => settingsOpen() || toggleSettings());
    else if (!(await page.evaluate(() => settingsOpen()))) await press(page.locator("#gui-settings"));
    await waitFor("Settings", () => page.evaluate(() => settingsOpen()));
  };

  await step("app view: Settings covers the window with Back, Search settings over the sections, and a page per section", async () => {
    await page.evaluate(() => (page = "general"));
    await openSettingsFromSidebar();
    try {
      const cover = await page.evaluate(() => [Object.values(document.getElementById("settings").getBoundingClientRect().toJSON()).slice(0, 4).map(Math.round).join(","), `${innerWidth},${innerHeight}`]);
      check(cover[0].startsWith("0,0,") && cover[0].endsWith(cover[1]), `Settings doesn't cover the window: ${cover.join(" in ")}`);
      const sections = await page.locator("#settings-nav .links button").allTextContents();
      check(sections.join(",") === "General,Appearance,Projects,Keybindings,Connectors,Accounts,Workflows,Account,Machines,Archive", `sections: ${sections.join(", ")}`);
      check((await page.locator("#settings-nav .links button svg").count()) === sections.length, "a section lacks its icon");
      check((await page.locator('#settings-nav [data-page="general"].active').count()) === 1, "General isn't marked");
      check((await page.locator("#settings-nav .settings-search input").getAttribute("placeholder")) === "Search", "no Search settings box");
      if (!touch) {
        await page.locator("#settings-page").click({ position: { x: 5, y: 5 } });
        await page.keyboard.press("/");
        await waitFor("/ to focus Search settings", () => page.evaluate(() => document.activeElement === document.querySelector("#settings-nav .settings-search input")), 2000);
      }
      await page.fill("#settings-nav .settings-search input", "zzqx");
      check((await text("#settings-nav .settings-none")) === "No settings found" && (await page.locator("#settings-nav .settings-none").isVisible()), "no empty state");
      check(await page.locator("#settings-nav .links").isHidden(), "the sections show beside the results");
      await page.fill("#settings-nav .settings-search input", "font");
      const results = await page.locator("#settings-nav .settings-results [data-result] .title").allTextContents();
      const fonts = `${(await page.evaluate(() => IS_MAC)) ? "Font smoothing," : ""}Interface font,Prompt font,Code font,Terminal font`;
      check(results.join(",") === fonts, `font finds ${results.join(", ")}`);
      await page.fill("#settings-nav .settings-search input", "glass");
      check((await text("#settings-nav .settings-results [data-result] .sub")) === "Appearance", "a result doesn't name its section");
      await page.press("#settings-nav .settings-search input", "Enter");
      await waitFor("the Appearance page", async () => (await text("#settings-page h2")) === "Appearance");
      await waitFor("the glass row in focus", () => page.evaluate(() => document.activeElement?.id === "setting-glass-opacity"));
      check((await page.inputValue("#settings-nav .settings-search input")) === "", "picking a result keeps the query");
      check((await page.locator('#settings-nav [data-page="appearance"].active').count()) === 1, "Appearance isn't marked");
      if (!touch) {
        await page.keyboard.press("Escape");
        await waitFor("Escape back to the thread", async () => !(await page.evaluate(() => settingsOpen())));
        await openSettingsFromSidebar();
      }
      await press(page.locator("#settings-back"));
      await waitFor("Back to the thread", async () => !(await page.evaluate(() => settingsOpen())));
      return results.join(", ");
    } finally {
      await page.evaluate(() => settingsOpen() && toggleSettings());
    }
  });

  await step("app view: Appearance is T3's page: themes, Interface (contrast, glass opacity, diff colors, chat width), Motion and Typography, each row resetting to its default", async () => {
    await openSettingsFromSidebar();
    await settingsNav("appearance");
    try {
      const heads = await page.locator("#settings-page .settings-section .section-head h3").allTextContents();
      check(heads.join(",") === "Interface,Motion,Typography", `sections: ${heads.join(", ")}`);
      check((await page.locator("#settings-page #appearance #theme-section .mode-tile").count()) === 3, "Colors & themes doesn't lead the page");
      check((await page.locator("#settings-page [data-reset]:not([hidden])").count()) === 0, "a reset shows at the defaults");
      await page.locator("#glass-opacity").fill("60");
      const glass = await page.evaluate(() => [getComputedStyle(document.documentElement).getPropertyValue("--g-glass-opacity").trim(), document.querySelector("#setting-glass-opacity output").textContent, document.querySelector('[data-reset="glass"]').hidden].join("|"));
      check(glass === "60%|60%|false", `glass at 60: ${glass}`);
      await press(page.locator('[data-reset="glass"]'));
      check((await page.inputValue("#glass-opacity")) === "80" && (await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--g-glass-opacity").trim())) === "80%", "Reset leaves glass off 80%");
      await press(page.locator("#look-chat-width"));
      const options = await page.locator("#g-select-popup [role=option]").allTextContents();
      check(options.join(",") === "Comfortable (default),Wide,Full", `chat width offers ${options.join(", ")}`);
      await press(page.locator('#g-select-popup [data-value="wide"]'));
      check((await text("#look-chat-width")) === "Wide" && (await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--lane").trim())) === "1152px", "Wide doesn't take");
      await press(page.locator('[data-reset="chat-width"]'));
      check((await text("#look-chat-width")) === "Comfortable", `after Reset the trigger reads ${await text("#look-chat-width")}`);
      check(/Red & green/.test(await text("#look-diff-colors")) && (await page.locator("#look-diff-colors .diff-dots i").count()) === 2, "Diff colors lacks its dots");
      const motion = page.locator("#panel-animation-duration");
      check((await motion.evaluate((i) => [i.min, i.max, i.step, i.value].join("|"))) === "0|400|25|0", "the motion slider isn't 0 to 400 by 25");
      await motion.fill("175");
      check((await text("#panel-animations output")) === "175 ms" && (await page.evaluate(() => localStorage.getItem("gui-panel-animation"))) === "175", "175 ms isn't kept");
      await press(page.locator(".motion-preview"));
      check(await page.evaluate(() => document.querySelector(".motion-preview").classList.contains("closed") && getComputedStyle(document.querySelector(".motion-preview .side")).transitionDuration.startsWith("0.175s")), "the preview doesn't fold at 175 ms");
      if (!touch) {
        await press(page.locator("#settings-back"));
        await press(page.locator("#sidebar-toggle"));
        const animating = await page.evaluate(() => [document.body.classList.contains("sidebar-animating"), getComputedStyle(document.getElementById("workspace")).transitionDuration].join("|"));
        check(animating === "true|0.175s", `the sidebar collapses with ${animating}`);
        await page.evaluate(() => toggleSidebar(true));
        await openSettingsFromSidebar();
      }
      await press(page.locator('[data-reset="motion"]'));
      check((await motion.inputValue()) === "0", "Reset leaves the motion on");
      await pickOption("#look-ui-size", "18px");
      check(!(await page.locator('[data-reset="interface"]').isHidden()), "a changed font size shows no reset");
      await press(page.locator('[data-reset="interface"]'));
      check((await text("#look-ui-size")) === "16 px", `after Reset the size reads ${await text("#look-ui-size")}`);
      const bad = await overflowing();
      check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    } finally {
      await page.evaluate(() => {
        for (const key of ["gui-glass-opacity", "gui-panel-animation", "look-chat-width", "look-ui-size"]) localStorage.removeItem(key);
        applyLooks();
        if (settingsOpen()) toggleSettings();
      });
    }
  });

  await step("app view: a control's title shows in T3's styled tooltip with its shortcut, theme toasts stack a bold title over a description, and ⌘⌥⇧A works over Settings, once per press, but not in a terminal", async () => {
    await resetTheme();
    try {
      if (!touch) {
        await page.mouse.move(600, 400);
        await page.locator("#sidebar-toggle").hover();
        await waitFor("the tooltip", () => page.locator("#g-tooltip").isVisible(), 3000);
        const tip = await page.evaluate(() => {
          const t = document.getElementById("g-tooltip");
          const b = document.getElementById("sidebar-toggle");
          const [tr, br] = [t.getBoundingClientRect(), b.getBoundingClientRect()];
          return [t.textContent, b.hasAttribute("title"), tr.bottom <= br.top || tr.top >= br.bottom, getComputedStyle(t).fontSize, getComputedStyle(t).borderTopLeftRadius].join("|");
        });
        check(tip === "Toggle sidebar (⌘B)|false|true|12px|6px", `the tooltip: ${tip}`);
        await page.mouse.move(600, 400);
        await waitFor("the tooltip to go", async () => (await page.locator("#g-tooltip").isHidden()) && (await page.getAttribute("#sidebar-toggle", "title")) === "Toggle sidebar (⌘B)", 3000);
        await page.evaluate(() => toggleSettings());
        await settingsNav("appearance");
        await page.locator('#theme-section .pick[data-theme-half="light"]').first().hover();
        await waitFor("the card's tooltip", async () => (await page.locator("#g-tooltip").isVisible()) && (await text("#g-tooltip")) === "Use for light mode only", 3000);
        await page.mouse.move(5, 5);
      } else await page.evaluate(() => toggleSettings());
      await waitFor("Settings", () => page.evaluate(() => settingsOpen()));
      if (!touch) {
        await themeKey("A", true);
        await waitFor("Light over Settings", async () => (await appearance()) === "light|");
        const toast = await page.evaluate(() => [document.querySelector("#g-toast b")?.textContent, document.querySelectorAll("#g-toast .title svg").length, document.querySelector("#g-toast p")?.textContent ?? ""].join("|"));
        check(toast === "Appearance: Light|0|", `the appearance toast: ${toast}`);
        await waitFor("the toast to leave after 1.5s", () => page.locator("#g-toast").isHidden(), 4000);
        await page.evaluate(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "A", code: "KeyA", altKey: true, shiftKey: true, metaKey: IS_MAC, ctrlKey: !IS_MAC, repeat: true, bubbles: true })));
        check((await appearance()) === "light|", "a held ⌘⌥⇧A keeps cycling");
        await page.evaluate(() => toggleSettings());
        await page.evaluate(() => active.term.textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "A", code: "KeyA", altKey: true, shiftKey: true, metaKey: IS_MAC, ctrlKey: !IS_MAC, bubbles: true })));
        check((await appearance()) === "light|", "⌘⌥⇧A cycles from a focused terminal");
      }
      await page.evaluate(() => themesImported([{ label: "Aurora" }, { label: "Aurora Midnight" }], false));
      await waitFor("the toast to slide in", () => page.evaluate(() => !document.getElementById("g-toast").getAnimations().length), 3000);
      const stacked = await page.evaluate(() => {
        const t = document.getElementById("g-toast");
        const r = t.getBoundingClientRect();
        return [t.querySelector("b").textContent, t.querySelector("p").textContent, !!t.querySelector(".title svg.success"), Math.round(document.documentElement.clientWidth - r.right) === (document.documentElement.clientWidth >= 640 ? 32 : 16), r.width <= 360, !!t.querySelector(".dismiss")].join("|");
      });
      check(stacked === "2 themes added|Aurora, Aurora Midnight|true|true|true|true", `the stacked toast: ${stacked}`);
      await press(page.locator("#g-toast .dismiss"));
      await waitFor("the dismissed toast", () => page.locator("#g-toast").isHidden(), 3000);
    } finally {
      await page.evaluate(() => settingsOpen() && toggleSettings());
      await resetTheme();
    }
  });

  await step("app view: the theme editor filters by family and role, keeps a bad hex marked, re-derives an imported theme, moves a taken name to the free appearance, merges a renamed theme whole, and Cancel clears the pick", async () => {
    await resetTheme();
    try {
      await page.evaluate(() => {
        customThemes = [{ id: "solo", label: "Solo", appearance: "dark", colors: colorsFor(themeById("grove"), "dark") }, { id: "pair", label: "Pair", appearance: "light", colors: colorsFor(themeById("grove"), "light") }];
        saveCustomThemes();
        openThemeEditor({ appearance: "dark" });
      });
      await page.fill("#theme-name", "Solo");
      await waitFor("the free appearance", async () => (await text("#theme-editor .te-seg .on")) === "Light");
      check((await text('#theme-editor [data-te="save"]')) === "Add light palette", `save reads ${await text('#theme-editor [data-te="save"]')}`);
      check((await page.evaluate(() => document.activeElement?.id)) === "theme-name", "typing lost the name field");
      await press(page.locator('#theme-editor [data-te-usage="accent"]'));
      await waitFor("the role's uses", async () => /^Accent color · \d+ uses?$/.test(await text("#theme-editor .te-title span")));
      await press(page.locator('#theme-editor [data-te="inspect"]'));
      await press(page.locator('#theme-editor [data-te="inspect"]'));
      check((await text("#theme-editor .te-title span")) === "Select a color below" && (await page.locator("#theme-editor .te-field.selected").count()) === 0, "Cancel kept the selection");
      await page.locator('#theme-editor [data-te="advanced"]').setChecked(true);
      await page.fill("#theme-editor .te-filter", "sidebar");
      const found = await page.locator("#theme-editor .te-field .te-label").allTextContents();
      check(found.join(",") === "Muted text,Border,Sidebar background,Sidebar controls,Sidebar selection", `sidebar finds ${found.join(", ")}`);
      await page.fill("#theme-editor .te-filter", "");
      const hex = page.locator('#theme-editor [data-theme-color-role="canvas"] .te-value');
      await hex.fill("#zz12");
      check((await hex.getAttribute("aria-invalid")) === "true" && (await hex.inputValue()) === "#zz12", "a bad hex isn't kept and marked");
      await press(page.locator('#theme-editor [data-te="save"]'));
      check(/must be a literal CSS color/.test(await text("#theme-editor .te-error")), `saving a bad hex says ${await text("#theme-editor .te-error")}`);
      await page.locator('#theme-editor [data-theme-color-role="canvas"] .te-value').fill("#101820");
      check((await page.locator('#theme-editor [data-theme-color-role="canvas"] .te-value').getAttribute("aria-invalid")) === null, "a good hex stays marked");
      await page.evaluate(() => {
        closeThemeEditor();
        openThemeEditor({ editingId: "solo" });
      });
      check(await page.locator('#theme-editor [data-te="advanced"]').isChecked(), "an imported theme opens in simple mode");
      const before = await page.evaluate(() => themeEditor.colors.dark.messageSurface);
      await page.locator('#theme-editor [data-te="advanced"]').setChecked(false);
      const after = await page.evaluate(() => [themeEditor.colors.dark.messageSurface, themeEditor.colors.light.messageSurface === colorsFor(themeById("grove"), "light").messageSurface || themeEditor.colors.light.messageSurface === standardColors("light").messageSurface]);
      check(after[0] !== before && after[1], `turning Advanced off: ${before} to ${after.join(" | ")}`);
      await page.fill("#theme-name", "Pair");
      check((await text('#theme-editor [data-te="save"]')) === "Merge into “Pair”", `save reads ${await text('#theme-editor [data-te="save"]')}`);
      await press(page.locator('#theme-editor [data-te="save"]'));
      await waitFor("the merge", () => page.evaluate(() => !themeEditor));
      const merged = await page.evaluate(() => [customThemes.map((t) => t.id).join(","), modesOf(themeById("pair")).join(",")].join("|"));
      check(merged === "pair|light,dark", `after the merge: ${merged}`);
      check((await toastText()) === "Pair updated · Its dark palette was added.", `the toast reads ${await toastText()}`);
    } finally {
      await resetTheme();
    }
  });

  await step("app view: font fields take only installed fonts (monospace for code), name the default face, and turn into T3's font list once the browser lists installed fonts; panel and drawer terminals follow the terminal font", async () => {
    // Chromium may list the machine's fonts itself; the typed field is checked without them first.
    await page.evaluate(() => {
      Object.assign(installedFonts, { status: "unavailable", families: [] });
      toggleSettings();
    });
    await settingsNav("appearance");
    try {
      const sans = page.locator("#look-sans-font");
      await sans.fill("NoSuchFontXyz");
      await sans.press("Enter");
      check((await sans.getAttribute("aria-invalid")) === "true" && (await page.evaluate(() => localStorage.getItem("look-sans-font"))) === null, "a missing font was taken");
      await sans.press("Escape");
      const mono = page.locator("#look-mono-font");
      await mono.fill("Georgia");
      await mono.press("Enter");
      const monoState = await page.evaluate(() => [document.getElementById("look-mono-font").tagName, document.getElementById("look-mono-font").getAttribute("aria-invalid"), localStorage.getItem("look-mono-font"), fontAvailable("Georgia"), fontMonospace("Georgia")].join("|"));
      check(/^INPUT\|true\|(null)?\|/.test(monoState), `a proportional code font was taken: ${monoState}`);
      await mono.press("Escape");
      const placeholders = await page.evaluate(() => [document.getElementById("look-sans-font").placeholder, document.getElementById("look-mono-font").placeholder]);
      check(await page.evaluate((names) => names.every((p) => /^SF /.test(p) || fontAvailable(p)), placeholders), `the default faces read ${placeholders.join(" | ")}`);
      await page.evaluate(() => {
        window.queryLocalFonts = async () => [{ family: "Georgia" }, { family: "Courier New" }, { family: ".SF NS" }, { family: "Menlo" }, { family: "Georgia" }];
        Object.assign(installedFonts, { status: "unknown", families: [] });
      });
      await sans.focus();
      await waitFor("the font list", () => page.locator("#font-picker").isVisible(), 5000);
      const listed = await page.locator("#font-picker [role=option] .name").allTextContents();
      check(listed.slice(1).join(",") === "Courier New,Georgia,Menlo" && listed[0] === placeholders[0], `the list: ${listed.join(", ")}`);
      check((await page.locator("#font-picker [role=option] .default").count()) === 1, "the default row isn't marked");
      check((await page.evaluate(() => getComputedStyle(document.querySelectorAll("#font-picker .name")[2]).fontFamily.split(",")[0].replaceAll('"', ""))) === "Georgia", "a family isn't drawn in its own face");
      await page.keyboard.type("geo");
      check((await page.locator("#font-picker [role=option]").allTextContents()).join(",") === "Georgia", "the search doesn't narrow");
      await page.keyboard.press("Enter");
      await waitFor("Georgia", async () => (await page.evaluate(() => localStorage.getItem("look-sans-font"))) === "Georgia" && (await text("#look-sans-font")) === "Georgia");
      await press(page.locator("#look-mono-font"));
      await press(page.locator('#font-picker [data-font="Georgia"]'));
      check((await toastText()) === "\"Georgia\" isn't monospace · Code and terminal need a fixed-width font, so the current font was kept." && (await page.evaluate(() => localStorage.getItem("look-mono-font"))) === null, `a proportional pick: ${await toastText()}`);
      const sizes = await page.evaluate(() => {
        setLook("advanced", "true");
        setLook("terminal-size", "15px");
        looksChanged();
        const entry = shellTerm(active, { id: "terminal:font-probe" });
        const first = entry.term.options.fontSize;
        setLook("terminal-size", "17px");
        looksChanged();
        const second = entry.term.options.fontSize;
        shellTerms.delete("terminal:font-probe");
        entry.term.dispose();
        return [first, second].join("|");
      });
      check(sizes === "15|17", `a panel terminal's font size: ${sizes}`);
    } finally {
      await page.evaluate(() => {
        closeFontPicker();
        delete window.queryLocalFonts;
        Object.assign(installedFonts, { status: typeof window.queryLocalFonts === "function" ? "unknown" : "unavailable", families: [] });
        for (const name of ["sans-font", "mono-font", "advanced", "terminal-size"]) setLook(name, "");
        looksChanged();
        if (settingsOpen()) toggleSettings();
      });
    }
  });

  await step("app view: T3's tokens: Working in blue-500, the default theme's wireframe colors, a palette's code surface and JSON colors, and the Sort menu as a styled select", async () => {
    await resetTheme();
    try {
      const probe = (html, selector, prop) =>
        page.evaluate(([html, selector, prop]) => {
          const box = Object.assign(document.createElement("div"), { innerHTML: html });
          document.body.append(box);
          const read = getComputedStyle(box.querySelector(selector))[prop];
          box.remove();
          return read;
        }, [html, selector, prop]);
      check((await probe('<span class="status info">Working</span>', ".status", "color")) === "rgb(43, 127, 255)", "Working isn't blue-500");
      await page.evaluate(() => toggleSettings());
      await settingsNav("appearance");
      const wire = await page.evaluate(() => {
        const pane = (mode) => document.querySelector(`#theme-section .mode-tile[data-theme-mode="${mode}"] .wire-pane`).style;
        return [pane("dark").getPropertyValue("--w-sidebar"), pane("dark").getPropertyValue("--w-action"), pane("light").getPropertyValue("--w-message")].join("|");
      });
      check(wire === "#0f0f10|#8b9cff|#e4e4e7", `the wireframe: ${wire}`);
      const json = '<div class="theme-import"><div class="ti-editor"><span class="key">"a"</span></div></div>';
      check((await probe(json, ".key", "color")) === "rgb(21, 93, 252)", "a key isn't blue-600");
      await page.evaluate(() => toggleSettings());
      await page.evaluate(() => setTheme("grove"));
      await waitFor("Grove", async () => (await appearance()).endsWith("|grove"));
      const accent = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--app-theme-accent").trim());
      const key = await probe(json, ".key", "color");
      const accentRgb = await probe(`<span style="color: ${accent}">x</span>`, "span", "color");
      check(key === accentRgb, `under Grove a key is ${key}, the accent ${accentRgb}`);
      const code = await probe('<div class="codeblock">x</div>', ".codeblock", "backgroundColor");
      const codeBg = await probe(`<span style="background: ${await page.evaluate(() => colorsFor(themeById("grove"), resolvedAppearance()).codeBackground)}">x</span>`, "span", "backgroundColor");
      check(code === codeBg, `under Grove a code block is ${code}, its code surface ${codeBg}`);
    } finally {
      await page.evaluate(() => settingsOpen() && toggleSettings());
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

  await step("app view: tool rows read as T3's labels, MCP calls group by integration, and each kind opens to its own body", async () => {
    await sendPrompt("tools");
    await answered("Used the tools.");
    await press(lastTurn(".work > summary"));
    const group = lastTurn(".group > summary");
    const summary = (await group.locator(".label").textContent()).trim();
    check(summary === "Used Linear integration, ran 1 command, changed 1 file, and performed 6 other actions", `the group reads ${summary}`);
    check(await group.evaluate((el) => el.querySelector("svg.kind").innerHTML.includes("m15 12-8.373")), "the mixed group doesn't draw T3's hammer");
    await press(group);
    const labels = (await page.locator("#thread .turn:last-of-type .group .tool > summary .label").allTextContents()).map((l) => l.trim());
    const want = ["Read notes.md", "git status --short", "notes.md", "Searched **/*.rs", "Searched TODO in src", "rust parser crates", "https://example.com/docs", "Skill: deploy", "create issue", "list issues"];
    check(JSON.stringify(labels) === JSON.stringify(want), `the rows read ${labels.join(" | ")}`);
    const row = (label) => page.locator("#thread .turn:last-of-type .group .tool").filter({ has: page.locator("summary .label", { hasText: new RegExp(`^${label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`) }) }).first();
    const body = async (label) => {
      const r = row(label);
      if ((await r.getAttribute("open")) === null) await press(r.locator("summary"));
      return (await r.locator(".tool-body").innerText()).replace(/\s+/g, " ").trim();
    };
    const read = await body("Read notes.md");
    check(read === path.join(HOME, "work", "beta", "notes.md"), `the Read row opens to ${read}`);
    const mcp = await body("create issue");
    check(mcp === 'title Fix the parser team ENG {"id":"ENG-1"}', `the MCP row opens to ${mcp}`);
    const search = await body("rust parser crates");
    check(search === "rust parser crates nom on crates.io", `the search row opens to ${search}`);
    check((await row("rust parser crates").locator('a[href="https://crates.io/crates/nom"]').count()) === 1, "the search result isn't a link");
    const skill = await body("Skill: deploy");
    check(skill === "prod", `the Skill row opens to ${skill}`);
    await waitFor("the turn's changes", async () => (await page.locator("#thread .turn:last-of-type .changes").count()) > 0, 10000);
    const edit = await body("notes.md");
    check(edit === "notes.md Open diff", `the Edit row opens to ${edit}`);
    await press(row("notes.md").locator(".open-diff"));
    await waitFor("the diff panel", () => page.locator("#panel-body .file").first().isVisible());
    await tapOrClick("#toggle-panel");
    // The diff steps take beta for a thread that never opened Diff, so it forgets the scope it kept.
    await page.evaluate(() => {
      threadOf(active).diff = undefined;
      save(`gui-diff:${threadKey(active)}`, null);
    });
    return summary;
  });

  await step("app view: a message's images open in T3's media dialog with a close button, Previous/Next, ←/→ and a name (i/N) caption", async () => {
    await sendPrompt("pictures");
    await answered("Two dots.");
    await press(gui("#thread .turn:last-of-type .bubble .picture"));
    await waitFor("the media dialog", () => page.locator("#image-zoom").isVisible());
    const caption = () => text("#image-zoom figcaption");
    check((await caption()) === "Image #1 (1/2)", `the caption reads ${await caption()}`);
    check((await page.getAttribute("#image-zoom .media-close", "aria-label")) === "Close image preview", "the dialog has no close button");
    await press(gui('#image-zoom [aria-label="Next media"]'));
    check((await caption()) === "Image #2 (2/2)", `Next shows ${await caption()}`);
    if (!touch) {
      await page.keyboard.press("ArrowRight");
      check((await caption()) === "Image #1 (1/2)", `→ shows ${await caption()}`);
      await page.keyboard.press("ArrowLeft");
      check((await caption()) === "Image #2 (2/2)", `← shows ${await caption()}`);
    }
    await press(gui('#image-zoom [aria-label="Previous media"]'));
    check((await caption()) === "Image #1 (1/2)", `Previous shows ${await caption()}`);
    await press(gui("#image-zoom img"));
    check(await page.locator("#image-zoom").isVisible(), "a click on the image closed the dialog");
    await press(gui("#image-zoom .media-close"));
    await waitFor("the dialog to close", async () => !(await page.locator("#image-zoom").isVisible()));
  });

  await step("app view: a compaction reads as a Context compacted divider with its counts, after the /compact that asked for it", async () => {
    const placed = await page.evaluate(() => {
      const turns = turnsOf([
        { kind: "user", id: "u1", text: "hi", images: 0 },
        { kind: "text", id: "a1", text: "hello" },
        { kind: "compaction", id: "c1", at: "", before: 258550, after: 25325 },
        { kind: "user", id: "u2", text: "/compact", images: 0 },
        { kind: "note", id: "n1", text: "Compacted" },
      ]);
      return [turns[0].dividers.length, turns[1].dividers[0]?.id, turns[1].notes.length, new DOMParser().parseFromString(dividerHtml(turns[1].dividers[0]), "text/html").body.textContent];
    });
    check(JSON.stringify(placed) === JSON.stringify([0, "c1", 0, "Context compacted 259K25.3K tokens"]), `the compaction lands as ${JSON.stringify(placed)}`);
  });

  await step("app view: replies draw images, sanitized HTML and mermaid diagrams as T3 does", async () => {
    await sendPrompt("media");
    const answer = lastTurn(".answer .assistant");
    await waitFor("the reply", async () => (await page.locator("#thread .assistant", { hasText: "Two images" }).count()) > 0, 10000);
    await waitFor("both images", () => answer.evaluate((el) => [...el.querySelectorAll("img.md-image")].filter((i) => i.complete && i.naturalWidth === 48).length === 2), 8000);
    check((await answer.locator(".md-image-missing").textContent()).trim() === "Image unavailable · gone", "the missing image has no unavailable note");
    await press(answer.locator('img.md-image[alt="dot"]'));
    await waitFor("the media dialog", () => page.locator("#image-zoom").isVisible());
    check((await text("#image-zoom figcaption")) === "dot (1/2)", `the caption reads ${await text("#image-zoom figcaption")}`);
    await press(gui("#image-zoom .media-close"));
    const details = answer.locator("details.md-details");
    check((await details.locator("summary").textContent()).trim() === "More" && (await details.getAttribute("open")) === null, "the <details> isn't a closed fold named More");
    await press(details.locator("summary"));
    check((await details.locator("strong").textContent()) === "bold", "the fold's markdown didn't render");
    const html = await answer.evaluate((el) => [el.querySelectorAll("kbd").length, el.querySelectorAll("br").length, el.querySelectorAll("script, [onclick], span[style]:not(.ficon), .evil").length, /alert\(1\)/.test(el.textContent), el.textContent.includes("plain")]);
    check(JSON.stringify(html) === "[2,1,0,false,true]", `the HTML renders as ${JSON.stringify(html)}`);
    const diagram = answer.locator(".mermaid-diagram");
    await waitFor("the diagram", async () => (await diagram.locator(".diagram svg").count()) === 1, 10000);
    check(/Start/.test(await diagram.locator(".diagram svg").textContent()), "the diagram doesn't show its nodes");
    await press(diagram.locator('[aria-label="Show code"]'));
    await waitFor("the source", async () => (await answer.locator(".codeblock .lang", { hasText: "mermaid" }).count()) === 1);
    await press(answer.locator('[aria-label="Show diagram"]'));
    await waitFor("the diagram again", async () => (await answer.locator(".mermaid-diagram .diagram svg").count()) === 1, 10000);
    await press(answer.locator(".mermaid-diagram .diagram"));
    await waitFor("the expanded diagram", async () => (await text("#image-zoom figcaption")) === "Mermaid diagram");
    await press(gui("#image-zoom .media-close"));
  });

  await step("app view: a file chip opens in the editor on ⌘-click, and its menu opens, reveals and copies its path", async () => {
    const chip = lastTurn('.answer .file-chip[data-file-path="notes.md"]');
    const log = () => (fs.existsSync(path.join(HOME, "open.log")) ? fs.readFileSync(path.join(HOME, "open.log"), "utf8") : "");
    await waitFor("the machine's editors", () => page.evaluate(() => editorsOn(active.machine).some((e) => e.id === "zed")), 8000);
    // This Mac's own editors list too, so the step picks the stack's Zed and puts the choice back after.
    const chosen = await page.evaluate(() => [load(`gui-editor:${active.machine}`) ?? null, save(`gui-editor:${active.machine}`, "zed")][0]);
    const before = log().split("\n").length;
    if (!touch) {
      await chip.click({ modifiers: [(await page.evaluate(() => IS_MAC)) ? "Meta" : "Control"] });
      await waitFor("notes.md in Zed", async () => log().split("\n").length > before && /Zed\.app .*\/work\/beta\/notes\.md$/m.test(log()));
      check(!(await page.locator("#panel").isVisible()), "⌘-click also opened the File tab");
    }
    if (touch) await chip.dispatchEvent("contextmenu");
    else await chip.click({ button: "right" });
    await waitFor("the chip's menu", () => page.locator("#side-menu").isVisible());
    const items = (await page.locator("#side-menu button .label").allTextContents()).join(" | ");
    check(items === "Open in Zed | Reveal in Finder | Copy relative path | Copy full path", `the menu reads ${items}`);
    await press(page.locator("#side-menu button", { hasText: "Copy relative path" }));
    const copiedPath = await clipboard();
    check(copiedPath === null || copiedPath === "notes.md:1", `the clipboard holds ${copiedPath}`);
    await page.evaluate((v) => save(`gui-editor:${active.machine}`, v), chosen);
    return items;
  });

  await step("app view: a prompt's Copy prompt link copies T3's /new link for it", async () => {
    const user = lastTurn(".user");
    if (!touch) await user.hover();
    const button = user.locator('[data-copy-link]');
    check((await button.getAttribute("aria-label")) === "Copy prompt link", "the prompt has no Copy prompt link");
    await press(button);
    await waitFor("the copied check", async () => (await user.locator("[data-copy-link].copied").count()) === 1, 1500);
    const link = await clipboard();
    check(link === null || link === `${HUB}/new?prompt=media`, `the clipboard holds ${link}`);
  });

  await step("app view: selecting reply text offers Cite, which puts a quote chip with a comment in the composer and sends it as T3's citation", async () => {
    const source = lastTurn(".answer .assistant");
    await source.evaluate((el) => {
      const node = [...el.querySelectorAll("p")].find((p) => p.textContent.startsWith("Two images")).firstChild;
      const range = document.createRange();
      range.setStart(node, 0);
      range.setEnd(node, "Two images".length);
      getSelection().removeAllRanges();
      getSelection().addRange(range);
    });
    await waitFor("the Cite button", () => page.locator("#cite-selection").isVisible(), 3000);
    check((await text("#cite-selection")) === "Cite" && (await page.getAttribute("#cite-selection", "aria-label")) === "Cite selection in composer", `the button reads ${await text("#cite-selection")}`);
    await press(page.locator("#cite-selection"));
    await waitFor("the comment editor", () => page.locator('#cite-popover textarea[placeholder="Add an optional comment..."]').isVisible());
    await page.fill("#cite-popover textarea", "why two?");
    await page.press("#cite-popover textarea", "Enter");
    const row = page.locator("#attachments .file", { hasText: "why two?" });
    await waitFor("the quote chip", () => row.isVisible());
    check((await row.locator("[data-attachment-action]").getAttribute("aria-label")) === "Edit citation comment", "the chip can't edit its comment");
    await sendPrompt("explain");
    const bubble = lastTurn(".user .bubble");
    await waitFor("the sent quote", async () => (await bubble.locator(".cite-chip").count()) === 1, 10000);
    check((await bubble.textContent()).replace(/\s+/g, " ").trim() === "explain why two?", `the prompt reads ${await bubble.textContent()}`);
    const sent = await page.evaluate(() => threadOf(active).items.filter((i) => i.kind === "user").at(-1).text);
    check(/^explain \[assistant-quote-1\]\n\n<assistant_citations>\n/.test(sent) && /"text": "Two images"/.test(sent) && /"comment": "why two\?"/.test(sent), `claude got ${sent.slice(0, 120)}`);
    await press(bubble.locator(".cite-chip"));
    await waitFor("the quote", () => page.locator("#cite-popover blockquote").isVisible());
    check((await text("#cite-popover blockquote")) === "Two images", `the quote reads ${await text("#cite-popover blockquote")}`);
    await press(page.locator("#cite-popover [data-cite-source-go]"));
    await waitFor("the source selected", () => page.evaluate(() => getSelection().toString() === "Two images"), 3000);
    check(!(await page.locator("#cite-popover").isVisible()), "the quote stays open after Go to source");
    await page.evaluate(() => getSelection().removeAllRanges());
  });

  await step("app view: Edit from here gives back a prompt's pictures as attachments", async () => {
    await sendPrompt("pictures");
    // An earlier step already drew "Two dots.", so the answer waited for is the newest turn's.
    await waitFor("the pictures' answer in the newest turn", () => page.evaluate(() => {
      const turn = [...document.querySelectorAll("#thread .lane > .turn")].at(-1);
      return /compare these/.test(turn?.querySelector(".user")?.textContent ?? "") && /Two dots\./.test(turn.textContent);
    }), 10000);
    const user = lastTurn(".user");
    if (!touch) await user.hover();
    await press(user.locator("[data-rewind]"));
    await waitFor("the dialog", () => page.locator(".tl-dialog").isVisible());
    check((await text(".tl-dialog .about")).startsWith("Rewind chat to before this message. Your prompt and attachments return to the composer."), `the dialog says ${await text(".tl-dialog .about")}`);
    await press(page.locator('.tl-dialog button[value="chat"]'));
    await waitFor("the prompt back", async () => (await page.inputValue("#prompt")) === "compare these", 10000);
    await waitFor("the pictures back", async () => (await page.locator("#attachments .tile img").count()) === 2, 10000);
    await page.evaluate(() => clearComposer(active));
  });

  await step("app view: an image named by its upload's path draws the picture, not only the path", async () => {
    // A 1x1 PNG, saved where the node keeps its uploads, as a client's attachment lands there.
    const shot = path.join(DIR, "node", "uploads", "sweep-shot.png");
    fs.mkdirSync(path.dirname(shot), { recursive: true });
    fs.writeFileSync(shot, Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==", "base64"));
    await sendPrompt(`what is in ${shot}`);
    await waitFor("the upload's picture", async () => /^data:image\/png;base64,/.test((await lastTurn(".user .pictures img").getAttribute("src").catch(() => "")) ?? ""), 15000);
    check((await lastTurn(".user .body").textContent()).includes(shot), "the message doesn't keep the path");
  });

  await step("app view: a fork starts with a Forked from conversation divider that opens its source", async () => {
    const tabs = await tabCount();
    const source = await page.evaluate(() => active.view.session);
    const answer = lastTurn(".answer");
    if (!touch) await answer.hover();
    await press(answer.locator("[data-fork]"));
    await waitFor("the forked thread", async () => (await tabCount()) === tabs + 1 && (await page.evaluate(() => threadOf(active).items.length)) > 0, 10000);
    const divider = gui("#thread .system-divider button.pill");
    await waitFor("the fork divider", () => divider.isVisible(), 10000);
    check((await divider.textContent()).trim() === "Forked from conversation" && (await divider.getAttribute("aria-label")) === "Open source conversation", `the divider reads ${await divider.textContent()}`);
    const fork = await page.evaluate(() => active.key);
    await press(divider);
    await waitFor("the source thread", async () => (await page.evaluate(() => active.view.session)) === source);
    await page.evaluate((key) => closeTab(tabs.find((t) => t.key === key)), fork);
    await waitFor("the fork to close", async () => (await tabCount()) === tabs);
    await toThread("beta");
  });

  await step("app view: a subagent's thread opens under a Subagent of divider that leads back to its parent", async () => {
    await sendPrompt("agent");
    await answered("The agent reported back.");
    await press(lastTurn(".work > summary"));
    await press(gui('#thread .turn:last-of-type [data-subagent]'));
    await waitFor("the subagent's answer", async () => (await page.locator("#thread .assistant", { hasText: "The subagent found it in src/parse.rs." }).count()) === 1);
    const divider = gui("#thread .system-divider button.pill");
    const label = `${await divider.locator(".what").textContent()} ${(await divider.locator(".detail").textContent()).replace(/\s+/g, " ")}`;
    const parent = await page.evaluate(() => threadTitle(active));
    check(label === `Subagent of · ${parent}` && (await divider.getAttribute("aria-label")) === "Open parent thread", `the divider reads ${label}`);
    await press(divider);
    await waitFor("the parent thread", async () => (await page.locator("#thread .assistant", { hasText: "The agent reported back." }).count()) > 0 && !(await page.evaluate(() => !!subagentShown())));
    return label;
  });

  await step("app view: resting on a PR link previews the pull request", async () => {
    try {
      // The subagent step before this one goes back to whichever thread it started in, which can be beta's.
      await toThread("alpha");
      await sendPrompt("prlink");
      await answered("Opened the pull request");
      if (touch) return "a touch screen has no hover";
      await gui('#thread .turn:last-of-type a[data-pr]').hover();
      await waitFor("the preview card", () => page.locator("#pr-card").isVisible(), 8000);
      const card = await page.evaluate(() => [...document.querySelectorAll("#pr-card .head > *, #pr-card .title, #pr-card .who > *")].map((el) => el.textContent.trim()).filter(Boolean).join(" | "));
      check(/^example\/alpha \| #42 \| · \| Open \| Add notes \| sweeper \| · \| opened .+ ago$/.test(card), `the card reads ${card}`);
      await page.mouse.move(5, 5);
      await waitFor("the card to close", async () => !(await page.locator("#pr-card").isVisible()), 2000);
    } finally {
      // The steps after this block expect alpha, whether or not the card showed.
      await toThread("alpha");
    }
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
    // A file waits for its grammar rather than draw in plain colors first.
    await waitFor("the added lines", async () => (await page.locator("#panel-body .dl.add").count()) >= 2, 10000).catch(() => {});
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
    if (touch) {
      await tapOrClick("#head-more");
      check(await page.locator("#panel-menu button", { hasText: "Open PR #42" }).isVisible(), "the Options menu has no Open PR #42 after the session linked it");
      await tapOrClick("#head-more");
    }
    check(!(await page.locator("#chat-head #pr-chip, #chat-head .chip").count()), "the header has a PR chip, which T3 keeps in thread details");
    check(/#42/.test(await text("#composer-footer .pr")), "the composer footer doesn't show #42");
    await tapOrClick("#composer-footer .pr");
    const prPanelSettled = async () => /#42/.test(await text("#panel-body")) && !/Loading/.test(await text("#panel-body"));
    await waitFor("the PR panel", prPanelSettled, 15000);
    check(await page.locator('#panel-tabs [data-panel="pr"].active').isVisible(), "the PR tab is not the active one");
    const shown = (await text("#panel-body")).slice(0, 80);
    await tapOrClick(await threadPrLink(41));
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
  // pinned's JSON keeps its keys in the order the code writes them, which a check shouldn't hang on.
  const sortedJson = (value) => JSON.stringify(value, (_, v) => (v && typeof v === "object" && !Array.isArray(v) ? Object.fromEntries(Object.entries(v).sort(([a], [b]) => (a < b ? -1 : 1))) : v));
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
    await waitFor("the code turn's changes", async () => (await page.locator("#thread .turn:last-of-type", { hasText: "I wrote app.js" }).locator("summary .open-diff").count()) === 1, 15000);
    await press(page.locator("#thread .turn:last-of-type summary .open-diff"));
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
    check(/app\.js:L2 to L3 rename greet/.test(await text("#attachments .file .name")), `the composer's chip reads ${await text("#attachments")}`);
    const sent = await page.evaluate(() => threadOf(active).attachments.map((a) => a.text).join("\n"));
    check(sent.startsWith('<review_comment filePath="app.js" lines="L2 to L3">\nrename greet\n```diff\n+function greet(name) {\n+  return "hi " + name;\n```\n</review_comment>'), `what goes out: ${JSON.stringify(sent)}`);
    await press(gui("#panel-body [data-comment-delete]"));
    await waitFor("the comment to go", async () => !(await page.locator("#panel-body .dcomment").count()) && !(await page.locator("#attachments .file").count()));
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
    await waitFor("the loading bar", async () => (await page.getAttribute("#panel-body .loading-bar", "data-loading")) === "true", 2000);
    check((await page.getAttribute('#panel-body [data-nav="reload"]', "aria-label")) === "Stop", "while loading, Reload isn't Stop");
    await waitFor("the page to load", async () => (await page.getAttribute("#panel-body .loading-bar", "data-loading")) === "false", 10000);
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
    await press(gui("#panel-body [data-device]"));
    await press(gui('#panel-menu button:has(.label:text-is("iPhone SE"))'));
    const width = () => page.locator("#panel-body iframe").evaluate((f) => f.offsetWidth);
    await waitFor("the iPhone SE size", async () => (await width()) === 375);
    await press(gui('#panel-body [data-device-do="rotate"]'));
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
    await tapOrClick("#composer-footer .pr");
    await waitFor("#42's details", async () => /Add notes/.test(await text("#panel-body .pr-head h3")), 15000);
    const meta = await text("#panel-body .meta");
    check(/monalisa/.test(meta) && /hubot/.test(meta) && /documentation/.test(meta), `reviewers and labels: ${meta}`);
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
    await tapOrClick(await threadPrLink(41));
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
    await press(gui("#panel-body [data-device]"));
    await press(menuItem("iPhone SE"));
    await press(gui('#panel-body [data-device-do="lock"]'));
    check((await page.getAttribute('#panel-body [data-device-do="lock"]', "title")) === "Unlock aspect ratio", "the lock didn't lock");
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
    const source = await page.evaluate(() => miniSource(panelOf(active).surfaces.find((x) => x.id === panelOf(active).pip)));
    check(Math.abs(box.width / box.height - source.w / source.h) < 0.03 && (Math.round(Math.max(box.width, box.height)) === 320 || Math.round(box.width) === 240), `the floating page is ${box.width}×${box.height} for a ${source.w}×${source.h} page`);
    await press(gui('#preview-pip [data-pip="panel"]'));
    await waitFor("the page back in the panel", async () => (await page.locator("#panel-body .viewport iframe").count()) === 1 && !(await page.locator("#preview-pip").isVisible()));
    await closePanelIfOpen();
  });

  await step("browser: Refresh spins while a page loads, and the bar sweeps to full width and fades", async () => {
    await openPanelTo("browser", "Browser");
    const port = readLog("serve.port").trim();
    await page.fill("#panel-body .url-bar input", `http://localhost:${port}/slow`);
    await page.press("#panel-body .url-bar input", "Enter");
    const reload = '#panel-body [data-nav="reload"]';
    await waitFor("the page to start loading", async () => (await page.getAttribute("#panel-body .loading-bar", "data-loading")) === "true", 3000);
    check((await page.getAttribute(reload, "aria-label")) === "Stop" && (await page.getAttribute(reload, "title")) === "Loading…", `while loading, Refresh reads ${await page.getAttribute(reload, "aria-label")} / ${await page.getAttribute(reload, "title")}`);
    const spin = await page.locator(`${reload} .i`).evaluate((i) => getComputedStyle(i).animationName);
    check(spin === "browser-spin", `the Refresh icon doesn't spin: ${spin}`);
    check((await page.locator(`${reload} .i path`).first().getAttribute("d")) === "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8", "the Refresh icon isn't RefreshCw");
    const seen = await page.locator("#panel-body .loading-bar").evaluate((bar) => new Promise((done) => {
      const samples = [];
      const scale = () => new DOMMatrix(getComputedStyle(bar).transform).a;
      const tick = () => {
        samples.push({ loading: bar.isConnected && bar.dataset.loading === "true", scale: scale(), opacity: Number(getComputedStyle(bar).opacity), glow: getComputedStyle(bar).boxShadow });
        if (samples.length > 400 || (!samples.at(-1).loading && samples.at(-1).opacity === 0)) return done(samples);
        requestAnimationFrame(tick);
      };
      tick();
    }));
    const loading = seen.filter((x) => x.loading);
    const after = seen.filter((x) => !x.loading);
    check(loading.length && loading.every((x) => x.scale >= 0.03 && x.scale <= 0.91 && x.opacity === 1), `while loading the bar ran ${loading.map((x) => x.scale.toFixed(2)).join(",")}`);
    check(loading[0].glow !== "none", "the bar has no glow");
    check(after.some((x) => x.scale > 0.98 && x.opacity > 0.5), `the bar didn't sweep to full width before fading: ${after.map((x) => `${x.scale.toFixed(2)}@${x.opacity.toFixed(2)}`).join(" ")}`);
    check(after.at(-1)?.opacity === 0, "the bar didn't fade out");
    check((await page.getAttribute(reload, "aria-label")) === "Refresh", "Refresh stayed Stop");
    return `${loading.length} loading frames, ${after.length} to fade`;
  });

  await step("browser: the empty tab lists recent pages with their icon, title and age, and only the thread's own servers", async () => {
    const port = readLog("serve.port").trim();
    await openPanelTo("browser", "Browser");
    await page.fill("#panel-body .url-bar input", `http://localhost:${port}/a`);
    await page.press("#panel-body .url-bar input", "Enter");
    await waitFor("Page A's title", async () => (await text("#panel-tabs .ptab.active")) === "Page A", 10000);
    await fromPlus("Browser");
    const recent = `#panel-body .servers .row:has(button[data-url="http://localhost:${port}/a"])`;
    await waitFor("Page A among Recently used", async () => (await page.locator(recent).count()) === 1, 5000);
    check(!/No preview yet/.test(await text("#panel-body .browser-empty")), "the title stays above the lists");
    check((await page.locator("#panel-body .browser-empty .group").first().locator(".i").count()) === 1, "the heading has no icon");
    check((await text(`${recent} b`)) === "Page A", `the card's title is ${await text(`${recent} b`)}`);
    check(new RegExp(`^localhost:${port}/a · (just now|\\d+s ago|\\d+m ago)$`).test(await text(`${recent} b + span`)), `the card's subline is ${await text(`${recent} b + span`)}`);
    await waitFor("Page A's icon", async () => /^data:image\/png/.test((await page.getAttribute(`${recent} img.favicon`, "src").catch(() => "")) ?? ""), 5000);
    const server = `#panel-body section:has(.group:text-is("Local servers")) button[data-url="http://localhost:${port}"]`;
    await waitFor("the server's card", async () => (await page.locator(server).count()) === 1, 12000);
    check((await text(`${server} b + span`)) === `localhost:${port}`, `the server's subline is ${await text(`${server} b + span`)}`);
    check((await page.locator(`${server} .mockup, ${server} img.favicon`).count()) >= 1 && !(await page.locator(`${server} [data-icon], ${server} svg.i`).count()), "the server's card shows an icon, not its page's");
    check(/Select a live local server to open it in this browser tab\./.test(await text("#panel-body .browser-empty")), "no line under the servers");
    if (!touch) await page.hover(recent);
    await press(gui(`${recent} [data-forget]`));
    await waitFor("Page A to leave Recently used", async () => (await page.locator(recent).count()) === 0);
    check(!(await page.evaluate((u) => browserRecents.some((e) => e.url === u), `http://localhost:${port}/a`)), "the removed page is still saved");
    const kept = await page.evaluate(() => JSON.stringify(browserRecents));
    try {
      await page.evaluate(() => { browserRecents = []; activate(tabs.find((t) => t.view.cwd.endsWith("/beta"))); });
      await waitFor("beta", async () => /beta/.test(await text("#chat-head .crumb")));
      await openPanelTo("browser", "Browser");
      await waitFor("beta's ports", () => page.evaluate(() => Array.isArray(threadOf(active).ports)), 8000);
      await page.evaluate(() => renderBrowser(shownSurface(active), true));
      const blank = await text("#panel-body .browser-empty");
      check(/^No preview yet\s*Type a URL above, or run a dev script\. Browser-ready localhost servers will show up here automatically\.\s*(This thread runs on .*)?$/.test(blank), `beta's empty tab reads: ${blank}`);
      check((await page.locator("#panel-body .browser-empty .media .i").count()) === 1, "no globe over the blank state");
      check(!(await page.locator(`#panel-body [data-url="http://localhost:${new URL(HUB).port}"]`).count()), "beta lists the machine's other listening ports");
    } finally {
      await page.evaluate((k) => { browserRecents = JSON.parse(k); saveRecents(); activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))); }, kept);
      await waitFor("alpha", async () => /alpha/.test(await text("#chat-head .crumb")));
      await closePanelIfOpen();
    }
  });

  await step("browser: the device toolbar opens Responsive at the page's size, lists DevTools' devices, rotates, resizes from its rails and closes", async () => {
    await openPanelTo("browser", "Browser");
    const port = readLog("serve.port").trim();
    if (!(await page.locator("#panel-body iframe").count())) {
      await page.fill("#panel-body .url-bar input", `http://localhost:${port}/a`);
      await page.press("#panel-body .url-bar input", "Enter");
    }
    await press(gui('#panel-body [data-nav="more"]'));
    await press(menuItem("Show device toolbar"));
    const size = async () => [Number(await page.inputValue('#panel-body [data-size="w"]')), Number(await page.inputValue('#panel-body [data-size="h"]'))];
    const frame = () => page.locator("#panel-body iframe").evaluate((f) => [f.offsetWidth, f.offsetHeight]);
    check((await text("#panel-body [data-device]")) === "Responsive", `the preset reads ${await text("#panel-body [data-device]")}`);
    const [w, h] = await size();
    check(w >= 240 && h >= 240 && JSON.stringify(await frame()) === JSON.stringify([w, h]), `Responsive is ${w}×${h}, the page ${(await frame()).join("×")}`);
    const barWidth = (await rect("#panel-body .device-bar")).width;
    check((await page.locator("#panel-body .device-bar .dims-label").count()) === (barWidth >= 560 ? 1 : 0), `"Dimensions" at a ${barWidth}px toolbar`);
    await press(gui("#panel-body [data-device]"));
    const labels = await menuLabels();
    check(labels.length === 18 && labels[0] === "Responsive" && labels[1] === "iPhone SE" && labels.at(-1) === "Nest Hub Max" && labels.includes("Samsung Galaxy S20 Ultra"), `presets: ${labels.join(" | ")}`);
    check((await text("#panel-menu .group")) === "Standard" && (await text('#panel-menu button:has(.label:text-is("iPad Air")) .hint')) === "820 × 1180", "the presets aren't grouped with their sizes");
    await page.keyboard.press("Escape");
    await press(gui('#panel-body [data-device-do="rotate"]'));
    await waitFor("Responsive to rotate", async () => JSON.stringify(await size()) === JSON.stringify([h, w]));
    const link = async () => page.locator('#panel-body [data-device-do="lock"] path').first().getAttribute("d");
    check((await link()) === "M15 7h2a5 5 0 0 1 0 10h-2", "the open lock isn't Unlink2");
    await press(gui('#panel-body [data-device-do="lock"]'));
    check((await link()) === "M9 17H7A5 5 0 0 1 7 7h2", `the shut lock isn't Link2: ${await link()}`);
    await press(gui('#panel-body [data-device-do="lock"]'));
    await page.locator("#panel-body .rail.e").focus();
    await page.keyboard.press("ArrowRight");
    await waitFor("the east rail's arrow key", async () => (await size())[0] === h + 10);
    if (!touch) {
      const rail = await rect("#panel-body .rail.e");
      await page.mouse.move(rail.x + rail.width / 2, rail.y + rail.height / 2);
      await page.mouse.down();
      await page.mouse.move(rail.x + rail.width / 2 - 30, rail.y + rail.height / 2, { steps: 4 });
      check(/^\d+ × \d+$/.test(await text("#panel-body .device-size")), "no size readout while dragging");
      await page.mouse.up();
      await waitFor("the drag to narrow the page", async () => (await size())[0] < h + 10 && (await frame())[0] === (await size())[0]);
    }
    check((await page.locator("#panel-body .rail").count()) === 5, "the page has no resize rails");
    await press(gui('#panel-body [data-device-do="close"]'));
    await waitFor("the toolbar to close", async () => !(await page.locator("#panel-body .device-bar").count()) && !(await page.locator("#panel-body .viewport.device").count()));
    await closePanelIfOpen();
    return `Responsive ${w}×${h}`;
  });

  await step("browser: Settings pick the address bar's search engine and where links open, and terminal links follow them across wrapped rows", async () => {
    await page.evaluate(() => toggleSettings());
    await tapOrClick('#settings-nav [data-page="general"]');
    await waitFor("the Browser settings", () => page.locator("#browser-settings").isVisible());
    check((await page.inputValue("#browser-links")) === "system", "links don't open in the default browser at first");
    const engines = await page.locator("#browser-search option").allTextContents();
    check(engines.join("|") === "DuckDuckGo|Google|Bing|Brave Search|Kagi|Ecosia|Custom", `engines: ${engines.join(" | ")}`);
    await page.selectOption("#browser-search", "custom");
    await waitFor("the custom URL field", () => page.locator("#browser-search-url").isVisible());
    check(/Searches use DuckDuckGo until then/.test(await text("#browser-settings")), "an empty custom URL isn't flagged");
    await page.fill("#browser-search-url", "https://search.example.test/find?q=%s");
    await page.locator("#browser-search-url").dispatchEvent("change");
    await waitFor("the custom URL to be taken", async () => !/Searches use DuckDuckGo/.test(await text("#browser-settings")));
    await page.evaluate(() => toggleSettings());
    await openPanelTo("browser", "Browser");
    const port = readLog("serve.port").trim();
    const typed = async (value) => {
      await page.fill("#panel-body .url-bar input", value);
      await page.press("#panel-body .url-bar input", "Enter");
      await sleep(150);
      return page.evaluate(() => shownSurface(active).url);
    };
    check((await typed("pinned sweep words")) === "https://search.example.test/find?q=pinned%20sweep%20words", "a query didn't use the custom engine");
    check((await typed(`localhost:${port}/a`)) === `http://localhost:${port}/a`, "a loopback host didn't go over http");
    check((await typed("mailto:someone@example.test")) === `http://localhost:${port}/a`, "a mailto: address went somewhere");
    await page.evaluate(() => toggleSettings());
    await tapOrClick('#settings-nav [data-page="general"]');
    await page.selectOption("#browser-search", "kagi");
    await page.evaluate(() => toggleSettings());
    await openPanelTo("browser", "Browser");
    check((await typed("pinned")) === "https://kagi.com/search?q=pinned", "a query didn't use Kagi");
    await typed(`localhost:${port}/a`);
    await page.evaluate(() => { save("gui-browser-search", ""); save("gui-browser-search-url", ""); });

    // The steps after this one count terminals from 1, so this one closes its own whatever happens.
    try {
      await fromPlus("Terminal");
      await waitFor("the shell", async () => (await page.locator("#panel-body .xterm-rows").count()) === 1, 10000);
      await page.locator("#panel-body .xterm-helper-textarea").focus();
      const wrapped = `http://localhost:${port}/a?w=${"q".repeat(220)}END`;
      // Quoted, as the panel's shell can be zsh, which takes the ? for a glob.
      await page.keyboard.type(`clear; echo '${wrapped}'; echo ./notes.md:1\n`);
      const spot = (needle, back) => page.evaluate(([n, b]) => {
        for (const row of document.querySelectorAll("#panel-body .xterm-rows > div")) {
          const i = row.textContent.indexOf(n);
          if (i < 0 || row.textContent.includes("echo")) continue;
          const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
          let seen = 0;
          for (let t = walker.nextNode(); t; t = walker.nextNode()) {
            if (seen + t.length > i - b) {
              const range = document.createRange();
              range.setStart(t, i - b - seen);
              range.setEnd(t, i - b - seen + 1);
              const r = range.getBoundingClientRect();
              return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
            }
            seen += t.length;
          }
        }
        return null;
      }, [needle, back]);
      // The typed command shows the address too, so wait for the row echo printed.
      await waitFor("the wrapped link", async () => !!(await spot("END", 3)) && !!(await spot("notes.md:1", -3)), 10000);
      const clickAt = async (at, modifiers = []) => {
        await page.mouse.move(at.x, at.y);
        // xterm looks for a link once the pointer rests on a row, and shows the hand when it has one.
        await waitFor("the hand over the link", () => page.evaluate(([x, y]) => getComputedStyle(document.elementFromPoint(x, y)).cursor === "pointer", [at.x, at.y]), 5000);
        for (const m of modifiers) await page.keyboard.down(m);
        await page.mouse.click(at.x, at.y);
        for (const m of modifiers) await page.keyboard.up(m);
      };
      const tail = await spot("END", 3);
      check(tail, "the wrapped link's last row isn't on screen");
      const popup = page.context().waitForEvent("page", { timeout: 8000 });
      await clickAt(tail);
      const outside = await popup;
      check(outside.url() === wrapped, `a plain click with the default setting opened ${outside.url().slice(0, 60)} outside`);
      await outside.close();
      check(await openTab("terminal").isVisible(), "the default setting opened the preview");
      await page.evaluate(() => save("gui-browser-links", "app"));
      await clickAt(tail);
      await waitFor("the wrapped link in the preview", async () => (await page.evaluate(() => shownSurface(active)?.url)) === wrapped, 10000);
      await page.click('#panel-tabs .ptab[data-panel="terminal"]');
      await waitFor("the shell again", async () => (await page.locator("#panel-body .xterm-rows").count()) === 1);
      const popup2 = page.context().waitForEvent("page", { timeout: 8000 });
      await clickAt(await spot("END", 3), [(await page.evaluate(() => IS_MAC)) ? "Meta" : "Control"]);
      const forced = await popup2;
      check(forced.url() === wrapped, "⌘-click didn't open the link outside");
      await forced.close();
      await clickAt(await spot("notes.md:1", -3));
      await waitFor("notes.md in a File tab", async () => /notes\.md/.test(await text("#panel-tabs .ptab.active")), 10000);
    } finally {
      await page.evaluate(() => save("gui-browser-links", ""));
      for (let n = await page.locator('#panel-tabs .ptab[data-panel="terminal"]').count(); n > 0; n--) {
        await page.click('#panel-tabs .ptab[data-panel="terminal"]');
        await press(gui('#panel-tabs .ptab[data-panel="terminal"] .lead'));
      }
      await closePanelIfOpen();
    }
  });

  await step("browser: an unreachable address and a file show T3's screens in place of the page", async () => {
    await openPanelTo("browser", "Browser");
    const port = readLog("serve.port").trim();
    await page.fill("#panel-body .url-bar input", "http://localhost:1/");
    await page.press("#panel-body .url-bar input", "Enter");
    await waitFor("the unreachable screen", () => gui("#panel-body .page-failure").isVisible(), 10000);
    const failure = await text("#panel-body .page-failure");
    check(/This site can’t be reached/.test(failure) && /localhost:1: Connection refused\./.test(failure) && /ERR_CONNECTION_REFUSED/.test(failure), `the screen reads ${failure}`);
    check(!(await page.locator("#panel-body iframe").count()), "the page still shows under the screen");
    await press(gui('#panel-body [data-failure="details"]'));
    check(/Try:\s*Checking your connection/.test(await text("#panel-body .page-failure")) && (await text('#panel-body [data-failure="details"]')) === "Hide details", "Details shows no advice");
    await press(gui('#panel-body [data-failure="reload"]'));
    await waitFor("the screen again after Reload", () => gui("#panel-body .page-failure").isVisible(), 10000);
    await page.fill("#panel-body .url-bar input", `http://localhost:${port}/report.zip`);
    await page.press("#panel-body .url-bar input", "Enter");
    await waitFor("the file screen", async () => /This file can’t be shown here/.test(await text("#panel-body .page-failure")), 10000);
    check(new RegExp(`report\\.zip from localhost:${port} was downloaded instead\\.`).test(await text("#panel-body .page-failure")), `the file screen reads ${await text("#panel-body .page-failure")}`);
    check((await page.getAttribute("#panel-body .page-failure a[download]", "href")) === `http://localhost:${port}/report.zip` && (await text('#panel-body [data-failure="open"]')) === "Open in browser", "the file screen has no Download and Open in browser");
    await page.fill("#panel-body .url-bar input", `http://localhost:${port}/a`);
    await page.press("#panel-body .url-bar input", "Enter");
    await waitFor("the page back", async () => /page a/.test((await page.frameLocator("#panel-body iframe").locator("body").textContent().catch(() => "")) ?? ""), 10000);
    check(!(await page.locator("#panel-body .page-failure").count()), "the screen stayed over the page");
    await closePanelIfOpen();
  });

  await step("browser: zooming fades the new zoom in at the page's top right, then out", async () => {
    await openPanelTo("browser", "Browser");
    await press(gui('#panel-body [data-nav="more"]'));
    await press(gui('#panel-menu [data-zoom="0.1"]'));
    const pill = "#panel-body .viewport .zoom-pill";
    await waitFor("the zoom pill", async () => (await page.locator(pill).evaluate((p) => Number(getComputedStyle(p).opacity)).catch(() => 0)) === 1, 2000);
    check((await text(pill)) === "110%", `the pill reads ${await text(pill)}`);
    const box = await rect(pill);
    const view = await rect("#panel-body .viewport");
    check(Math.abs(view.right - box.right - 12) <= 1 && Math.abs(box.top - view.top - 12) <= 1, `the pill sits at ${box.right - view.right},${box.top - view.top}`);
    check(!/Zoom 110%/.test(await text("#panel-note").catch(() => "")), "the zoom still flashes in the bottom note");
    await waitFor("the pill to fade", async () => (await page.locator(pill).evaluate((p) => Number(getComputedStyle(p).opacity))) === 0, 4000);
    await press(gui('#panel-menu [data-zoom="0"]'));
    await page.keyboard.press("Escape");
    await closePanelIfOpen();
  });

  await step("browser: the floating preview keeps the page's shape as it resizes from a corner, moves by its handle, and offers T3's controls", async () => {
    await openPanelTo("browser", "Browser");
    await press(gui('#panel-body [data-nav="pip"]'));
    if (touch) await closePanelIfOpen();
    await waitFor("the floating page", () => page.locator("#preview-pip iframe").isVisible());
    const pip = "#preview-pip";
    const before = await rect(pip);
    const labels = await page.locator("#preview-pip .pill button").evaluateAll((bs) => bs.map((b) => b.getAttribute("aria-label")));
    check(labels.join("|") === "Open in right panel|Close floating preview", `the pill offers ${labels.join(" | ")}`);
    check(!(await page.locator("#preview-pip .pill .title").count()), "the pill shows a title");
    const drag = async (selector, dx, dy) => {
      const r = await rect(selector);
      await page.mouse.move(r.x + r.width / 2, r.y + r.height / 2);
      await page.mouse.down();
      await page.mouse.move(r.x + r.width / 2 + dx, r.y + r.height / 2 + dy, { steps: 5 });
      await page.mouse.up();
    };
    if (!touch) {
      await drag("#preview-pip .grip.nw", -60, -60);
      const grown = await rect(pip);
      check(grown.width > before.width + 20 && Math.abs(grown.width / grown.height - before.width / before.height) < 0.03, `a corner drag made it ${grown.width}×${grown.height} from ${before.width}×${before.height}`);
      check(Math.abs(grown.right - before.right) <= 1 && Math.abs(grown.bottom - before.bottom) <= 1, "the opposite corner moved");
      await page.hover(pip);
      await drag("#preview-pip .handle", -40, -30);
      const moved = await rect(pip);
      check(Math.abs(moved.x - (grown.x - 40)) <= 1 && Math.abs(moved.y - (grown.y - 30)) <= 1, `the handle moved it to ${moved.x},${moved.y} from ${grown.x},${grown.y}`);
      await drag("#preview-pip .handle", -4000, -4000);
      const chat = await rect("#chat");
      const clamped = await rect(pip);
      check(Math.round(clamped.x - chat.x) === 12 && Math.round(clamped.y - chat.y) === 12, `dragged off, it sits at ${clamped.x - chat.x},${clamped.y - chat.y}`);
    }
    check((await page.locator("#preview-pip .grip").count()) === 8, "the player has no resize grips");
    await press(gui('#preview-pip [data-pip="close"]'));
    await waitFor("the player to close", async () => !(await page.locator(pip).isVisible()));
    await page.evaluate(() => { panelOf(active).pipAt = null; savePanel(active); });
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
      // Beta keeps its own scope, such as the turn an Open diff showed earlier, but not alpha's.
      const betaScope = JSON.parse((await page.evaluate(() => load(`gui-diff:${threadKey(active)}`))) ?? "null");
      check(betaScope?.base !== "origin/feature-base", `beta took alpha's diff scope: ${JSON.stringify(betaScope)}`);
      await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
      await openPanelTo("diff", "Diff");
      await waitFor("alpha's scope back", async () => (await text("#diff-scope")) === "Changes" && (await text("#panel-body .base-pick")) === "origin/feature-base", 10000);
      await press(gui("#diff-scope"));
      await press(gui("#panel-menu button[data-sub]"));
      const turns = await page.locator("#panel-submenu button").allTextContents();
      await press(page.locator("#panel-submenu button").last());
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
      check(jsColor === "rgb(255, 212, 82)", `a .js file's icon is ${jsColor}`);
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

  // The Diff surface's own steps share one Uncommitted diff: a modified, a renamed, a deleted file,
  // and new files in languages only pierre's file-name table knows.
  const DIFF3 = path.join(HOME, "work/alpha/diff3");
  const diff3 = { head: null };
  const diff3File = (name) => `#panel-body .file[data-path="diff3/${name}"]`;
  const DIFF3_LANGS = {
    "page.astro": '---\nconst title = "Hi";\n---\n<h1 class="big">{title}</h1>\n',
    "build.gradle.kts": 'plugins {\n    kotlin("jvm") version "1.9.0"\n}\nval answer: Int = 42\n',
    "flake.nix": '{ pkgs ? import <nixpkgs> {} }:\npkgs.mkShell { buildInputs = [ pkgs.hello ]; }\n',
    "run.ps1": '$name = "world"\nWrite-Host "Hello $name"\n',
    "schema.prisma": 'model User {\n  id    Int    @id @default(autoincrement())\n  email String @unique\n}\n',
  };
  const openDiff3 = async () => {
    await openPanelTo("diff", "Diff");
    await press(gui("#diff-scope"));
    await press(menuItem("Uncommitted"));
    await waitFor("diff3 in the diff", () => page.locator(diff3File("sep.txt")).isVisible(), 10000);
  };
  const unfold = async (name) => {
    if ((await page.locator(diff3File(name)).getAttribute("data-collapsed")) === "") await press(gui(`${diff3File(name)} > header .change`));
    await waitFor(`${name} open`, async () => (await page.locator(diff3File(name)).getAttribute("data-collapsed")) === null);
  };

  await step("app view: the diff colors every language pierre names from its first paint, with no plain flash", async () => {
    fs.rmSync(DIFF3, { recursive: true, force: true });
    fs.mkdirSync(DIFF3, { recursive: true });
    const sep = Array.from({ length: 700 }, (_, i) => `row ${i + 1}`);
    fs.writeFileSync(path.join(DIFF3, "sep.txt"), `${sep.join("\n")}\n`);
    fs.writeFileSync(path.join(DIFF3, "old-name.txt"), "one\ntwo\nthree\nfour\nfive\nsix\n");
    fs.writeFileSync(path.join(DIFF3, "gone.txt"), "bye\n");
    diff3.head = alphaGit("symbolic-ref", "--short", "HEAD");
    alphaGit("add", "diff3");
    alphaGit("commit", "-q", "-m", "diff3");
    sep[9] = "row 10 changed";
    sep[249] = "row 250 changed";
    sep[499] = "row 500 changed";
    fs.writeFileSync(path.join(DIFF3, "sep.txt"), `${sep.join("\n")}\n`);
    fs.renameSync(path.join(DIFF3, "old-name.txt"), path.join(DIFF3, "new-name.txt"));
    fs.rmSync(path.join(DIFF3, "gone.txt"));
    for (const [name, body] of Object.entries(DIFF3_LANGS)) fs.writeFileSync(path.join(DIFF3, name), body);
    await openPanelTo("diff", "Diff");
    await page.evaluate(() => {
      window.diffPlain = new Set();
      const look = () => {
        for (const c of document.querySelectorAll("#panel-body .file .dl .code")) {
          const p = c.closest(".file").dataset.path;
          if (/\.(astro|kts|nix|ps1|prisma)$/.test(p) && c.textContent.trim() && !c.querySelector('span[style*="color"]')) window.diffPlain.add(p);
        }
      };
      window.diffPlainWatch = new MutationObserver(look);
      window.diffPlainWatch.observe(panelBody, { childList: true, subtree: true });
    });
    await openDiff3();
    await press(gui('#panel-body [data-diff="collapse"]'));
    const names = Object.keys(DIFF3_LANGS);
    const colors = {};
    await waitFor("each language's colors", async () => {
      for (const name of names) colors[name] = await page.locator(`${diff3File(name)} .dl .code span[style*="color"]`).evaluateAll((spans) => new Set(spans.map((s) => s.style.color)).size);
      return names.every((n) => colors[n] >= 2);
    }, 15000).catch(() => check(false, `colors per file: ${JSON.stringify(colors)}`));
    const plain = await page.evaluate(() => {
      window.diffPlainWatch.disconnect();
      return [...window.diffPlain];
    });
    check(!plain.length, `drawn in plain text before Shiki colored them: ${plain.join(", ")}`);
    return names.map((n) => `${n} ${colors[n]} colors`).join(", ");
  });

  await step("app view: diff file headers draw pierre's status icon, a rename's old name, and only the counts that apply", async () => {
    await openDiff3();
    const symbol = (name) => page.locator(`${diff3File(name)} > header .change use`).getAttribute("href");
    const symbols = { "sep.txt": await symbol("sep.txt"), "new-name.txt": await symbol("new-name.txt"), "gone.txt": await symbol("gone.txt"), "page.astro": await symbol("page.astro") };
    check(symbols["sep.txt"] === "#diffs-icon-symbol-modified" && symbols["new-name.txt"] === "#diffs-icon-symbol-moved" && symbols["gone.txt"] === "#diffs-icon-symbol-deleted" && symbols["page.astro"] === "#diffs-icon-symbol-added", `icons: ${JSON.stringify(symbols)}`);
    check((await text(`${diff3File("new-name.txt")} > header .from`)) === "diff3/old-name.txt", `the rename's old name reads ${await text(`${diff3File("new-name.txt")} > header .from`)}`);
    check((await page.locator(`${diff3File("new-name.txt")} > header .moved use`).getAttribute("href")) === "#diffs-icon-arrow-right-short", "no arrow between a rename's names");
    check(!(await page.locator(`${diff3File("new-name.txt")} .dl`).count()), "a pure rename shows lines");
    const counts = async (name) => (await page.locator(`${diff3File(name)} > header :is(.add, .del)`).allTextContents()).join(" ");
    const shown = { added: await counts("page.astro"), deleted: await counts("gone.txt"), changed: await counts("sep.txt") };
    check(shown.added === "+4" && shown.deleted === "-1" && shown.changed === "-3 +3", `counts: ${JSON.stringify(shown)}`);
    const tint = await page.locator(`${diff3File("gone.txt")} > header .change`).evaluate((el) => getComputedStyle(el).color);
    check(tint === "rgb(255, 103, 98)", `a deleted file's icon is ${tint}`);
    return JSON.stringify(shown);
  });

  await step("app view: the diff toolbar shows totals and file controls only with files, says what each toggle will do, and Refresh keeps the diff", async () => {
    await openDiff3();
    const title = (action) => page.getAttribute(`#panel-body [data-diff="${action}"]`, "title");
    if ((await page.locator("#panel-body .file:not([data-collapsed])").count()) > 0) await press(gui('#panel-body [data-diff="collapse"]'));
    check((await title("collapse")) === "Expand all files", `with every file folded the button says ${await title("collapse")}`);
    await press(gui('#panel-body [data-diff="collapse"]'));
    check((await title("collapse")) === "Collapse all files", `with files open the button says ${await title("collapse")}`);
    const wrapped = await page.evaluate(() => ui.diffWrap);
    check((await title("wrap")) === (wrapped ? "Disable line wrapping" : "Enable line wrapping"), `wrap reads ${await title("wrap")}`);
    await press(gui('#panel-body [data-diff="wrap"]'));
    check((await title("wrap")) === (wrapped ? "Enable line wrapping" : "Disable line wrapping"), `toggled, wrap reads ${await title("wrap")}`);
    await press(gui('#panel-body [data-diff="wrap"]'));
    const treeOn = await page.evaluate(() => ui.diffTree);
    check((await title("tree")) === (treeOn ? "Hide file tree" : "Show file tree"), `the tree toggle reads ${await title("tree")}`);
    await page.evaluate(() => {
      window.diffRefresh = { spun: false, emptied: false };
      window.diffRefreshWatch = new MutationObserver(() => {
        if (document.querySelector('#panel-body [data-diff="refresh"].refreshing')) window.diffRefresh.spun = true;
        if (document.querySelector("#panel-body .diff-skeleton") || !document.querySelector("#panel-body .file")) window.diffRefresh.emptied = true;
      });
      window.diffRefreshWatch.observe(panelBody, { childList: true, subtree: true, attributes: true });
    });
    await press(gui('#panel-body [data-diff="refresh"]'));
    await waitFor("the refresh to finish", async () => !(await page.locator('#panel-body [data-diff="refresh"].refreshing').count()) && (await page.evaluate(() => window.diffRefresh.spun)), 10000);
    const refresh = await page.evaluate(() => (window.diffRefreshWatch.disconnect(), window.diffRefresh));
    check(refresh.spun && !refresh.emptied, `refresh: ${JSON.stringify(refresh)}`);
    check((await page.getAttribute('#panel-body [data-diff="refresh"]', "title")) === "Refresh diff", "Refresh's title");
    return `wrap was ${wrapped ? "on" : "off"}`;
  });

  await step("app view: the diff's file tree draws pierre's icons, git-status letters and the chosen file", async () => {
    await openDiff3();
    if (!(await page.evaluate(() => ui.diffTree))) await press(gui('#panel-body [data-diff="tree"]'));
    await waitFor("the tree", () => page.locator("#panel-body .file-tree").isVisible());
    check((await page.getAttribute('#panel-body [data-diff="tree"]', "title")) === "Hide file tree", "with the tree open its toggle doesn't say Hide file tree");
    const leaf = (name) => `#panel-body .file-tree [data-tree-file="diff3/${name}"]`;
    const letters = {};
    for (const name of ["sep.txt", "new-name.txt", "gone.txt", "page.astro"]) letters[name] = await text(`${leaf(name)} .git`);
    check(letters["sep.txt"] === "M" && letters["new-name.txt"] === "R" && letters["gone.txt"] === "D" && letters["page.astro"] === "A", `letters: ${JSON.stringify(letters)}`);
    // The tree redraws as the diff lands, so the row is found and read in one task, never across a redraw.
    const added = await page.evaluate((sel) => getComputedStyle(document.querySelector(sel)).color, `${leaf("page.astro")} .name`);
    check(added === "rgb(0, 202, 177)", `an added file's name is ${added}`);
    check(/^#file-tree-builtin-/.test((await page.locator(`${leaf("page.astro")} .ficon use`).getAttribute("href")) ?? ""), "the file's icon isn't pierre's");
    check(await page.locator('#panel-body .file-tree [data-tree-dir="diff3"] .git svg').isVisible(), "the folder has no dot for its changes");
    check(!(await page.locator('#panel-body .file-tree [data-tree-dir="diff3"] .i:not(.chevron)').count()), "a folder row draws a folder icon");
    await press(gui(leaf("gone.txt")));
    await waitFor("the chosen file lit", async () => (await page.locator(`${leaf("gone.txt")}.picked`).count()) === 1);
    const lit = await page.locator(`${leaf("gone.txt")}`).evaluate((el) => getComputedStyle(el).backgroundColor);
    check(lit !== "rgba(0, 0, 0, 0)", "the chosen file isn't highlighted");
    await press(gui('#panel-body [data-diff="tree"]'));
    return Object.values(letters).join("");
  });

  await step("app view: diff separators use pierre's chevrons, open 100 lines on each side from the label, and Shift opens a whole stretch", async () => {
    await openDiff3();
    await unfold("sep.txt");
    const gap = (i) => `${diff3File("sep.txt")} .gap[data-gap="${i}"]`;
    const buttons = async (i) => page.locator(`${gap(i)} .expand`).evaluateAll((els) => els.map((b) => `${b.dataset.expand}:${b.querySelector("use")?.getAttribute("href")}:${getComputedStyle(b.querySelector("svg")).transform}`));
    const first = await buttons(0);
    check(first.length === 1 && first[0] === "bottom:#diffs-icon-expand:matrix(1, 0, 0, -1, 0, 0)", `the stretch above the first hunk: ${first.join(" | ")}`);
    const middle = await buttons(1);
    check(middle.length === 2 && middle[0] === "top:#diffs-icon-expand:none" && middle[1].startsWith("bottom:#diffs-icon-expand:matrix"), `the stretch between hunks: ${middle.join(" | ")}`);
    check(await page.locator(`${gap(1)}.multi`).count() === 1, "the two chevrons aren't stacked");
    const label = () => text(`${gap(1)} .label`);
    check((await label()) === "233 unmodified lines", `the middle reads ${await label()}`);
    await page.locator(`${gap(1)} .label`).evaluate((el) => el.scrollIntoView({ block: "center" }));
    await press(gui(`${gap(1)} .label`));
    await waitFor("100 lines on each side", async () => (await label()) === "33 unmodified lines", 10000);
    check((await page.locator(`${gap(1)} .expand`).count()) === 1, "33 lines left don't get a single chevron");
    const topChevron = page.locator(`${gap(2)} [data-expand="top"]`);
    check((await text(`${gap(2)} .label`)) === "243 unmodified lines", `the second stretch reads ${await text(`${gap(2)} .label`)}`);
    await topChevron.evaluate((el) => el.scrollIntoView({ block: "center" }));
    if (touch) await topChevron.tap({ modifiers: ["Shift"] });
    else await topChevron.click({ modifiers: ["Shift"] });
    await waitFor("Shift to open the whole stretch", async () => !(await page.locator(gap(2)).count()));
    await page.locator(`${gap(0)} .expand`).evaluate((el) => el.scrollIntoView({ block: "center" }));
    await press(gui(`${gap(0)} .expand`));
    await waitFor("the first six lines", async () => !(await page.locator(gap(0)).count()));
    return `${first.length} + ${middle.length} chevrons`;
  });

  await step("app view: dragging over diff line numbers selects them, and a comment reads as T3's", async () => {
    await openDiff3();
    await unfold("sep.txt");
    const no = (n) => `${diff3File("sep.txt")} .no[data-side="new"][data-line="${n}"]`;
    const selected = () => page.locator(`${diff3File("sep.txt")} .dl.sel`).count();
    await press(gui(no(7)));
    check((await selected()) === 1, `${await selected()} rows after pressing line 7`);
    await press(gui(no(7)));
    check((await selected()) === 0, "pressing the one selected line again kept it");
    let dragged = "tapped";
    if (!touch) {
      const at = async (n) => {
        const r = await page.locator(no(n)).boundingBox();
        return [r.x + r.width / 2, r.y + r.height / 2];
      };
      await page.mouse.move(...(await at(7)));
      await page.mouse.down();
      await page.mouse.move(...(await at(8)), { steps: 3 });
      await page.mouse.move(...(await at(9)), { steps: 3 });
      await page.mouse.up();
      check((await selected()) === 3, `${await selected()} rows after dragging from 7 to 9`);
      await press(gui(no(12)));
      await press(gui(no(12)));
      await page.hover(no(11));
      const [x, y] = await (async () => {
        const r = await page.locator(`${no(11)} [data-comment-line]`).boundingBox();
        return [r.x + r.width / 2, r.y + r.height / 2];
      })();
      await page.mouse.move(x, y);
      await page.mouse.down();
      await page.mouse.move(...(await at(13)), { steps: 4 });
      await page.mouse.up();
      dragged = "dragged";
    } else {
      await page.hover(no(11));
      await press(gui(`${no(11)} [data-comment-line]`));
    }
    await waitFor("the draft", () => page.locator("#panel-body .dcomment.draft textarea").isVisible());
    const range = await page.getAttribute("#panel-body .dcomment.draft textarea", "aria-label");
    check(range === (touch ? "Comment on lines L11" : "Comment on lines L11 to L13"), `the draft covers ${range}`);
    const box = await page.locator("#panel-body .dcomment.draft").evaluate((el) => [getComputedStyle(el).backgroundColor, getComputedStyle(el).borderInlineStartWidth]);
    check(box[0] === "rgba(0, 0, 0, 0)" && box[1] === "0px", `the draft sits in a box: ${box.join(", ")}`);
    check((await text("#panel-body .dcomment.draft .hint")) === "⌘/Ctrl Enter to send", `hint: ${await text("#panel-body .dcomment.draft .hint")}`);
    check((await page.locator("#panel-body .dcomment.draft [data-comment-cancel].ghost").count()) === 1, "Cancel isn't a ghost button");
    check(await page.locator("#panel-body [data-comment-save]").isDisabled(), "Comment is enabled with nothing written");
    await page.fill("#panel-body [data-comment-text]", "tighten this");
    check(!(await page.locator("#panel-body [data-comment-save]").isDisabled()), "Comment stays disabled once written");
    await press(gui("#panel-body [data-comment-save]"));
    await waitFor("the saved comment", () => page.locator("#panel-body .dcomment:not(.draft)").isVisible());
    check(!(await page.locator("#panel-body .dcomment:not(.draft) .range").count()) && (await text("#panel-body .dcomment:not(.draft) p")) === "tighten this", `the comment reads ${await text("#panel-body .dcomment:not(.draft) p")}`);
    const tone = await page.locator("#panel-body .dcomment:not(.draft) > .i").evaluate((el) => getComputedStyle(el).color);
    check(tone === "rgba(52, 107, 241, 0.7)", `the comment's icon is ${tone}`);
    await press(gui("#panel-body [data-comment-delete]"));
    await waitFor("the comment to go", async () => !(await page.locator("#panel-body .dcomment").count()));
    return `${dragged}, ${range}`;
  });

  await step("app view: the diff's scope menu lists the newest turn first and a fresh thread starts on Changes", async () => {
    await closePanelIfOpen();
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/beta"))));
    await waitFor("beta", async () => /beta/.test(await text("#chat-head .crumb")));
    await openPanelTo("diff", "Diff");
    await waitFor("beta's diff scope", async () => (await text("#diff-scope")) !== "", 10000);
    const fresh = await text("#diff-scope");
    check(fresh === "Changes", `a thread that never opened Diff starts on ${fresh}`);
    await waitFor("beta's empty Changes", async () => /no default branch/.test(await text("#panel-body")), 10000);
    const left = await page.locator('#panel-body .diff-bar :is(.add, .del, [data-diff="collapse"], [data-diff="tree"])').count();
    check(!left, `with no files the toolbar still shows ${left} totals or file controls`);
    await closePanelIfOpen();
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
    await openPanelTo("diff", "Diff");
    await press(gui("#diff-scope"));
    await press(gui("#panel-menu button[data-sub]"));
    const turns = (await page.locator("#panel-submenu button .label").allTextContents()).map((t) => Number(t.replace("Turn ", "")));
    check(turns.length >= 2 && turns.every((n, i) => i === 0 || n === turns[i - 1] - 1) && turns.at(-1) === 1, `submenu order: ${turns.join(", ")}`);
    await press(gui("#panel-submenu button >> nth=0"));
    await waitFor("the newest turn", async () => (await text("#diff-scope")) === "Latest turn");
    await press(gui("#diff-scope"));
    await press(gui("#panel-menu button[data-sub]"));
    await press(page.locator("#panel-submenu button").last());
    await waitFor("the first turn", async () => (await text("#diff-scope")) === "Turn 1");
    await press(gui("#diff-scope"));
    await press(menuItem("Uncommitted"));
    return turns.join(", ");
  });

  await step("app view: a diff file's name opens a menu to open it, reveal it in Finder, or open it with an editor", async () => {
    await openDiff3();
    const log = path.join(HOME, "open.log");
    const kept = readLog("open.log");
    try {
      return await diffFileMenu();
    } finally {
      if (await page.locator("#side-menu").isVisible()) await page.keyboard.press("Escape");
      if (kept) fs.writeFileSync(log, kept);
      else fs.rmSync(log, { force: true });
    }
  });
  async function diffFileMenu() {
    const lines = () => readLog("open.log").trim().split("\n");
    const pick = async (...labels) => {
      await page.click(`${diff3File("sep.txt")} > header .path`, { button: "right" });
      await waitFor("the file menu", () => page.locator("#side-menu").isVisible());
      for (const label of labels) await press(gui(`#side-menu button:has(.label:text-is("${label}"))`));
    };
    await page.click(`${diff3File("sep.txt")} > header .path`, { button: "right" });
    await waitFor("the file menu", () => page.locator("#side-menu").isVisible());
    const items = await page.locator("#side-menu button .label").allTextContents();
    check(items.join("|") === "Open|Reveal in Finder|Open with", `menu: ${items.join(" | ")}`);
    await page.keyboard.press("Escape");
    const before = lines().length;
    await pick("Reveal in Finder");
    await waitFor("Finder to reveal the file", async () => /^-R \/\S*\/work\/alpha\/diff3\/sep\.txt$/.test(lines().at(-1)));
    await pick("Open");
    await waitFor("the file to open in its app", async () => /^\/\S*\/work\/alpha\/diff3\/sep\.txt$/.test(lines().at(-1)));
    await pick("Open with", "Zed");
    await waitFor("the file to open in Zed", async () => /^-a \S*\/Zed\.app \S*\/work\/alpha\/diff3\/sep\.txt$/.test(lines().at(-1)));
    return `${lines().length - before} opens`;
  }

  await step("app view: Settings' Motion row sets panel animations on a 25 ms slider with a readout, preview and reset", async () => {
    await closePanelIfOpen();
    await page.evaluate(() => toggleSettings());
    try {
      await motionRow();
    } finally {
      await page.evaluate(() => save("gui-panel-animation", null));
      await page.evaluate(() => toggleSettings());
    }
  });
  // The row sits on Appearance, as in T3's settings.
  async function motionRow() {
    await settingsNav("appearance");
    const row = "#settings-page #panel-animations";
    await waitFor("the Motion row", () => page.locator(row).isVisible());
    const slider = await page.locator("#panel-animation-duration").evaluate((el) => [el.type, el.min, el.max, el.step].join(" "));
    check(slider === "range 0 400 25", `the control is ${slider}`);
    check((await text(`${row} h4`)) === "Panel animations" && (await page.locator(row).evaluate((el) => el.closest(".settings-section")?.querySelector("h3")?.textContent)) === "Motion", "the row isn't Motion › Panel animations");
    check((await text(`${row} output`)) === "0 ms" && (await page.locator('[data-reset="motion"]').isHidden()), "at the default the readout isn't 0 ms with no reset");
    await page.locator("#panel-animation-duration").fill("225");
    check((await text(`${row} output`)) === "225 ms" && (await page.evaluate(() => load("gui-panel-animation"))) === "225", `set to 225 the readout is ${await text(`${row} output`)}`);
    const duration = await page.locator(`${row} .motion-preview .side`).evaluate((el) => getComputedStyle(el).transitionDuration);
    check(duration.split(", ").every((d) => d === "0.225s"), `the preview takes ${duration}`);
    await press(gui(`${row} .motion-preview`));
    check(await page.locator(`${row} .motion-preview.closed`).count(), "the preview didn't fold");
    await press(gui(`${row} .motion-preview`));
    check(!(await page.locator(`${row} .motion-preview.closed`).count()), "the preview didn't unfold");
    await press(gui('[data-reset="motion"]'));
    check((await text(`${row} output`)) === "0 ms" && (await page.evaluate(() => load("gui-panel-animation"))) === null, "reset didn't return to 0 ms");
  }

  await step("app view: the diff steps clean up", async () => {
    await closePanelIfOpen();
    if (diff3.head) alphaGit("update-ref", "-d", `refs/heads/${diff3.head}`);
    alphaGit("rm", "-r", "-q", "-f", "--cached", "--ignore-unmatch", "diff3");
    fs.rmSync(DIFF3, { recursive: true, force: true });
    const d = await page.evaluate(() => (setDiffState(active, { scope: "turn", turn: null }), diffState(active).scope));
    check(d === "turn", "alpha's diff scope didn't go back to its turn");
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
    await page.evaluate(() => save("gui-browser-links", "app"));
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
    await page.evaluate(() => save("gui-browser-links", ""));
    await closePanelIfOpen();
    fs.rmSync(path.join(HOME, "open.log"), { force: true });
  });

  await step("app view: Linked pull requests lists the thread's pull requests and links another", async () => {
    await openPanelTo("prs", "Linked pull requests");
    await waitFor("the thread's two", async () => (await page.locator("#panel-body .pr-row").count()) === 2 && /Draft the notes/.test(await text("#panel-body .prs-list")), 15000);
    check(/1 open · 2 linked/.test(await text("#panel-body .prs footer")), `footer: ${await text("#panel-body .prs footer")}`);
    await press(gui("#panel-body [data-prs-link]"));
    await page.fill("#link-pr input", "https://github.com/example/alpha/pull/43");
    await page.press("#link-pr input", "Enter");
    await waitFor("#43 linked", async () => (await page.locator("#panel-body .pr-row").count()) === 3, 10000);
    if (!touch) await page.hover('#panel-body .pr-row[data-prs-open$="/pull/43"]');
    await press(gui('#panel-body [data-prs-more$="/pull/43"]'));
    await press(menuItem("Unlink from thread"));
    await waitFor("#43 unlinked", async () => (await page.locator("#panel-body .pr-row").count()) === 2);
    await press(gui('#panel-body .pr-row[data-prs-open$="/pull/41"] .what'));
    await waitFor("#41 opened", async () => /#41/.test(await text('#panel-tabs .ptab.active[data-panel="pr"]')));
    await closePanelIfOpen();
  });

  await step("app view: the Device tab sets up the device hub as T3's wizard does, lists the machine's simulators and emulators, and opens one to its live screen", async () => {
    await openPanelTo("device", "Device");
    await waitFor("the setup wizard", async () => (await text("#device-setup h2")) === "Set up devices", 10000);
    check(await page.isDisabled("#device-setup [data-setup-next]"), "setup continues with the hub off");
    await press(gui('#device-setup [aria-label="Enable device hub"]'));
    await waitFor("the hub ready", async () => /Device hub is ready\./.test(await text("#device-setup .setup-body")), 30000);
    check(/expo-device-hub@0\.15\.3 --host 127\.0\.0\.1 --port \d+/.test(readLog("npx.log")), `npx ran: ${readLog("npx.log")}`);
    await press(gui("#device-setup [data-setup-next]"));
    check((await text('#device-setup [data-platform="ios"]')).includes("Xcode and iOS Simulator are available."), "iOS isn't ready");
    check((await text('#device-setup [data-platform="android"]')).includes("The Android SDK and Emulator are available."), "Android isn't ready");
    await press(gui("#device-setup [data-setup-next]"));
    check(/Allow agent control/.test(await text("#device-setup .setup-body")), "the third step isn't agent access");
    await press(gui('#device-setup [aria-label="Allow agents to control devices"]'));
    await waitFor("agent tools", async () => /Agent tools are ready\./.test(await text("#device-setup .setup-body")));
    check(await page.evaluate(() => callOn(active.machine, { cmd: "device_agents" })), "the machine doesn't keep the agents' access");
    await press(gui('#device-setup [aria-label="Allow agents to control devices"]'));
    await waitFor("agent access off", async () => !/Agent tools are ready/.test(await text("#device-setup .setup-body")));
    await press(gui("#device-setup [data-setup-next]"));
    await waitFor("the device list", async () => (await page.locator("#panel-body .device-list h3").allTextContents()).join("|") === "iOS Simulators|Android Emulators", 15000);
    const rows = (await page.locator("#panel-body .device-row").allInnerTexts()).map((r) => r.replace(/\s+/g, " ").trim());
    check(rows.join("|") === "iPhone 17 sweep-node · iOS 26.0 · Stopped Start|Pixel_9 sweep-node · Android 16 · Running Open|Tablet_10 sweep-node · Android · Stopped Start", `the rows: ${rows.join(" | ")}`);
    await press(gui('#panel-body [aria-label="Open Pixel_9"]'));
    await waitFor("Pixel_9's tab", async () => (await text('#panel-tabs .ptab.active[data-panel="device"]')) === "Pixel_9", 15000);
    check(await page.locator('#panel-tabs .ptab.active[data-panel="device"] svg.brand').count(), "the tab has no Android mark");
    check(/Pixel_9\nsweep-node · Android 16/.test(await page.getAttribute('#panel-tabs .ptab.active[data-panel="device"]', "title")), "the tab's tooltip lacks the machine and version");
    await waitFor("the H.264 stream", async () => (await page.getAttribute("#panel-body .device-screen", "data-live").catch(() => null)) === "video", 15000);
    const pixel = await page.evaluate(() => [...document.querySelector("#panel-body .device-screen canvas").getContext("2d").getImageData(108, 240, 1, 1).data]);
    // Teal, within the engines' YUV conversions: headless Chromium draws the frame at 37,157,138 and WebKit at 59,165,150.
    check(Math.abs(pixel[0] - 42) < 24 && Math.abs(pixel[1] - 157) < 24 && Math.abs(pixel[2] - 143) < 24, `the decoded frame's center is ${pixel}`);
    check(/stream emulator-5554 \?device=emulator-5554&frame-meta=1/.test(readLog("hub.log")), "the screen didn't open serve-emu's socket");
    check((await page.locator("#panel-body .device-rail button").evaluateAll((bs) => bs.map((b) => b.getAttribute("aria-label")))).join("|") === "Home|Back|Recents|Rotate device|Switch device to dark mode|Device text size|Device tools|Save screenshot|More device actions", "the rail isn't T3's");
    await press(gui("#panel-body .device-screen"));
    await waitFor("a touch on the socket", async () => /ws emulator-5554 \{"type":"touch","action":"down","x":0\.\d+,"y":0\.\d+,"ack":false\}[\s\S]*"action":"up"/.test(readLog("hub.log")));
    if (!touch) {
      await page.focus("#panel-body .device-screen");
      await page.keyboard.type("hi");
      await page.keyboard.press("Enter");
      await waitFor("the keys", async () => /"type":"text","text":"h"[\s\S]*"type":"text","text":"i"[\s\S]*"type":"key","keycode":66/.test(readLog("hub.log")));
    }
    for (const button of ["Home", "Back", "Recents"]) await press(gui(`#panel-body .device-rail [aria-label="${button}"]`));
    await waitFor("the buttons", async () => /\{"type":"home","ack":false\}[\s\S]*\{"type":"back","ack":false\}[\s\S]*\{"type":"recents","ack":false\}/.test(readLog("hub.log")));
    await press(gui('#panel-body .device-rail [aria-label="Rotate device"]'));
    check((await menuLabels()).join("|") === "Portrait|Landscape", "the rotate menu");
    await press(menuItem("Landscape"));
    await waitFor("adb to tilt the emulator", async () => readLog("adb.log").includes("adb -s emulator-5554 emu sensor set acceleration 9.81:0:0"));
    await press(gui('#panel-body .device-rail [aria-label="Switch device to dark mode"]'));
    await waitFor("dark mode", async () => readLog("adb.log").includes("adb -s emulator-5554 shell cmd uimode night yes") && (await page.locator('#panel-body .device-rail [aria-label="Switch device to light mode"]').count()) === 1);
    await press(gui('#panel-body .device-rail [aria-label="Device text size"]'));
    check((await menuLabels()).join("|") === "Small|Default|Large|Extra large", "the text sizes");
    await press(menuItem("Large"));
    await waitFor("a larger font", async () => readLog("adb.log").includes("shell settings put system font_scale 1.15"));
    await press(gui('#panel-body .device-rail [aria-label="Device tools"]'));
    await waitFor("the tools", async () => /Foreground\s*com\.android\.launcher3/.test(await text("#panel-body .device-tools")), 10000);
    check((await page.locator("#panel-body .device-tools h4").allTextContents()).join("|") === "App|Emulator|Accessibility|Location|Permissions", "the tools' sections");
    await page.fill('#panel-body .device-tools [name="app"][placeholder="Package name to launch"]', "com.example.app");
    await press(gui('#panel-body .device-tools [data-tool-form="launch"] button'));
    await waitFor("the app in front", async () => /Foreground\s*com\.example\.app/.test(await text("#panel-body .device-tools")), 10000);
    check(readLog("adb.log").includes("shell monkey -p com.example.app -c android.intent.category.LAUNCHER 1"), "adb launched nothing");
    await press(gui('#panel-body .device-tools [aria-label="Network"]'));
    await waitFor("the network off", async () => readLog("adb.log").includes("shell svc wifi disable") && readLog("adb.log").includes("shell svc data disable"));
    await page.selectOption('#panel-body .device-tools [data-tool-select="preset"]', "Tokyo");
    await press(gui('#panel-body .device-tools [data-tool-form="location"] button[type="submit"]'));
    await waitFor("Tokyo", async () => readLog("adb.log").includes("emu geo fix 139.6503 35.6762"));
    await press(gui('#panel-body .device-tools [data-decision="grant"]'));
    await waitFor("the camera granted", async () => readLog("adb.log").includes("shell pm grant com.example.app android.permission.CAMERA"));
    await press(gui('#panel-body .device-tools [aria-label="Overlay element frames"]'));
    await waitFor("the frames over the screen", async () => (await text("#panel-body .device-screen .ax .chip")) === "Sign in", 10000);
    // A phone's tools cover the screen, so it is measured once they close; the overlay outlives them.
    await press(gui('#panel-body .device-tools [aria-label="Close tools"]'));
    const frame = await page.evaluate(() => {
      const [screen, box] = [document.querySelector("#panel-body .device-screen").getBoundingClientRect(), document.querySelector("#panel-body .device-screen .ax").getBoundingClientRect()];
      return [(box.left - screen.left) / screen.width, (box.top - screen.top) / screen.height].map((n) => Math.round(n * 100));
    });
    check(frame.join(",") === "25,75", `the Sign in frame sits at ${frame}`);
    const [download] = await Promise.all([page.waitForEvent("download"), press(gui('#panel-body .device-rail [aria-label="Save screenshot"]'))]);
    check(/^Pixel_9-.+\.png$/.test(download.suggestedFilename()), `the screenshot is ${download.suggestedFilename()}`);
    // Away from the hub's machine the screen is the machine's screenshots, and taps go through it.
    await page.evaluate(() => fallBackToScreenshots([...deviceScreens.values()][0]));
    await waitFor("screenshots", async () => (await page.getAttribute("#panel-body .device-screen", "data-live")) === "screenshots" && /^data:image\/png;base64,/.test(await page.getAttribute("#panel-body .device-screen img", "src")), 10000);
    await press(gui("#panel-body .device-screen"));
    await waitFor("a tap through the machine", async () => /rest tap emulator-5554 \{"type":"tap","x":0\.\d+,"y":0\.\d+\}/.test(readLog("hub.log")), 10000);
    await press(gui('#panel-body .device-rail [aria-label="More device actions"]'));
    check((await menuLabels()).join("|") === "Float device over chat|Close device panel|Power off device", "the more menu");
    await press(menuItem("Float device over chat"));
    await waitFor("the device floating", async () => (await page.locator("#preview-pip .device-screen").count()) === 1 && (await page.evaluate(() => document.body.dataset.panel)) === "closed");
    await press(gui('#preview-pip [data-pip="panel"]'));
    await waitFor("the device back", async () => (await page.locator("#panel-body .device-stage .device-screen").count()) === 1);
    await press(gui('#panel-body .device-rail [aria-label="More device actions"]'));
    await press(menuItem("Power off device"));
    await waitFor("Pixel_9 off and its tab closed", async () => readLog("hub.log").includes("shutdown emulator-5554") && !(await page.locator('#panel-tabs .ptab[data-panel="device"]').count()), 10000);
    await openPanelTo("device", "Device");
    await press(gui('#panel-body [aria-label="Start iPhone 17"]'));
    await waitFor("the iPhone's screen", async () => (await page.getAttribute("#panel-body .device-screen", "data-live").catch(() => null)) === "mjpeg", 15000);
    check(/boot ios 5A1C7E0B-2D7A-4E1B-9C3D-6F8A9B0C1D2E iPhone 17\nattach 5A1C7E0B-2D7A-4E1B-9C3D-6F8A9B0C1D2E/.test(readLog("hub.log")), "the simulator didn't boot and attach");
    check(await page.locator('#panel-tabs .ptab.active[data-panel="device"] svg.brand').count(), "the tab has no Apple mark");
    check(!(await page.locator('#panel-body .device-rail [aria-label="Back"]').count()), "an iPhone has a Back button");
    await press(gui('#panel-body .device-rail [aria-label="Home"]'));
    await press(gui("#panel-body .device-screen"));
    await waitFor("the simulator's HID input", async () => /hid 5A1C7E0B-2D7A-4E1B-9C3D-6F8A9B0C1D2E 4 \{"button":"home"\}[\s\S]*hid \S+ 3 \{"type":"begin"[\s\S]*hid \S+ 3 \{"type":"end"/.test(readLog("hub.log")), 10000);
    if (!touch) {
      await page.dblclick('#panel-tabs .ptab[data-panel="device"] span:last-child');
      await page.fill("#panel-tabs .ptab .rename", "Sim");
      await page.press("#panel-tabs .ptab .rename", "Enter");
      await waitFor("the renamed tab", async () => (await text('#panel-tabs .ptab[data-panel="device"]')) === "Sim");
    }
    await press(gui('#panel-tabs .ptab[data-panel="device"] .lead'));
    await closePanelIfOpen();
  });

  const dropMention = (from) =>
    page.evaluate((selector) => {
      const data = new DataTransfer();
      document.querySelector(selector).dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: data }));
      const composer = document.getElementById("composer");
      composer.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data }));
      composer.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }));
    }, from);

  await step("app view: Files draws T3's Pierre rows, shares a lone folder's row, and its right-click menu opens, reveals, mentions and adds a file to the chat, as a drag does", async () => {
    fs.mkdirSync(path.join(HOME, "work/alpha/pkg/src"), { recursive: true });
    fs.writeFileSync(path.join(HOME, "work/alpha/pkg/src/index.ts"), "export {};\n");
    await page.fill("#prompt", "");
    await openPanelTo("files", "Files");
    await page.fill("#panel-body [data-files-filter]", "");
    await press(gui("#panel-body [data-files-refresh]"));
    await waitFor("pkg/src on one row", async () => (await text('#panel-body [data-files-dir="pkg/src"]')) === "pkg/src", 10000);
    check(await page.locator('#panel-body [data-files-open="app.js"] svg.pierre use[href="#file-tree-builtin-javascript"]').count(), "app.js has no Pierre JavaScript icon");
    check(!(await page.locator("#panel-body .files-tree .dir .i:not(.chevron)").count()), "folders draw a folder icon T3's tree has not");
    await page.locator('#panel-body [data-files-open="app.js"]').click({ button: "right" });
    const items = await menuLabels();
    check(items.join("|") === "Open|Reveal in Finder|Open with|Copy mention|Add to chat", `the file menu: ${items.join(" | ")}`);
    await press(menuItem("Open"));
    await waitFor("the system to open app.js", async () => readLog("open.log").trim().split("\n").pop() === path.join(HOME, "work/alpha/app.js"));
    await page.locator('#panel-body [data-files-open="app.js"]').click({ button: "right" });
    await press(menuItem("Reveal in Finder"));
    await waitFor("Finder to reveal app.js", async () => readLog("open.log").trim().split("\n").pop() === `-R ${path.join(HOME, "work/alpha/app.js")}`);
    await page.locator('#panel-body [data-files-dir="pkg/src"]').click({ button: "right" });
    await press(menuItem("Copy mention"));
    await waitFor("the copy note", async () => (engine === "chromium" ? /Mention copied/ : /Mention copied|Couldn't reach the clipboard/).test(await text("#panel-note")));
    if (engine === "chromium") check((await page.evaluate(() => navigator.clipboard.readText())) === "@pkg/src", "the clipboard lacks the mention");
    await page.locator('#panel-body [data-files-open="app.js"]').click({ button: "right" });
    await press(menuItem("Add to chat"));
    await waitFor("app.js in the composer", async () => (await page.inputValue("#prompt")) === "@app.js ");
    await dropMention('#panel-body [data-files-open="notes.md"]');
    await waitFor("notes.md dropped in", async () => (await page.inputValue("#prompt")) === "@app.js @notes.md ");
    check(await page.evaluate(() => chipsOf(active)["@notes.md"]?.value === "notes.md"), "the dropped file is no mention chip");
    await page.fill("#prompt", "");
    await closePanelIfOpen();
  });

  await step("app view: a File tab carries the file's icon, draws HTML from the folder with its stylesheet, opens it in the preview browser and in T3's Open-in group, and shows a big file's first 1 MB read-only", async () => {
    fs.mkdirSync(path.join(HOME, "work/alpha/site"), { recursive: true });
    fs.writeFileSync(path.join(HOME, "work/alpha/site/index.html"), `<!doctype html><link rel="stylesheet" href="style.css"><h1>The site</h1>`);
    fs.writeFileSync(path.join(HOME, "work/alpha/site/style.css"), "h1 { color: rgb(1, 2, 3); }");
    fs.writeFileSync(path.join(HOME, "work/alpha/big.log"), "x".repeat(1258291));
    await openPanelTo("files", "Files");
    await press(gui("#panel-body [data-files-refresh]"));
    await page.fill("#panel-body [data-files-filter]", "index.html");
    await press(gui('#panel-body [data-files-open="site/index.html"]'));
    await waitFor("index.html in a File tab", async () => (await text('#panel-tabs .ptab.active[data-panel="file"]')) === "index.html");
    check(await page.locator('#panel-tabs .ptab.active .lead svg.pierre use[href="#file-tree-builtin-html"]').count(), "the File tab has no Pierre HTML icon");
    const frame = page.frameLocator("#panel-body iframe.html-view");
    await waitFor("the page with its stylesheet", async () => (await frame.locator("h1").evaluate((h) => getComputedStyle(h).color).catch(() => "")) === "rgb(1, 2, 3)", 10000);
    check(/^http:\/\/127\.0\.0\.1:\d+\/[0-9a-f]{32}\/site\/index\.html$/.test(await page.getAttribute("#panel-body iframe.html-view", "src")), "the page isn't served from the folder");
    await press(gui("#panel-body .open-in-file .main"));
    await waitFor("the preferred editor to open the file", async () => /^-a \S+\.app /.test(readLog("open.log").trim().split("\n").pop()) && readLog("open.log").trim().endsWith(` ${path.join(HOME, "work/alpha/site/index.html")}`));
    await press(gui("#panel-body .open-in-file .pick"));
    const editors = await menuLabels();
    check(editors.includes("Zed") && editors.includes("Finder"), `the editors: ${editors.join(" | ")}`);
    await page.keyboard.press("Escape");
    await press(gui('#panel-body [data-file-browser]'));
    await waitFor("the file in the browser", async () => /The site/.test((await page.frameLocator("#panel-body .browser iframe").locator("body").textContent().catch(() => "")) ?? ""), 10000);
    await press(gui('#panel-tabs .ptab.active[data-panel="browser"] .lead'));
    await page.fill("#panel-body [data-files-filter]", "").catch(() => {});
    await page.evaluate(() => openFileSurface("big.log"));
    await waitFor("the 1 MB notice", async () => (await text("#panel-body .file-notice")) === "Preview limited to the first 1 MB of a 1,258,291 byte file.", 10000);
    check(await page.locator("#panel-body .code-edit[readonly]").count(), "a cut-off file can be edited");
    await page.evaluate(() => {
      machines.get(active.machine).view.online = false;
      renderPanel();
    });
    await waitFor("the read-only notice", async () => /Read-only connection\. Unsaved edits are kept until write access returns\./.test(await text("#panel-body .file-view")));
    await page.evaluate(() => {
      machines.get(active.machine).view.online = true;
      renderPanel();
    });
    await press(gui('#panel-tabs .ptab.active[data-panel="file"] .lead'));
    await press(gui('#panel-tabs .ptab.active[data-panel="file"] .lead'));
    await closePanelIfOpen();
    for (const made of ["site", "big.log", "pkg"]) fs.rmSync(path.join(HOME, "work/alpha", made), { recursive: true, force: true });
  });

  await step("app view: the first Terminal tab reads Terminal 1, splits side by side and stacked up to four, lists and closes them, opens another, and its menu adds a selection to the chat", async () => {
    await openPanelTo("terminal", "Terminal");
    await waitFor("Terminal 1", async () => (await text('#panel-tabs .ptab.active[data-panel="terminal"]')) === "Terminal 1");
    await press(gui('#panel-body [data-term-act="split"]'));
    await waitFor("two side by side", async () => (await page.locator("#panel-body .term-pane .xterm").count()) === 2);
    check((await text('#panel-tabs .ptab.active[data-panel="terminal"]')) === "Terminal 2", "the tab doesn't name the new terminal");
    check((await page.locator("#panel-body .term-list .term-row .name").allTextContents()).join("|") === "Terminal 1|Terminal 2", "the list misses a terminal");
    check(/Side by side\s*2/.test(await text("#panel-body .term-group")), `the group reads ${await text("#panel-body .term-group")}`);
    await press(gui('#panel-body .term-list [data-term-act="split-vertical"]'));
    await waitFor("three stacked", async () => (await page.locator("#panel-body .term-pane .xterm").count()) === 3 && (await page.getAttribute("#panel-body .term-panes", "data-split")) === "vertical");
    const [a, b] = await page.locator("#panel-body .term-pane").evaluateAll((els) => els.slice(0, 2).map((el) => el.getBoundingClientRect().toJSON()));
    check(Math.abs(a.left - b.left) < 2 && b.top > a.top + 20, "the panes don't stack");
    await press(gui('#panel-body .term-list [data-term-act="split"]'));
    await waitFor("four", async () => (await page.locator("#panel-body .term-pane").count()) === 4);
    check((await page.getAttribute('#panel-body .term-list [data-term-act="split"]', "aria-disabled")) === "true", "a fifth split is offered");
    check(/max 4 per group/.test(await page.getAttribute('#panel-body .term-list [data-term-act="split"]', "aria-label")), "the split's label lacks its limit");
    await press(gui('#panel-body .term-list [data-term-act="close"]'));
    await waitFor("three again", async () => (await page.locator("#panel-body .term-pane").count()) === 3);
    await press(gui('#panel-body .term-row [data-term-close="term-3"]'));
    await waitFor("Terminal 3 closed", async () => (await page.locator("#panel-body .term-list .term-row .name").allTextContents()).join("|") === "Terminal 1|Terminal 2");
    await press(gui('#panel-body .term-row[data-term-pick="term-1"] .name'));
    await waitFor("Terminal 1 picked", async () => (await text('#panel-tabs .ptab.active[data-panel="terminal"]')) === "Terminal 1");
    await page.locator('#panel-body .term-pane[data-term="term-1"] .xterm-helper-textarea').focus();
    await page.keyboard.type("echo split-$((20+22))\n");
    const pane = '#panel-body .term-pane[data-term="term-1"]';
    await waitFor("the output", async () => /split-42/.test(await text(`${pane} .xterm-rows`)), 10000);
    await page.locator(`${pane} .xterm-screen`).click({ button: "right" });
    check((await menuLabels()).join("|") === "Add to chat|Copy|Paste|Select all|Jump to latest", `the terminal menu: ${(await menuLabels()).join(" | ")}`);
    check(await page.isDisabled('#panel-menu button:has(.label:text-is("Add to chat"))'), "Add to chat works with nothing selected");
    await press(menuItem("Select all"));
    await page.locator(`${pane} .xterm-screen`).click({ button: "right" });
    await press(menuItem("Add to chat"));
    await waitFor("the selection in the composer", async () => /Terminal 1 lines? 1/.test(await text("#attachments")));
    check(await page.evaluate(() => /^<terminal_context>\n- Terminal 1 lines? 1[\s\S]*split-42[\s\S]*<\/terminal_context>$/.test(threadOf(active).attachments.at(-1).text)), "the context block isn't T3's");
    await page.evaluate(() => {
      threadOf(active).attachments.pop();
      renderComposer(active);
    });
    if (!touch) {
      await page.locator(`${pane} .xterm-helper-textarea`).focus();
      await page.keyboard.press("Meta+d");
      await waitFor("⌘D to split", async () => (await page.locator("#panel-body .term-pane").count()) === 3);
      await page.keyboard.press("Meta+w");
      await waitFor("⌘W to close the terminal, not the tab", async () => (await page.locator("#panel-body .term-pane").count()) === 2 && (await openTab("terminal").count()) === 1);
    }
    await press(gui('#panel-body .term-list [data-term-act="new"]'));
    await waitFor("a new Terminal tab", async () => (await page.locator('#panel-tabs .ptab[data-panel="terminal"]').count()) === 2 && (await text('#panel-tabs .ptab.active[data-panel="terminal"]')) === "Terminal 3");
    await press(gui('#panel-body .term-actions [data-term-act="close"]'));
    await waitFor("its tab to close", async () => (await page.locator('#panel-tabs .ptab[data-panel="terminal"]').count()) === 1);
    await press(gui('#panel-tabs .ptab[data-panel="terminal"]'));
    await press(gui('#panel-tabs .ptab[data-panel="terminal"] .lead'));
    await closePanelIfOpen();
    fs.rmSync(path.join(HOME, "open.log"), { force: true });
  });

  await step("app view: Linked pull requests rows carry T3's state, checks, review, author, repository, branches, age and who linked them, Shift shows quick actions, a watch wakes claude on a new comment, and a row drags into the composer", async () => {
    const pr43 = "https://github.com/example/alpha/pull/43";
    const row = (n) => `#panel-body .pr-row[data-prs-open$="/pull/${n}"]`;
    const stateFile = (n) => path.join(HOME, `.fake-gh-${n}`);
    const kept = Object.fromEntries([41, 42].map((n) => [n, fs.existsSync(stateFile(n)) ? fs.readFileSync(stateFile(n), "utf8") : null]));
    fs.writeFileSync(stateFile(41), "draft");
    fs.writeFileSync(stateFile(42), "open");
    await page.evaluate(() => prCache.clear());
    await page.fill("#prompt", "");
    await openPanelTo("prs", "Linked pull requests");
    await waitFor("#41 loaded", async () => (await page.locator(`${row(41)} .pr-glyph[data-state="draft"]`).count()) === 1, 15000);
    check((await page.getAttribute(`${row(41)} .sig.fail`, "aria-label")) === "Some checks were not successful", "#41 has no failing checks glyph");
    check((await page.getAttribute(`${row(41)} .sig.quiet`, "aria-label")) === "Awaiting review", "#41 has no review glyph");
    check(/^S\s*sweeper\s*example\/alpha\s*sweep\/notes-41\s*→\s*main\s*\d+\w+ ago$/.test((await text(`${row(41)} .meta`)).trim()), `#41's meta: ${await text(`${row(41)} .meta`)}`);
    check(/^Linked by the agent/.test(await page.getAttribute(`${row(41)} .num`, "title")), "#41 doesn't say the agent linked it");
    await waitFor("#42's stack", async () => (await text(`${row(42)} .stack`)).trim() === "2", 15000);
    check((await page.getAttribute(`${row(42)} .sig.ok`, "aria-label")) === "All checks have passed", "#42 has no passing checks glyph");
    await press(gui("#panel-body [data-prs-link]"));
    await page.fill("#link-pr input", pr43);
    await page.press("#link-pr input", "Enter");
    await waitFor("#43 linked", async () => (await page.locator(`${row(43)} .pr-glyph[data-state="open"]`).count()) === 1, 10000);
    check(/^Linked by you/.test(await page.getAttribute(`${row(43)} .num`, "title")), "#43 doesn't say you linked it");
    if (!touch) {
      await page.keyboard.down("Shift");
      await waitFor("the quick actions", async () => await page.locator(`${row(43)} .quick`).isVisible());
      check((await page.locator(`${row(43)} .quick button`).allTextContents()).join("|") === "Close|Merge", `#43's quick actions: ${(await page.locator(`${row(43)} .quick button`).allTextContents()).join("|")}`);
      check(await page.isDisabled(`${row(42)} [data-prs-quick="merge"]`), "a stacked pull request merges from the list");
      check((await page.locator(`${row(41)} .quick button`).allTextContents()).join("|") === "Close|Ready for review", "#41's quick actions");
      await page.click(`${row(43)} [data-prs-quick="close"]`);
      await waitFor("#43 closed", async () => (await page.locator(`${row(43)} .pr-glyph[data-state="closed"]`).count()) === 1, 10000);
      await page.click(`${row(43)} [data-prs-quick="reopen"]`);
      await waitFor("#43 reopened", async () => (await page.locator(`${row(43)} .pr-glyph[data-state="open"]`).count()) === 1, 10000);
      await page.keyboard.up("Shift");
      check(!(await page.locator(`${row(43)} .quick`).isVisible()), "the quick actions outlast Shift");
      check(/pr close .*\/pull\/43/.test(readLog("gh.log")) && /pr reopen .*\/pull\/43/.test(readLog("gh.log")), "gh closed and reopened nothing");
    }
    if (!touch) await page.hover(row(43));
    await press(gui(`#panel-body [data-prs-more="${pr43}"]`));
    check((await menuLabels()).join("|") === "Copy link|Open|Watch for changes|Unlink from thread", `#43's menu: ${(await menuLabels()).join(" | ")}`);
    await press(menuItem("Watch for changes"));
    await waitFor("the watching eye", async () => (await page.getAttribute(`${row(43)} .sig[aria-label="Watching"]`, "title").catch(() => "")) === "Watching: the agent wakes when checks finish, someone comments, or the branch conflicts");
    fs.writeFileSync(path.join(HOME, ".fake-gh-43.json"), JSON.stringify({ comments: [{ author: { login: "monalisa" }, body: "Can this  land\ntoday?", createdAt: "2026-01-02T10:00:00Z" }] }));
    await waitFor("claude woken by the comment", async () => /Update on pull request #43 \(https:\/\/github\.com\/example\/alpha\/pull\/43\), which Pinned is watching for you:[\s\S]*1 new comment:[\s\S]*monalisa: "Can this land today\?"/.test(await text("#thread")), 20000);
    if (!touch) await page.hover(row(43));
    await press(gui(`#panel-body [data-prs-more="${pr43}"]`));
    await press(menuItem("Stop watching"));
    await waitFor("the eye gone", async () => !(await page.locator(`${row(43)} .sig[aria-label="Watching"]`).count()));
    if (!touch) await page.hover(row(43));
    await press(gui(`#panel-body [data-prs-more="${pr43}"]`));
    await press(menuItem("Unlink from thread"));
    await waitFor("#43 unlinked", async () => !(await page.locator(row(43)).count()));
    for (const f of [".fake-gh-43", ".fake-gh-43.json"]) fs.rmSync(path.join(HOME, f), { force: true });
    for (const [n, was] of Object.entries(kept)) (was === null ? fs.rmSync(stateFile(n), { force: true }) : fs.writeFileSync(stateFile(n), was));
    await page.evaluate(() => {
      const data = new DataTransfer();
      document.querySelector('#panel-body .pr-row[data-prs-open$="/pull/42"]').dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: data }));
      const composer = document.getElementById("composer");
      composer.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data }));
      composer.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }));
    });
    await waitFor("#42 in the composer", async () => (await page.inputValue("#prompt")) === "#42 ");
    check(await page.evaluate(() => chipsOf(active)["#42"]?.value === "https://github.com/example/alpha/pull/42"), "the dropped pull request is no chip");
    await page.fill("#prompt", "");
    await page.evaluate(() => prCache.clear());
    await closePanelIfOpen();
  });

  await step("app view: a pull request's title and description edit, its menu closes, reopens, reverts and auto-merges, and its checks, stack, threads and crumbs show", async () => {
    await tapOrClick(await threadPrLink(41));
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
    check((await text("#pr-pop .pop-sub")) === "1 of 3 failing" && (await page.locator("#pr-pop .check-grid .name").allTextContents()).join("|") === "lint|e2e", `checks popover: ${await text("#pr-pop")}`);
    await page.keyboard.press("Escape");
    await press(gui('#panel-body details[data-fold="checks"] > summary'));
    await press(gui('#panel-body .checks .check-row.fail [data-check-rerun]'));
    await waitFor("gh to re-run the job", async () => /^run rerun --job 3$/m.test(readLog("gh.log")));
    await press(gui("#panel-body [data-pr-more]"));
    await press(menuItem("Ask a question"));
    check((await page.inputValue("#prompt")).startsWith("About pull request #41 (https://github.com/example/alpha/pull/41): "), `Ask a question wrote ${await page.inputValue("#prompt")}`);
    await page.fill("#prompt", "");
    await press(gui('#panel-body .checks .check-row.fail [data-check-fix]'));
    await waitFor("claude asked to fix lint", async () => /The check "lint" fails on pull request #41/.test(await text("#thread")), 10000);
    check(await gui('#panel-body .review-outcome[aria-label="Review required"]').isVisible(), "no review outcome icon");
    await press(gui("#panel-body [data-pr-more]"));
    const items = await menuLabels();
    for (const want of ["Ask a question", "Explain this PR", "Fix findings in this thread", "Write guide", "Convert to draft", "Merge now", "Enable auto-merge", "Unlink from this thread", "Close pull request"]) check(items.includes(want), `More menu: ${items.join(" | ")}`);
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
    await tapOrClick(await threadPrLink(42));
    await waitFor("#42", async () => /Merged/.test(await text("#panel-body .pr > .crumbs")), 15000);
    await press(gui('#panel-body [data-pr-tab="summary"]'));
    await waitFor("its stack", () => gui("#panel-body [data-pr-stack]").isVisible(), 10000);
    await press(gui("#panel-body [data-pr-stack]"));
    check((await menuLabels()).includes("Draft the notes"), `stack: ${(await menuLabels()).join(" | ")}`);
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
    await tapOrClick(await threadPrLink(42));
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
    await press(gui("#panel-body [data-composer]"));
    await page.fill("#panel-body [data-composer-text]", "Ready when you are.");
    await press(gui("#panel-body [data-composer-send]"));
    await waitFor("the comment", async () => /Ready when you are\./.test(await text("#panel-body .conversation")), 10000);
    await press(gui('#panel-body [data-pr-tab="code"]'));
    if ((await page.locator('#panel-body .file[data-path="notes.md"]').getAttribute("data-collapsed")) === "") await press(gui('#panel-body .file[data-path="notes.md"] > header .change'));
    await page.hover('#panel-body .file[data-path="notes.md"] .no[data-line="2"]');
    await press(gui('#panel-body .file[data-path="notes.md"] .no[data-line="2"] [data-comment-line]'));
    await page.fill("#panel-body [data-comment-text]", "Say why");
    await press(gui("#panel-body [data-comment-save]"));
    await waitFor("the pending comment", async () => (await page.getAttribute("#panel-body [data-composer]", "aria-label")) === "Review pull request, 1 comment pending" && (await text("#panel-body [data-composer] .count")) === "1");
    check(!/review /.test(readLog("gh.log")), "a pending comment went out on its own");
    await press(gui("#panel-body [data-composer]"));
    check(await gui('#panel-body .composer-pop [data-composer-mode="review"].on').isVisible(), "the composer didn't open on the review with a comment pending");
    await page.fill("#panel-body [data-review-body]", "Two small notes.");
    await press(gui("#panel-body [data-verdict-pick]"));
    check((await page.locator("#pr-pop [data-verdict]").allTextContents()).join("|") === "Comment|Approve|Request changes", `verdicts: ${await text("#pr-pop")}`);
    await press(gui('#pr-pop [data-verdict="REQUEST_CHANGES"]'));
    check((await text("#panel-body [data-verdict-pick]")) === "Request changes", `the verdict reads ${await text("#panel-body [data-verdict-pick]")}`);
    await press(gui("#panel-body [data-review-submit]"));
    const review = () => JSON.parse(readLog("gh.log").split("\n").find((l) => l.startsWith("review "))?.slice(7) ?? "null");
    await waitFor("the review", async () => !!review(), 10000);
    check(sortedJson(review()) === sortedJson({ body: "Two small notes.", comments: [{ body: "Say why", line: 2, path: "notes.md", side: "RIGHT" }], event: "REQUEST_CHANGES" }), `review: ${JSON.stringify(review())}`);
    await press(gui('#panel-body [data-pr-tab="summary"]'));
    await closePanelIfOpen();
  });

  await step("app view: the Guide tab has claude write a review guide and walks its stops", async () => {
    await tapOrClick(await threadPrLink(42));
    await waitFor("#42", async () => /#42/.test(await text("#panel-body .pr > .crumbs")), 15000);
    await press(gui('#panel-body [data-pr-tab="guide"]'));
    await waitFor("the empty guide", () => gui("#panel-body [data-guide-write]").isVisible());
    await press(gui("#panel-body [data-guide-write]"));
    await waitFor("claude writing", async () => /Writing a review guide/.test(await text("#panel-body .pr-body")));
    await waitFor("the guide", async () => /The notes file arrives/.test(await text("#panel-body .stop h4")), 20000);
    check(/-p --output-format json --json-schema --allowedTools Read,Grep,Glob/.test(readLog("claude-p.log")) && /change request #42/.test(readLog("claude-p.log")), `claude ran: ${readLog("claude-p.log")}`);
    check(/A product call\s*Should notes live at the root\?/.test(await text("#panel-body .stop .attention")), `attention: ${await text("#panel-body .stop .attention")}`);
    check((await page.locator("#panel-body .stop .excerpt .dl").count()) === 2, "the excerpt doesn't show the two new lines");
    await press(gui('#panel-body .stop footer [data-guide-step="1"]'));
    await waitFor("stop 2", async () => (await text("#panel-body .stop header .step")) === "Stop 2 of 2");
    await press(gui('#panel-body [data-pr-tab="summary"]'));
    await closePanelIfOpen();
  });

  const openPrPanel = async (n) => {
    await tapOrClick(await threadPrLink(n));
    await waitFor(`#${n}`, async () => new RegExp(`#${n}`).test(await text("#panel-body .pr > .crumbs")) && !(await page.locator("#panel-body .pr-ghost").count()), 15000);
  };
  const style = (selector, prop) => page.locator(selector).first().evaluate((el, p) => getComputedStyle(el)[p], prop);
  const opened = () => page.evaluate(() => window.__opened);

  await step("app view: a pull request's crumbs open its repository, step back to the thread's pull requests, give #N a context menu, and cross-fade as they condense", async () => {
    await page.evaluate(() => (window.open = (url) => (window.__opened = url)));
    await openPrPanel(42);
    check((await page.getAttribute("#panel-body .crumb-row.full .repo", "title")) === "Open example/alpha repository", "the repository isn't a button that opens it");
    await press(gui("#panel-body .crumb-row.full .repo"));
    check((await opened()) === "https://github.com/example/alpha", `the repository opened ${await opened()}`);
    if (!touch) {
      await page.click("#panel-body .crumb-row.full [data-pr-num]", { button: "right" });
      check((await menuLabels()).join("|") === "Copy link|Open on GitHub", `#42's context menu: ${(await menuLabels()).join("|")}`);
      await press(menuItem("Open on GitHub"));
      check(/\/pull\/42$/.test(await opened()), `the context menu opened ${await opened()}`);
    }
    const rows = () => page.locator("#panel-body .crumb-row").evaluateAll((els) => els.map((el) => `${getComputedStyle(el).opacity} ${el.inert}`));
    check(JSON.stringify(await rows()) === JSON.stringify(["1 false", "0 true"]), `the crumbs start as ${await rows()}`);
    check((await style("#panel-body .crumb-row.full", "transitionProperty")) === "opacity, transform", "the crumbs don't cross-fade");
    await page.evaluate(() => (panelBody.style.maxHeight = "240px"));
    await page.evaluate(() => {
      panelBody.scrollTop = 200;
      panelBody.dispatchEvent(new Event("scroll"));
    });
    await waitFor("the crumbs to cross-fade to #42 and its title", async () => JSON.stringify(await rows()) === JSON.stringify(["0 true", "1 false"]));
    check((await text("#panel-body .crumb-row.short .condensed-title")) === "Add notes", "the condensed crumbs miss the title");
    await page.evaluate(() => {
      panelBody.scrollTop = 0;
      panelBody.dispatchEvent(new Event("scroll"));
      panelBody.style.maxHeight = "";
    });
    await waitFor("the crumbs to come back", async () => JSON.stringify(await rows()) === JSON.stringify(["1 false", "0 true"]));
    await press(gui("#panel-body .crumb-row.full [data-pr-back]"));
    await waitFor("the thread's pull requests", () => gui("#panel-body .prs-list").isVisible());
    await closePanelIfOpen();
  });

  await step("app view: a pull request's title truncates under a hover pencil and edits with Cancel and Save below it, its checkout and branch copy in place, and its menu matches T3's", async () => {
    await openPrPanel(42);
    check(`${await style("#panel-body .pr-head h3", "whiteSpace")} ${await style("#panel-body .pr-head h3", "textOverflow")}` === "nowrap ellipsis", "the title wraps");
    if (!touch) {
      await page.mouse.move(0, 0);
      check((await style("#panel-body .title-row .edit-btn", "opacity")) === "0", "the pencil shows without a hover");
      await page.hover("#panel-body .title-row h3");
      await waitFor("the pencil on hover", async () => (await style("#panel-body .title-row .edit-btn", "opacity")) === "1");
    }
    await press(gui('#panel-body [data-edit="title"]'));
    const [input, cancel, save] = [await rect('#panel-body [data-pr-edit="title"]'), await rect("#panel-body .edit-title [data-edit-cancel]"), await rect("#panel-body .edit-title [data-edit-save]")];
    check(cancel.top >= input.bottom && save.left > cancel.left && save.right <= input.right + 1, `Cancel and Save don't sit under the title: ${JSON.stringify([input, cancel, save])}`);
    await press(gui("#panel-body .edit-title [data-edit-cancel]"));
    check((await page.getAttribute("#panel-body .refs .copyable", "title")) === "Copy pull request branch: sweep/notes-42", "the branch doesn't copy");
    await press(gui("#panel-body .byline .copyable"));
    if (engine === "chromium") {
      await waitFor("Copied in place", async () => (await page.getAttribute("#panel-body .byline .copyable", "aria-label")) === "Checkout command copied" && (await style("#panel-body .byline .copyable .copied", "opacity")) === "1");
      check((await page.evaluate(() => navigator.clipboard.readText())) === "gh pr checkout 42", "the checkout command isn't on the clipboard");
      await waitFor("the command to come back", async () => (await style("#panel-body .byline .copyable code", "opacity")) === "1", 4000);
    }
    await press(gui("#panel-body [data-pr-more]"));
    const labels = await menuLabels();
    check(labels[0] === "Unlink from this thread" && labels.includes("Fix findings in this thread"), `More menu: ${labels.join(" | ")}`);
    const mod = await page.evaluate(() => (IS_MAC ? "⌘⇧" : "Ctrl+Shift+"));
    check((await text('#panel-menu button:has(.label:text-is("Copy link")) kbd')) === `${mod}C`, "Copy link has no ⌘⇧C");
    const del = '#panel-menu button.danger:has(.label:text-is("Delete branch"))';
    check((await text(`${del} .detail`)) === "sweep/notes-42", "Delete branch doesn't name the branch");
    check((await style(del, "color")) !== (await style('#panel-menu button:has(.label:text-is("Copy link"))', "color")), "Delete branch isn't destructive");
    await press(gui(del));
    await waitFor("the delete dialog", async () => (await text("#shell-dialog h2")) === "Delete branch sweep/notes-42?");
    check(/This deletes sweep\/notes-42 from example\/alpha\. Local branches and worktrees are not changed\./.test(await text("#shell-dialog .dialog-head p")), `dialog: ${await text("#shell-dialog .dialog-head p")}`);
    await press(gui("#shell-dialog [data-delete-branch]"));
    await waitFor("gh to delete the branch", async () => /api -X DELETE repos\/example\/alpha\/git\/refs\/heads\/sweep\/notes-42/.test(readLog("gh.log")), 10000);
    await closePanelIfOpen();
    return labels.join(" | ");
  });

  await step("app view: the checks popover is a grid with a running check's spinner, and the Summary's checks open as a whole with a labeled Fix", async () => {
    await page.evaluate(() => (window.open = (url) => (window.__opened = url)));
    await openPrPanel(41);
    await press(gui('#panel-body [data-pr-tab="summary"]'));
    check((await text("#panel-body .checks-line")) === "1 of 3 failing", `checks line: ${await text("#panel-body .checks-line")}`);
    await press(gui("#panel-body [data-checks-pop]"));
    check((await text("#pr-pop .pop-title")) === "Some checks were not successful" && (await text("#pr-pop .pop-sub")) === "1 of 3 failing", `popover: ${await text("#pr-pop")}`);
    const grid = () => page.locator("#pr-pop .check-grid li").evaluateAll((els) => els.map((li) => [...li.children].map((c) => c.textContent.trim() || c.className).join(" ")));
    check(JSON.stringify(await grid()) === JSON.stringify(["ck fail lint Failed Details", "spinner small amber e2e Running "]), `grid: ${JSON.stringify(await grid())}`);
    check((await style("#pr-pop .check-grid", "display")) === "grid" && (await style("#pr-pop .check-grid", "maxHeight")) === "256px", "the checks aren't a 256px grid");
    await press(gui("#pr-pop [data-checks-all]"));
    await waitFor("every check", async () => (await page.locator("#pr-pop .check-grid .name").allTextContents()).join("|") === "lint|e2e|build" && (await text("#pr-pop [data-checks-all]")) === "Show less");
    await press(gui('#pr-pop [data-check-details*="runs/2/job/3"]'));
    check(/runs\/2\/job\/3$/.test(await opened()), `Details opened ${await opened()}`);
    await page.keyboard.press("Escape");
    check(!(await page.locator("#pr-pop").isVisible()), "Escape didn't close the checks");
    if ((await page.getAttribute('#panel-body details[data-fold="checks"]', "open")) === null) await press(gui('#panel-body details[data-fold="checks"] > summary'));
    check((await page.locator("#panel-body .check-row").count()) === 3, "the Summary lacks a row per check");
    check((await text("#panel-body .check-row.fail [data-check-fix]")) === "Fix in this thread", `Fix reads ${await text("#panel-body .check-row.fail [data-check-fix]")}`);
    await press(gui("#panel-body .check-row.ok .go"));
    check(/runs\/1$/.test(await opened()), `the build row opened ${await opened()}`);
    check(await page.locator("#panel-body .check-row.running .go").isDisabled(), "a check with no link opens");
    await closePanelIfOpen();
  });

  await step("app view: reviewers show as avatars ringed by their verdict, and the reviewer and label pickers ask, take back, put on and take off", async () => {
    await openPrPanel(42);
    await press(gui('#panel-body [data-pr-tab="summary"]'));
    const faces = () => page.locator("#panel-body .faces .face").evaluateAll((els) => els.map((el) => `${el.dataset.reviewer}:${el.className.replace(/^face ?/, "")}`));
    await waitFor("the reviewers' faces", async () => (await faces()).join() === "monalisa:,hubot:ring approved stale,sweeper:ring changes-requested", 10000);
    check((await page.getAttribute('#panel-body .face[data-reviewer="hubot"]', "title")) === "hubot: Approved earlier changes", "hubot's tooltip");
    check((await page.getAttribute('#panel-body .face[data-reviewer="monalisa"]', "title")) === "Mona Lisa (@monalisa)", "monalisa's tooltip");
    check((await style('#panel-body .face[data-reviewer="sweeper"]', "boxShadow")).includes("2px"), "the verdict has no ring");
    await waitFor("monalisa's face, or a letter where it didn't load", () => page.locator('#panel-body .face[data-reviewer="monalisa"] .avatar').evaluate((el) => (el.tagName === "IMG" ? el.complete && el.naturalWidth > 0 : el.textContent === "M")), 10000);
    await press(gui('#panel-body [data-pick="reviewers"]'));
    await waitFor("who may review", async () => (await page.locator("#pr-pop .cand .who").allTextContents()).join() === "monalisa,octocat,hubot", 10000);
    await page.fill("#pr-pop .pop-search", "octo");
    await waitFor("the search", async () => (await page.locator("#pr-pop .cand .who").allTextContents()).join() === "octocat");
    await press(gui("#pr-pop .cand"));
    await waitFor("gh to ask octocat", async () => /api -X POST repos\/example\/alpha\/pulls\/42\/requested_reviewers --input -/.test(readLog("gh.log")) && (await page.locator("#pr-pop .cand .on").count()) === 1, 10000);
    await waitFor("octocat's face", async () => (await faces()).some((f) => f.startsWith("octocat:")), 10000);
    await press(gui("#pr-pop .cand"));
    await waitFor("the request taken back", async () => /api -X DELETE repos\/example\/alpha\/pulls\/42\/requested_reviewers/.test(readLog("gh.log")) && !(await faces()).some((f) => f.startsWith("octocat:")), 10000);
    await page.keyboard.press("Escape");
    await press(gui('#panel-body [data-pick="labels"]'));
    await waitFor("the labels", async () => (await page.locator("#pr-pop .cand .who").allTextContents()).map((t) => t.split(" · ")[0]).join() === "bug,documentation,enhancement", 10000);
    check((await page.locator("#pr-pop .cand .on").count()) === 1, "documentation isn't marked as on");
    await press(gui('#pr-pop [data-cand="bug"]'));
    await waitFor("bug on", async () => /api -X POST repos\/example\/alpha\/issues\/42\/labels/.test(readLog("gh.log")) && (await page.locator("#panel-body .meta .chip.label").allTextContents()).join() === "bug,documentation", 10000);
    await press(gui('#pr-pop [data-cand="documentation"]'));
    await waitFor("documentation off", async () => /api -X DELETE repos\/example\/alpha\/issues\/42\/labels\/documentation/.test(readLog("gh.log")) && (await page.locator("#panel-body .meta .chip.label").allTextContents()).join() === "bug", 10000);
    await page.keyboard.press("Escape");
    await closePanelIfOpen();
  });

  await step("app view: comments carry reactions that toggle at once, a thread says how many more replies GitHub holds, the composer comments, your own comments edit in place, and it closes with a comment", async () => {
    await openPrPanel(42);
    await press(gui('#panel-body [data-pr-tab="summary"]'));
    const heart = '#panel-body [data-react="IC_1"][data-content="HEART"]';
    await waitFor("hubot's comment's heart", () => gui(heart).isVisible(), 10000);
    check((await text(heart)) === "❤️1" && (await page.getAttribute(heart, "title")) === "octocat reacted with heart emoji", `heart: ${await text(heart)} ${await page.getAttribute(heart, "title")}`);
    await press(gui(heart));
    check((await text(heart)) === "❤️2" && (await page.getAttribute(heart, "aria-pressed")) === "true", "the reaction didn't count at once");
    await waitFor("gh to react", async () => /reaction add IC_1 HEART/.test(readLog("gh.log")), 10000);
    await press(gui('#panel-body .conversation .thread [data-react-pick="c1"]'));
    check((await page.locator("#pr-pop .emoji-row button").count()) === 8, "the picker lacks GitHub's eight reactions");
    await press(gui('#pr-pop [data-emoji="ROCKET"]'));
    await waitFor("the rocket", async () => (await text('#panel-body [data-react="c1"][data-content="ROCKET"]')) === "🚀1" && /reaction add c1 ROCKET/.test(readLog("gh.log")), 10000);
    check(await gui('#panel-body .desc-reactions [data-react-pick="PR_42"]').isVisible(), "the description has no reactions");
    check((await text("#panel-body .conversation .thread .more-replies")) === "1 more reply View on host", `more replies: ${await text("#panel-body .conversation .thread .more-replies")}`);
    await press(gui("#panel-body [data-composer]"));
    check((await page.locator("#panel-body .composer-pop [data-composer-mode]").allTextContents()).join("|") === "Comment|Review" && (await page.locator("#panel-body [data-composer-send]").isDisabled()), "the composer");
    await page.fill("#panel-body [data-composer-text]", "Merged, thanks.");
    await press(gui("#panel-body [data-composer-send]"));
    await waitFor("the comment", async () => /Merged, thanks\./.test(await text("#panel-body .conversation")) && !(await page.locator("#panel-body .composer-pop").count()), 10000);
    check(!(await page.locator('#panel-body .conversation [data-edit="comment:issue:IC_1"]').count()), "hubot's comment offers an edit");
    const mine = page.locator("#panel-body .conversation li.comment", { hasText: "Merged, thanks." });
    await press(mine.locator('[data-edit^="comment:issue:"]'));
    check((await mine.locator(".md-editor [data-md-mode]").allTextContents()).join("|") === "Write|Preview", "the comment's editor isn't T3's markdown editor");
    await mine.locator("[data-pr-edit]").fill("Merged, thank you.");
    await press(mine.locator("[data-edit-save]"));
    await waitFor("the comment rewritten", async () => /comment edit IC_x\d+ Merged, thank you\./.test(readLog("gh.log")) && /Merged, thank you\./.test(await text("#panel-body .conversation")) && !(await page.locator("#panel-body .conversation .md-editor").count()), 10000);
    const reply = page.locator("#panel-body .conversation .replies .comment", { hasText: "Because two is clearer." });
    await press(reply.locator('[data-edit^="comment:line:"]'));
    await reply.locator("[data-pr-edit]").fill("Because two lines read clearer.");
    await page.keyboard.press("ControlOrMeta+Enter");
    await waitFor("the reply rewritten", async () => /comment edit r0 Because two lines read clearer\./.test(readLog("gh.log")) && /Because two lines read clearer\./.test(await text("#panel-body .conversation")), 10000);
    await openPrPanel(41);
    await press(gui("#panel-body [data-composer]"));
    await page.fill("#panel-body [data-composer-text]", "Closing for now.");
    await press(gui("#panel-body [data-composer-follow]"));
    check((await text("#panel-body [data-composer-follow] .hold > span:last-child")) === "Confirm close?", "closing didn't ask again");
    await press(gui("#panel-body [data-composer-follow]"));
    await waitFor("#41 closed with a comment", async () => /pr comment --body Closing for now\. -- https:\/\/github\.com\/example\/alpha\/pull\/41\npr close -- https:\/\/github\.com\/example\/alpha\/pull\/41/.test(readLog("gh.log")) && /Closed/.test(await text("#panel-body .pr > .crumbs")), 10000);
    await press(gui("#panel-body [data-pr-more]"));
    await press(menuItem("Reopen pull request"));
    await waitFor("#41 reopened", async () => !/Closed/.test(await text("#panel-body .pr > .crumbs")), 10000);
    await closePanelIfOpen();
  });

  await step("app view: the Code tab draws the pull request's review threads on their lines, resolves and replies there, and Viewed sits after the counts", async () => {
    await openPrPanel(42);
    await press(gui('#panel-body [data-pr-tab="code"]'));
    if ((await page.locator('#panel-body .file[data-path="notes.md"]').getAttribute("data-collapsed")) === "") await press(gui('#panel-body .file[data-path="notes.md"] > header .change'));
    const card = '#panel-body .file[data-path="notes.md"] .pr-thread[data-thread="T1"]';
    await waitFor("T1 on its line", () => gui(card).isVisible(), 10000);
    check(/Open · 3 comments/.test(await text(`${card} .head`)) && /1 more reply/.test(await text(card)), `the thread: ${await text(card)}`);
    const order = await page.locator('#panel-body .file[data-path="notes.md"] > header').evaluate((h) => [...h.children].map((c) => (c.getAttribute("class") ?? "").split(" ")[0] || c.tagName));
    check(order.indexOf("viewed") > order.indexOf("add") && order.indexOf("add") > -1, `the header reads ${order.join(" ")}`);
    await press(gui(`${card} [data-thread-resolve]`));
    await waitFor("T1 resolved", async () => /Resolved · 3 comments/.test(await text(`${card} .head`)) && !(await page.locator(`${card} .comments`).count()), 10000);
    check(/resolveReviewThread\(input: \{threadId: \$thread\}\)/.test(readLog("gh.log")), "gh didn't resolve T1");
    await press(gui(`${card} [data-thread-toggle]`));
    await waitFor("T1 open again to read", () => gui(`${card} .comments`).isVisible());
    await press(gui(`${card} [data-thread-resolve]`));
    await waitFor("T1 unresolved", async () => /Open · 3 comments/.test(await text(`${card} .head`)), 10000);
    await press(gui(`${card} [data-thread-reply]`));
    await page.fill(`${card} [data-thread-text]`, "On the line itself.");
    await press(gui(`${card} [data-thread-send]`));
    await waitFor("the reply on the line", async () => /On the line itself\./.test(await text(card)), 10000);
    await press(gui('#panel-body [data-pr-tab="summary"]'));
    await closePanelIfOpen();
  });

  await step("app view: the stack menu says which layer of how many, rebases and merges the stack, and More links the pull request to another thread from the Pull requests page", async () => {
    fs.writeFileSync(path.join(HOME, ".fake-gh-stack.json"), JSON.stringify({ 41: false }));
    await openPrPanel(41);
    const button = "#panel-body [data-pr-stack]";
    await waitFor("its stack", async () => (await text(button)) === "2/2", 10000);
    check((await page.getAttribute(button, "aria-label")) === "Stack 7, layer 2 of 2", `the stack button: ${await page.getAttribute(button, "aria-label")}`);
    await press(gui(button));
    const labels = await menuLabels();
    check(labels.join("|") === "Draft the notes|Add notes|Merge stack (1)|Rebase stack" && (await text("#panel-menu .group")) === "Stack #7", `stack menu: ${labels.join(" | ")}`);
    await press(menuItem("Rebase stack"));
    await waitFor("the rebase dialog", async () => (await text("#shell-dialog h2")) === "Rebase 1 pull requests?");
    await press(gui("#shell-dialog [data-stack-run]"));
    const log41 = () => require("child_process").execFileSync("git", ["-C", path.join(HOME, "remote-alpha.git"), "log", "--format=%s", "sweep/notes-41"], { encoding: "utf8" });
    await waitFor("#41 rebased onto main", async () => /later\.md\nbase\.md\n$/.test(log41()), 20000);
    check(log41() === "draft.md\nnotes.md\nlater.md\nbase.md\n", `#41's branch: ${log41()}`);
    await waitFor("Merge stack", async () => !(await page.locator("#panel-body [data-stack-merge]").isDisabled()), 10000);
    await press(gui("#panel-body [data-stack-merge]"));
    await waitFor("the merge dialog", async () => (await text("#shell-dialog h2")) === "Merge 1 pull requests?");
    await press(gui("#shell-dialog [data-stack-run]"));
    const merged = () => readLog("gh.log").split("\n").filter((l) => l.startsWith("merge-async 41 ")).map((l) => JSON.parse(l.slice(15)));
    await waitFor("GitHub's merge-async", async () => merged().some((b) => /^[0-9a-f]{40}$/.test(b.sha) && sortedJson({ ...b, sha: "" }) === sortedJson({ merge_action: "default", merge_method: "merge", sha: "" })), 15000);
    await waitFor("#41 merged", async () => /Merged/.test(await text("#panel-body .pr > .crumbs")), 10000);
    // Reopening #41 for the later steps undoes what GitHub never does, so the app, which asks gh no
    // more about a merged pull request, forgets it too.
    fs.writeFileSync(path.join(HOME, ".fake-gh-41"), "open");
    await page.evaluate(() => prStatuses.delete("https://github.com/example/alpha/pull/41"));
    fs.rmSync(path.join(HOME, ".fake-gh-stack.json"));
    await closePanelIfOpen();
    await page.evaluate(() => openPage("pull-requests"));
    await page.evaluate(() => openPull("https://github.com/example/alpha/pull/42"));
    await waitFor("#42 beside the page", async () => /#42/.test(await text("#panel-body .pr > .crumbs")), 15000);
    await press(gui("#panel-body [data-pr-more]"));
    check((await menuLabels())[0] === "Link to thread", `More on the page: ${(await menuLabels()).join(" | ")}`);
    await press(menuItem("Link to thread"));
    await waitFor("Choose a thread", () => gui("#shell-dialog .thread-picker input").isVisible());
    check((await page.getAttribute("#shell-dialog .thread-picker input", "placeholder")) === "Search threads or projects...", "the picker's search");
    const free = "#shell-dialog .thread-pick button:not([disabled])";
    const linked = await page.locator("#shell-dialog .thread-pick button[disabled]").count();
    check(linked >= 1 && /Linked/.test(await text("#shell-dialog .thread-pick button[disabled]")), "the thread linking #42 isn't marked Linked");
    const pick = await page.locator(free).first().getAttribute("data-pick-thread");
    await press(page.locator(free).first());
    check(await page.evaluate((key) => linkedPrs(tabs.find((t) => t.key === key)).some((pr) => pr.url.endsWith("/pull/42")), pick), "the picked thread didn't link #42");
    await page.evaluate((key) => unlinkPr(tabs.find((t) => t.key === key), "https://github.com/example/alpha/pull/42"), pick);
    await page.evaluate(() => closePage());
    await closePanelIfOpen();
  });

  await step("app view: the Guide tab puts its rail beside the stop when wide, steps with J and K, marks the stops read, asks about a question and finishes into the review", async () => {
    await openPrPanel(42);
    await press(gui('#panel-body [data-pr-tab="guide"]'));
    await waitFor("the guide", () => gui("#panel-body .guide .stop h4").isVisible(), 15000);
    if ((await text("#panel-body .stop header .step")) !== "Stop 1 of 2") await press(gui('#panel-body .stop [data-guide-step="-1"]'));
    check((await style("#panel-body .guide .side", "display")) === "none" && (await gui("#panel-body .rail-toggle").isVisible()), "a narrow guide doesn't fold its rail");
    await press(gui("#panel-body .rail-toggle"));
    check((await page.locator("#panel-body .rail-drop .guide-rail li").count()) === 2, "the folded rail");
    await page.evaluate(() => (panelBody.querySelector(".guide").style.width = "960px"));
    check((await style("#panel-body .guide .side", "display")) === "block" && (await style("#panel-body .guide .side", "position")) === "sticky", "a wide guide doesn't put its rail beside the stop");
    check((await rect("#panel-body .guide .side")).right <= (await rect("#panel-body .guide main")).left, "the rail isn't beside the stop");
    await page.evaluate(() => (panelBody.querySelector(".guide").style.width = ""));
    check((await text("#panel-body .stop footer")) === "Next: Nothing else changes", `footer: ${await text("#panel-body .stop footer")}`);
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press("j");
    await waitFor("J to stop 2", async () => (await text("#panel-body .stop header .step")) === "Stop 2 of 2");
    check((await text("#panel-body .guide .side .read")) === "1 of 2 read" && (await page.locator("#panel-body .guide .side .n.seen").count()) === 1, "stop 1 isn't marked read");
    check((await text("#panel-body .stop footer")) === "Finish review", "the last stop doesn't finish the review");
    await page.keyboard.press("k");
    await waitFor("K to stop 1", async () => (await text("#panel-body .stop header .step")) === "Stop 1 of 2");
    await press(gui('#panel-body .guide-questions [data-guide-ask="0"]'));
    check((await page.inputValue("#prompt")) === "Why a separate file?" && /PR #42 review guide · Stop 1/.test(await text("#attachments")), `asked: ${await page.inputValue("#prompt")} with ${await text("#attachments")}`);
    await page.evaluate(() => {
      threadOf(active).attachments = [];
      prompt.value = "";
      prompt.dispatchEvent(new Event("input"));
      renderComposer(active);
    });
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press("j");
    await waitFor("stop 2", async () => (await text("#panel-body .stop header .step")) === "Stop 2 of 2");
    await press(gui("#panel-body [data-guide-finish]"));
    await waitFor("the review composer", () => gui('#panel-body .composer-pop [data-composer-mode="review"].on').isVisible());
    await press(gui("#panel-body [data-composer-close]"));
    await press(gui('#panel-body [data-pr-tab="summary"]'));
    await closePanelIfOpen();
  });

  await step("app view: panel tabs scroll with chevrons, PR links open on GitHub with ⌘ or a middle click, and the panel can animate", async () => {
    await page.evaluate(() => (window.open = (url) => (window.__opened = url)));
    if (!touch) {
      await showSidebar();
      await page.click("#composer-footer .pr[data-pr]", { modifiers: ["Meta"] });
      await waitFor("the badge to open GitHub", async () => /github\.com\/example\/alpha\/pull\/4\d/.test((await page.evaluate(() => window.__opened)) ?? ""));
      await page.evaluate(() => (window.__opened = ""));
      await page.click(await threadPrLink(41), { button: "middle" });
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
    await settingsNav("appearance");
    await page.locator("#panel-animation-duration").fill("200");
    await page.evaluate(() => toggleSettings());
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
    await tapOrClick("#toggle-panel");
    check(await page.evaluate(() => document.body.classList.contains("panel-animating")), "opening didn't animate");
    await waitFor("the animation to end", async () => !(await page.evaluate(() => document.body.classList.contains("panel-animating"))));
    await page.evaluate(() => save("gui-panel-animation", null));
    await closePanelIfOpen();
  });

  await step("app view: Open in an editor from the header", async () => {
    await waitFor("the Open button", () => page.locator(touch ? "#head-more" : "#open-in").isVisible());
    await tapOrClick(touch ? "#head-more" : "#open-in .pick");
    const editors = await page.locator("#panel-menu button .label").allTextContents();
    check(editors.includes("Zed") && editors.includes("Finder"), `editors: ${editors.join(" | ")}`);
    await press(gui('#panel-menu button:has-text("Zed")'));
    // The phone's Options menu opens the editor it names; the desktop's picker only picks one.
    if (!touch) {
      await sleep(300);
      check(!readLog("open.log"), "picking an editor opened it");
      await tapOrClick("#open-in .main");
    }
    const opened = new RegExp(`^-a .*/home/Applications/Zed\\.app ${HOME.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}/work/alpha$`, "m");
    await waitFor("the folder to open in Zed", async () => opened.test(readLog("open.log")));
    if (touch) {
      // The thread details card holds Open too, as T3 Code's does.
      await tapOrClick("#toggle-details");
      await tapOrClick('#details-card [data-act="editors"]');
      const listed = await page.locator("#header-menu button .label").allTextContents();
      check(listed.includes("Zed") && listed.includes("Finder"), `the card's editors: ${listed.join(" | ")}`);
      await press(gui('#header-menu button:has-text("Zed")'));
      await tapOrClick('#details-card [data-act="open-in"]');
      await waitFor("the card to open it again", async () => readLog("open.log").trim().split("\n").length === 2);
      await tapOrClick("#toggle-details");
    }
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
    check(!/Do you want to proceed/.test(await page.locator("#approval").innerText()), "the banner repeats claude's question, which T3 Code's doesn't");
    await press(gui("#approval [data-approval-more]"));
    const options = (await page.locator('#composer-menu[data-kind="approval"] button').allTextContents()).map((o) => o.trim());
    check(options.length === 3 && options[1].startsWith("Always allow") && options[1].endsWith("Yes, and don't ask again for rm commands"), `options: ${options.join(" | ")}`);
    await page.fill("#prompt", "typed while claude asks");
    check(!/typed while claude asks/.test(await termText()), "the composer typed into claude's menu");
    await page.fill("#prompt", "");
    const model = await text("#model-button");
    await tapOrClick("#model-button");
    await press(page.locator("#composer-menu .model").first());
    await sleep(300);
    check(!/\/model/.test(await termText()), "picking a model typed into claude's menu");
    check((await text("#model-button")) === model, `the model button changed to ${await text("#model-button")} with nothing sent`);
    check((await text("#composer-note")) === "This applies once claude finishes the turn.", `the pick wasn't held for the turn: ${await text("#composer-note")}`);
    await press(gui("#approval [data-approval-more]"));
    await press(page.locator('#composer-menu[data-kind="approval"] button', { hasText: "Always allow" }));
    await waitTerm(/chose 2/);
    await waitFor("the banner to go", async () => !(await page.locator("#approval").isVisible()), 1500);
    const row = await text("#thread-list .thread-row.active");
    check(!/ran it/.test(await termText()), "claude finished before the banner was checked");
    check(!/Approval/.test(row), `after the answer, while claude still works: ${JSON.stringify(row)}`);
    await waitTerm(/ran it/);
    await waitFor("the held model pick to apply", async () => (await text("#model-button")) === "Fable 5.1", 10000);
    return options.join(" | ");
  });

  await step("app view: claude's folder trust menu, answered from the banner", async () => {
    await showSidebar();
    await tapOrClick("#add-project");
    await press(page.locator("#palette .item", { hasText: "Local folder" }));
    await page.fill("#palette input", `${HOME}/work/gamma/`);
    await waitFor("gamma's folders listed", async () => !(await page.locator("#palette .notice").isVisible()) && (await text("#palette .accessory [data-submit]")).startsWith("Add"));
    await press(page.locator("#palette .accessory [data-submit]"));
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

  await step("composer: the dialog parser reads claude 2.1's /usage, /status, /mcp and /model", async () => {
    const screen = (name) => fs.readFileSync(path.join(__dirname, "screens", name), "utf8").split("\n");
    const dialogs = await page.evaluate((screens) => screens.map((lines) => dialogOn(lines)?.join("\n") ?? null), ["usage", "status", "mcp", "model"].map((s) => screen(`claude-2.1.310-${s}.txt`)));
    const [usage, status, mcp, model] = dialogs;
    check(/^\s+Settings\s+Status\s+Config\s+Usage/.test(usage) && /Current session/.test(usage) && !/~\/work\/app/.test(usage), `/usage reads as ${JSON.stringify(usage?.slice(0, 80))}`);
    check(/Esc to cancel$/.test(status) && /Version:\s+2\.1\.310/.test(status), `/status reads as ${JSON.stringify(status?.slice(-60))}`);
    check(/Manage MCP servers/.test(mcp), `/mcp reads as ${JSON.stringify(mcp?.slice(0, 60))}`);
    check(/Select model/.test(model), `/model reads as ${JSON.stringify(model?.slice(0, 60))}`);
    const none = await page.evaluate((screens) => screens.map((lines) => dialogOn(lines)), ["idle", "permission", "question", "plan", "manual-mode"].map((s) => screen(`claude-2.1.310-${s}.txt`)));
    check(none.every((d) => d === null), `a prompt or menu read as a dialog: ${JSON.stringify(none)}`);
    const shown = await page.evaluate((lines) => dialogOn(lines), [...screen("claude-2.1.310-usage.txt").slice(0, 10), ...screen("claude-2.1.310-manual-mode.txt").slice(-5)]);
    check(shown === null, `a ▔ rule printed above claude's prompt box read as a dialog: ${JSON.stringify(shown)}`);
    return dialogs.map((d) => d.split("\n").length).join(" | ");
  });

  await step("composer: /usage shows claude's dialog live in the thread, takes keys, and a prompt closes it first", async () => {
    await sendPrompt("/usage");
    const card = gui("#thread .dialog-card");
    await waitFor("the dialog in the thread", () => card.isVisible());
    check(/Current session/.test(await card.locator(".screen").textContent()), `the card shows ${await card.locator(".screen").textContent()}`);
    check((await card.locator(".screen .inverse", { hasText: "Usage" }).count()) === 1, "the shown tab isn't drawn in reverse video");
    await press(card.locator('[data-dialog-key="tab"]'));
    await waitFor("the Status tab", async () => /Version:/.test(await card.locator(".screen").textContent()));
    if (!touch) {
      await card.focus();
      await page.keyboard.press("Tab");
      await waitFor("Tab typed into the card to reach claude", async () => /Current session/.test(await card.locator(".screen").textContent()));
      await page.evaluate(() => rerender());
      check(await page.evaluate(() => !!document.activeElement?.closest(".dialog-card")), "drawing the thread again took the focus off claude's dialog");
      await page.focus("#prompt");
      await page.evaluate(() => rerender());
      check(await page.evaluate(() => document.activeElement?.id === "prompt"), "drawing the thread again moved the focus into claude's dialog");
    }
    await press(card.locator('[data-dialog-key="esc"]'));
    await waitFor("the dialog to close", async () => !(await page.locator("#thread .dialog-card").isVisible()));
    await sendPrompt("/usage");
    await waitFor("the dialog again", () => gui("#thread .dialog-card").isVisible());
    if (touch) await sendPrompt("after the dialog");
    else {
      await page.fill("#prompt", "after the dialog");
      await page.press("#prompt", "Enter");
      await page.press("#prompt", "Enter");
    }
    await waitFor("the prompt to reach claude", async () => /echo: after the dialog/.test(await termText()));
    await answered("echo: after the dialog");
    check(!(await page.locator("#thread .dialog-card").isVisible()), "the dialog stayed up after the prompt went out");
    await sendPrompt("and one more");
    await answered("echo: and one more");
    const sent = (await termText()).match(/echo: after the dialog/g).length;
    check(sent === 1, `Enter pressed twice while the dialog closed sent the prompt ${sent} times`);
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
    await waitFor("the plan banner", async () => (await page.locator("#approval").isVisible()) && (await text("#approval .kind")) === "Plan ready");
    const placeholder = await page.getAttribute("#prompt", "placeholder");
    check(placeholder === "Add feedback to refine the plan, or leave this blank to implement it", `placeholder: ${placeholder}`);
    await sendPrompt("also say goodbye");
    await waitTerm(/answered: also say goodbye/);
    await waitFor("the banner to go", async () => !(await page.locator("#approval").isVisible()));
    await sendPrompt("/plan-ready");
    await waitFor("the plan banner again", async () => (await page.locator("#approval").isVisible()) && (await text("#approval .kind")) === "Plan ready");
    await sendPrompt("");
    await waitTerm(/answered: Yes, and use auto mode/);
    await waitFor("the banner to go", async () => !(await page.locator("#approval").isVisible()));
    return placeholder;
  });

  await step("composer: Access and Build / Plan read claude's mode and set it with Shift+Tab", async () => {
    // A narrow composer, as on a phone, folds Access and Build / Plan into "…".
    const folded = await page.evaluate(() => composer.dataset.compact === "true");
    const openAccess = () => tapOrClick(folded ? "#more-button" : "#mode-button");
    const accessRows = () => page.evaluate(() => {
      const rows = [...composerMenu.children];
      const from = rows.findIndex((r) => r.textContent === "Access");
      return rows.slice(from + 1).filter((r) => r.tagName === "BUTTON").map((r) => `${r.querySelector(".label").textContent}${r.classList.contains("checked") ? "*" : ""}`);
    });
    const pickAccess = async (label) => {
      await openAccess();
      await press(page.locator("#composer-menu button", { hasText: label }).last());
    };
    const togglePlan = async (to) => {
      if (!folded) return tapOrClick("#plan-toggle");
      await tapOrClick("#more-button");
      await press(page.locator("#composer-menu button", { hasText: to }).last());
    };
    check(!(await page.locator("#mode-button").isVisible()), "a mode shows before claude drew its mode line");
    await sendPrompt("/modes");
    await waitFor("the mode control", async () => (await text("#mode-button")) === "Supervised" && (await text("#plan-toggle")) === "Build");
    await openAccess();
    const modes = await accessRows();
    check(modes.join("|") === "Supervised*|Auto-accept edits|Auto|Full access", `access modes: ${modes.join("|")}`);
    await press(page.locator("#composer-menu button", { hasText: "Auto-accept edits" }).last());
    await waitFor("Auto-accept edits", async () => (await text("#mode-button")) === "Auto-accept edits");
    await togglePlan("Plan");
    await waitFor("Plan", async () => (await text("#plan-toggle")) === "Plan" && (await page.getAttribute("#plan-toggle", "aria-pressed")) === "true");
    check(/plan mode on/.test(await termText()), "claude's screen never reached plan mode");
    check((await text("#mode-button")) === "Auto-accept edits", `in Plan, Access shows ${await text("#mode-button")}`);
    await pickAccess("Supervised");
    await waitFor("Supervised kept for Build", async () => (await text("#mode-button")) === "Supervised");
    check((await text("#plan-toggle")) === "Plan", "picking an access mode left Plan");
    await togglePlan("Chat");
    await waitFor("Build in Supervised", async () => (await text("#plan-toggle")) === "Build" && (await text("#mode-button")) === "Supervised");
    check(/manual mode on/.test((await termText()).split("\n").filter(Boolean).slice(-3).join("\n")), "Build didn't go back to claude's manual mode");
    if (!touch) {
      await page.focus("#prompt");
      await page.keyboard.press("Shift+Tab");
      await waitFor("Shift+Tab to cycle to Auto-accept edits", async () => (await text("#mode-button")) === "Auto-accept edits");
      await page.keyboard.press("Meta+Shift+A");
      await waitFor("⌘⇧A to open the access menu", () => page.locator("#composer-menu button.checked", { hasText: "Auto-accept edits" }).isVisible());
      await page.keyboard.press("Escape");
      await pickAccess("Supervised");
      await waitFor("Supervised again", async () => (await text("#mode-button")) === "Supervised");
    }
    return modes.join(" | ");
  });

  await step("composer: the model picker lists Claude's models, ⌘⇧M opens it", async () => {
    if (touch) await tapOrClick("#model-button");
    else await page.keyboard.press("Meta+Shift+M");
    await waitFor("the model picker", () => page.locator('#composer-menu[data-kind="models"] .model').first().isVisible());
    const names = await page.locator("#composer-menu .model .name").allTextContents();
    check(names.join("|") === "Claude Fable 5.1|Claude Opus 5.5|Claude Sonnet 5.5|Claude Haiku 5.5New|Legacy models", `models: ${names.join("|")}`);
    if (touch) await page.tap("#model-button");
    else await page.keyboard.press("Escape");
    await waitFor("the picker to close", async () => !(await page.locator("#composer-menu").isVisible()));
    return names.join(", ");
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
    if (touch) {
      await tapOutsideComposer();
      // A phone's composer doesn't rest on a scroll: unfocused, it is T3 Code's collapsed row wherever the thread is.
      await sleep(300);
      const up = await page.evaluate(() => composer.dataset.collapsed);
      await page.evaluate(() => (threadView.scrollTop = threadView.scrollHeight));
      await sleep(300);
      const end = await page.evaluate(() => composer.dataset.collapsed);
      check(up === "true" && end === "true", `a phone's unfocused composer collapsed ${up} scrolled up and ${end} at the end`);
      return "a phone collapses instead";
    }
    await waitFor("the resting composer", () => page.evaluate(() => composer.dataset.resting === "true"));
    const resting = (await rect("#composer .box")).height;
    check(Math.round(resting) === 48, `the resting composer is ${Math.round(resting)}px tall`);
    check(await page.locator("#composer-footer #model-button").isVisible(), "the model control didn't move into the strip");
    check(await page.locator("#composer-footer").isVisible(), "the strip went with the rest");
    await page.focus("#prompt");
    await waitFor("the full composer", () => page.evaluate(() => composer.dataset.resting === "false"));
    const full = (await rect("#composer .box")).height;
    check(full > 100, `the focused composer is ${Math.round(full)}px tall`);
    check(await page.locator("#composer .tools #model-button").isVisible(), "the model control didn't come back");
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

  await step("composer: the send shortcut setting: Enter, ⌘Enter for multiline prompts, ⌘Enter always", async () => {
    const pick = async (value) => {
      await page.evaluate(() => toggleSettings());
      await settingsNav("general");
      const options = await page.locator("#send-shortcut option").allTextContents();
      await page.selectOption("#send-shortcut", value);
      await page.evaluate(() => toggleSettings());
      return options;
    };
    const options = await pick("mod-enter");
    check(options.join("|") === "Enter|⌘ + Enter for multiline prompts|⌘ + Enter always", `choices: ${options.join("|")}`);
    check((await page.getAttribute("#send", "title")) === "Send (⌘Enter)", `the send button says ${await page.getAttribute("#send", "title")}`);
    if (!touch) {
      await page.fill("#prompt", "two");
      await page.press("#prompt", "Enter");
      await page.keyboard.type("lines");
      check((await page.inputValue("#prompt")) === "two\nlines", `Enter left ${JSON.stringify(await page.inputValue("#prompt"))}`);
      await page.press("#prompt", "Meta+Enter");
      await waitTerm(/echo: two/);
      check((await page.inputValue("#prompt")) === "", "⌘Enter didn't send");
      await pick("mod-enter-multiline");
      await page.fill("#prompt", "one line");
      await page.press("#prompt", "Enter");
      await waitTerm(/echo: one line/);
      await page.fill("#prompt", "a second\nline");
      // Until the last turn ends, the button offers to queue the prompt instead.
      await waitFor("a multiline prompt to ask for ⌘Enter", async () => (await page.getAttribute("#send", "title")) === "Send (⌘Enter)");
      await page.press("#prompt", "Enter");
      check((await page.inputValue("#prompt")) === "a second\nline\n", `Enter on a multiline prompt left ${JSON.stringify(await page.inputValue("#prompt"))}`);
      await page.press("#prompt", "Meta+Enter");
      await waitTerm(/echo: a second/);
    }
    await pick("enter");
    await page.fill("#prompt", "");
    check((await page.getAttribute("#send", "title")) === "Send (Enter)", "the setting didn't switch back");
    return options.join(" | ");
  });

  await step("composer: attached files show above the prompt and go out with it", async () => {
    const attach = async (file) => {
      await openComposer();
      const [chooser] = await Promise.all([page.waitForEvent("filechooser"), tapOrClick("#composer-attach")]);
      await chooser.setFiles(file);
    };
    const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==", "base64");
    await attach({ name: "shot.png", mimeType: "image/png", buffer: png });
    await waitFor("the thumbnail", () => page.locator("#attachments .tile img").isVisible());
    const tile = await rect("#attachments .tile");
    check(Math.round(tile.width) === 64 && Math.round(tile.height) === 64, `the tile is ${Math.round(tile.width)}x${Math.round(tile.height)}`);
    check((await page.getAttribute("#attachments .tile img", "alt")) === "shot.png", "the tile isn't named shot.png");
    check((await page.inputValue("#prompt")) === "", "the file's path went into the text");
    await press(page.locator("#attachments .zoom"));
    await waitFor("the picture full size", () => page.locator("#image-zoom").isVisible());
    // A 1px picture leaves its caption at the dialog's center, which a tap there would hit.
    await press(page.locator("#image-zoom .media-close"));
    await waitFor("the picture to close", async () => !(await page.locator("#image-zoom").isVisible()));
    await sendPrompt("what is in this picture");
    await waitTerm(/echo: what is in this picture \S*shot\.png/);
    await waitFor("the strip to go", async () => !(await page.locator("#attachments").isVisible()));
    await attach({ name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from("hi there") });
    await waitFor("the file row", () => page.locator("#attachments .file").isVisible());
    const row = `${await text("#attachments .file .name")} ${await text("#attachments .file .size")}`;
    check(row === "notes.txt 8 B", `the row reads ${row}`);
    check(await page.locator("#attachments .file svg.ftype use[href='#file-tree-builtin-text']").count() === 1, "the row has no text-file icon");
    await press(page.locator("#attachments .file .name"));
    await waitFor("the preview", async () => (await text("#file-preview pre")) === "hi there");
    await press(page.locator("#file-preview [data-close]"));
    await openComposer();
    await press(page.locator("#attachments .remove"));
    check(!(await page.locator("#attachments").isVisible()), "the row's X left it there");
    await page.evaluate((data) => {
      const bytes = Uint8Array.from(atob(data), (c) => c.charCodeAt(0));
      const transfer = new DataTransfer();
      transfer.items.add(new File([bytes], "pasted.png", { type: "image/png" }));
      const paste = new Event("paste", { bubbles: true, cancelable: true });
      Object.defineProperty(paste, "clipboardData", { value: transfer });
      prompt.focus();
      prompt.dispatchEvent(paste);
    }, png.toString("base64"));
    await waitFor("the pasted picture in the strip", async () => (await page.getAttribute("#attachments .tile img", "alt").catch(() => null)) === "pasted.png");
    await openComposer();
    await press(page.locator("#attachments .remove"));
    await attach({ name: "huge.bin", mimeType: "application/octet-stream", buffer: Buffer.alloc(26 << 20) });
    await waitFor("the upload error", async () => /over 25 MB/.test(await text("#composer-note")));
    check(await page.locator("#composer-note").isVisible(), "the upload error isn't in the app view");
    return `${row}; ${await text("#composer-note")}`;
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

  await step("composer: the branch picker switches branches, ⌘⇧G, and the Git action reads the branch", async () => {
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
    if (touch) {
      await tapOrClick("#head-more");
      const options = await page.locator("#panel-menu button .label").allTextContents();
      check(options.join("|").includes("Push & create PR|Commit|Push|Create PR"), `Options menu: ${options.join("|")}`);
      await page.screenshot({ path: path.join(shots, "options-menu.png") });
      await tapOrClick("#head-more");
      await tapOrClick("#toggle-details");
    }
    await tapOrClick(touch ? '#details-card [data-act="git-menu"]' : "#git-action .more");
    const actions = await page.locator("#header-menu .label").allTextContents();
    check(actions.join("|") === "Commit|Push|Create PR", `Git menu: ${actions.join("|")}`);
    await tapOrClick(touch ? "#toggle-details" : "#git-action .more");
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

  await step("composer: the screen reader finds claude's effort marks and its /effort and /model menus", async () => {
    const screen = (name) => fs.readFileSync(path.join(__dirname, "screens", name), "utf8").split("\n");
    const marks = await page.evaluate((screens) => screens.map(traitsOn), ["effort-ultracode", "fast-on", "manual-mode"].map((s) => screen(`claude-2.1.310-${s}.txt`)));
    check(JSON.stringify(marks) === JSON.stringify([{ effort: "max", ultracode: true, fast: false }, { effort: "medium", ultracode: false, fast: true }, null]), `marks: ${JSON.stringify(marks)}`);
    const slider = await page.evaluate((lines) => effortMenuOn(lines), screen("claude-2.1.310-effort-menu.txt"));
    check(JSON.stringify(slider) === JSON.stringify({ levels: ["low", "medium", "high", "xhigh", "max"], at: 1, ultracode: false }), `slider: ${JSON.stringify(slider)}`);
    const tabs = await page.evaluate((screens) => screens.map((lines) => lines.map(questionTabsOf).find(Boolean)), ["first", "multi", "review"].map((s) => screen(`claude-2.1.310-questions-${s}.txt`)));
    check(JSON.stringify(tabs) === JSON.stringify([{ headers: ["Color", "Sizes"], answered: [false, false] }, { headers: ["Color", "Sizes"], answered: [true, false] }, { headers: ["Color", "Sizes"], answered: [true, true] }]), `question tabs: ${JSON.stringify(tabs)}`);
    const multi = await page.evaluate((lines) => menuOn(lines), screen("claude-2.1.310-questions-multi.txt"));
    check(multi?.options.map((o) => o.head).join("|") === "[ ] Small (tiny)|[ ] Large (huge)|[ ] Type something|Chat about this", `multi-select options: ${multi?.options.map((o) => o.head).join("|")}`);
    const models = await page.evaluate((lines) => modelMenuOn(lines), screen("claude-2.1.310-model-menu.txt"));
    const rows = models?.options.map((o) => `${o.n}${o.chosen ? "*" : ""} ${o.head.split(/\s/)[0]}`).join(", ");
    check(rows === "1* Default, 2 Sonnet, 3 Fable, 4 Opus, 5 Haiku", `/model rows: ${rows}`);
    return rows;
  });

  await step("composer: the model picker searches, stars favorites, and picks a model for this session", async () => {
    // These steps print a lot, so they run in a thread of their own and leave alpha's screen as it was.
    const rows = await page.locator("#thread-list .thread-row").count();
    const before = await page.evaluate(() => active?.key);
    await page.evaluate(() => newThread(true));
    await waitFor("a thread for the composer's steps", async () => (await page.locator("#thread-list .thread-row").count()) === rows + 1 && (await page.evaluate((key) => active?.key !== key, before)), 10000);
    await waitTerm(/fake claude in .*work\/alpha/);
    await page.evaluate(() => (window.composerThread = active.key));
    await sendPrompt("/modes");
    await waitFor("the mode control", async () => (await text("#mode-button")) === "Supervised");
    await tapOrClick("#model-button");
    await waitFor("the model picker", () => page.locator('#composer-menu[data-kind="models"] .model').first().isVisible());
    check((await text('#composer-menu .model[data-row="claude-haiku-5-5"] .new')) === "New", "Haiku 5.5 has no NEW badge");
    check((await text('#composer-menu .model[data-row="legacy"] .sub')) === "9 models", `legacy: ${await text('#composer-menu .model[data-row="legacy"] .sub')}`);
    await page.fill("#composer-menu .search", "opus");
    const found = await page.locator("#composer-menu .model .name").allTextContents();
    check(found.join("|") === "Claude Opus 5.5|Claude Opus 5|Claude Opus 4.8|Claude Opus 4.7|Claude Opus 4.6|Claude Opus 4.5", `search: ${found.join("|")}`);
    await press(page.locator('#composer-menu [data-fav="claude-opus-5-5"]'));
    await page.fill("#composer-menu .search", "");
    await press(page.locator('#composer-menu [data-provider="favorites"]'));
    const favorites = await page.locator("#composer-menu .model .name").allTextContents();
    check(favorites.join("|") === "Claude Opus 5.5", `favorites: ${favorites.join("|")}`);
    await press(page.locator('#composer-menu [data-provider="claude"]'));
    check((await page.locator("#composer-menu .model .name").first().textContent()) === "Claude Opus 5.5", "the favorite isn't first");
    await press(page.locator('#composer-menu .model[data-row="claude-haiku-5-5"]'));
    await waitTerm(/model Haiku 5\.5 for this session/);
    await waitFor("Haiku 5.5 on the button", async () => (await text("#model-button")) === "Haiku 5.5");
    await tapOrClick("#model-button");
    check(await page.locator('#composer-menu .model[data-row="claude-haiku-5-5"] .check').isVisible(), "Haiku 5.5 isn't checked");
    await press(page.locator('#composer-menu [data-fav="claude-opus-5-5"]'));
    if (touch) await page.tap("#model-button");
    else await page.keyboard.press("Escape");
    return found.join(", ");
  });

  await step("composer: Traits sets claude's effort for this session, Ultrathink goes in the prompt, ⌘⇧E", async () => {
    const folded = await page.evaluate(() => composer.dataset.compact === "true");
    const openTraits = () => tapOrClick(folded ? "#more-button" : "#traits-button");
    await sendPrompt("/traits");
    await waitFor("the traits control", async () => (await text("#traits-button")) === "Medium");
    await tapOrClick("#model-button");
    await press(page.locator('#composer-menu .model[data-row="claude-opus-5-5"]'));
    await waitFor("Opus 5.5, whose traits have Fast Mode", async () => (await text("#model-button")) === "Opus 5.5");
    await openTraits();
    const shown = await page.evaluate(() => {
      const rows = [...composerMenu.children];
      const end = rows.findIndex((r) => r.textContent === "Mode");
      return rows.slice(0, end < 0 ? rows.length : end).filter((r) => !r.matches(".rule")).map((r) => `${r.querySelector(".label")?.textContent ?? r.textContent}${r.classList.contains("checked") ? "*" : ""}`);
    });
    check(shown.join("|") === "Reasoning|Low|MediumDefault*|High|Extra High|Max|Ultracode|Ultrathink|Fast Mode|On|Off*", `traits menu: ${shown.join("|")}`);
    await press(page.locator("#composer-menu button", { hasText: /^\s*High\s*$/ }).first());
    await waitTerm(/effort high for this session/);
    await waitFor("High", async () => (await text("#traits-button")) === "High");
    if (touch) await openTraits();
    else await page.keyboard.press("Meta+Shift+E");
    await press(page.locator("#composer-menu button", { hasText: /^\s*Ultracode/ }).first());
    await waitTerm(/effort xhigh ultracode for this session/);
    await waitFor("Ultracode", async () => (await text("#traits-button")) === "Ultracode");
    await page.fill("#prompt", "think hard about it");
    await openTraits();
    await press(page.locator("#composer-menu button", { hasText: /^\s*Ultrathink/ }).first());
    check((await page.inputValue("#prompt")) === "Ultrathink:\nthink hard about it", `Ultrathink left the prompt ${JSON.stringify(await page.inputValue("#prompt"))}`);
    await waitFor("Ultrathink", async () => (await text("#traits-button")) === "Ultrathink");
    await openTraits();
    await press(page.locator("#composer-menu button", { hasText: /^\s*Medium/ }).first());
    await waitTerm(/effort medium for this session/);
    check((await page.inputValue("#prompt")) === "think hard about it", "picking Medium kept the Ultrathink prefix");
    await page.fill("#prompt", "");
    await openTraits();
    await press(page.locator("#composer-menu button", { hasText: /^\s*On\s*$/ }).first());
    await waitFor("Medium Fast", async () => (await text("#traits-button")) === "Medium Fast");
    await openTraits();
    await press(page.locator("#composer-menu button", { hasText: /^\s*Off\s*$/ }).first());
    await waitFor("Medium", async () => (await text("#traits-button")) === "Medium");
    return shown.join(" | ");
  });

  await step("composer: a legacy model and Full access restart claude on the conversation", async () => {
    const rows = await page.locator("#thread-list .thread-row").count();
    const before = await page.evaluate(() => active?.key);
    await page.evaluate(() => newThread(true));
    await waitFor("a new thread", async () => (await page.locator("#thread-list .thread-row").count()) === rows + 1 && (await page.evaluate((key) => active?.key !== key, before)), 10000);
    await waitTerm(/fake claude in .*work\/alpha/);
    await sendPrompt("/modes");
    await waitFor("the mode control", async () => (await text("#mode-button")) === "Supervised");
    await tapOrClick("#model-button");
    await press(page.locator('#composer-menu .model[data-row="legacy"]'));
    await press(page.locator('#composer-menu .model[data-row="claude-opus-4-8"]'));
    await waitTerm(/\(model claude-opus-4-8\)/);
    await waitFor("Opus 4.8 on the button", async () => (await text("#model-button")) === "Opus 4.8");
    await sendPrompt("/modes");
    await waitFor("the mode control", async () => (await text("#mode-button")) === "Supervised");
    await tapOrClick((await page.evaluate(() => composer.dataset.compact === "true")) ? "#more-button" : "#mode-button");
    await press(page.locator("#composer-menu button", { hasText: "Full access" }).first());
    await waitTerm(/\(bypass\) \(model claude-opus-4-8\)/);
    check(await page.evaluate(() => active.view.bypass), "the session doesn't skip permissions");
    await page.evaluate(() => closeTab(active));
    await waitFor("the thread to close", async () => (await page.locator("#thread-list .thread-row").count()) === rows);
    await page.evaluate(() => activate(tabs.find((t) => t.key === window.composerThread)));
    await waitFor("alpha", async () => /alpha/.test(await text("#chat-head .crumb")));
  });

  await step("composer: a narrow composer folds Traits, Mode and Access into …", async () => {
    if (!touch) await page.evaluate(() => document.documentElement.style.setProperty("--lane", "560px"));
    try {
      await waitFor("the … button", () => page.locator("#more-button").isVisible());
      check(!(await page.locator("#traits-button").isVisible()) && !(await page.locator("#mode-button").isVisible()), "Traits and Access stay out with …");
      await tapOrClick("#more-button");
      const headings = await page.locator("#composer-menu .heading").allTextContents();
      check(headings.join("|") === "Reasoning|Fast Mode|Mode|Access", `… menu for Opus 5.5: ${headings.join("|")}`);
      await press(page.locator("#composer-menu button", { hasText: "Plan" }).last());
      await waitFor("plan mode", async () => /plan mode on/.test((await termText()).split("\n").filter(Boolean).slice(-3).join("\n")));
      await tapOrClick("#more-button");
      await press(page.locator("#composer-menu button", { hasText: "Chat" }));
      await waitFor("manual mode", async () => /manual mode on/.test((await termText()).split("\n").filter(Boolean).slice(-3).join("\n")));
      return headings.join(" | ");
    } finally {
      await page.evaluate(() => document.documentElement.style.removeProperty("--lane"));
    }
  });

  await step("composer: the context ring shows how full the window is, and a cold long session compacts first", async () => {
    await sendPrompt("full");
    await openComposer();
    await waitFor("the context ring", () => page.locator("#context-meter").isVisible());
    check((await page.getAttribute("#context-meter", "aria-label")) === "Context window 23% used", `ring: ${await page.getAttribute("#context-meter", "aria-label")}`);
    await tapOrClick("#context-meter");
    await waitFor("the context card", () => page.locator("#context-card").isVisible());
    const card = (await text("#context-card .head")).replace(/\s+/g, " ");
    check(card === "Context Window23%·231k/1m", `card: ${card}`);
    check(await page.locator("#context-card [data-compact]").isVisible(), "no Compact context button");
    // The phone's composer collapses under the open card and takes the ring with it, so a tap outside closes the card there.
    if (touch) await tapOutsideComposer();
    else await tapOrClick("#context-meter");
    await waitFor("the card to close", () => page.locator("#context-card").isHidden());
    await openComposer();
    await waitFor("the compact pill", async () => (await text("#compact-pill")) === "Compact231k");
    check((await page.getAttribute("#send", "title")) === "Summarize 231k tokens of history, then send", `send says ${await page.getAttribute("#send", "title")}`);
    await tapOrClick("#compact-pill");
    check((await text("#compact-pill")) === "Full231k", "the pill didn't switch to keeping the history");
    await openComposer();
    await tapOrClick("#compact-pill");
    await sendPrompt("after the break");
    await waitTerm(/\/compact[\s\S]*echo: after the break/);
    await waitFor("the compaction divider", async () => (await page.locator("#thread .compaction").count()) > 0, 10000);
    await waitFor("the pill to go", async () => !(await page.locator("#compact-pill").isVisible()));
    return card;
  });

  await step("composer: @ files become chips with their icons, $ lists skills, # lists pull requests, ⌘⌥Enter", async () => {
    fs.mkdirSync(path.join(HOME, "work/alpha/docs"), { recursive: true });
    fs.writeFileSync(path.join(HOME, "work/alpha/docs/notes-guide.md"), "guide");
    const skill = path.join(HOME, ".claude/skills/tidy-up/SKILL.md");
    fs.mkdirSync(path.dirname(skill), { recursive: true });
    fs.writeFileSync(skill, "---\nname: tidy-up\ndescription: Tidy the code\n---\n");
    const pick = async () => (touch ? page.tap("#composer-menu [data-pick]") : page.press("#prompt", "Enter"));
    try {
      await page.fill("#prompt", "look at @notes-gu");
      await waitFor("the @ menu", () => page.locator('#composer-menu[data-kind="mention"] [data-pick="@docs/notes-guide.md"]').isVisible());
      check(await page.locator('#composer-menu [data-pick="@docs/notes-guide.md"] use[href="#file-tree-builtin-markdown"]').count() === 1, "the menu has no Markdown icon");
      await pick();
      check((await page.inputValue("#prompt")) === "look at @notes-guide.md ", `the pick left ${JSON.stringify(await page.inputValue("#prompt"))}`);
      await waitFor("the chip", () => page.locator("#prompt-mirror .chip.mention").isVisible());
      check((await text("#prompt-mirror .chip.mention")) === "@notes-guide.md", `the chip reads ${await text("#prompt-mirror .chip.mention")}`);
      check(await page.locator("#prompt-mirror .chip.mention use[href='#file-tree-builtin-markdown']").count() === 1, "the chip has no Markdown icon");
      const [chip, typed] = await page.evaluate(() => [document.querySelector("#prompt-mirror .chip").getBoundingClientRect().width, getComputedStyle(prompt).color]);
      check(chip > 40 && typed === "rgba(0, 0, 0, 0)", `chip ${Math.round(chip)}px, prompt text ${typed}`);
      if (!touch) {
        await page.focus("#prompt");
        await page.keyboard.press("End");
        await page.keyboard.press("Backspace");
        await page.keyboard.press("Backspace");
        check((await page.inputValue("#prompt")) === "look at ", `Backspace left ${JSON.stringify(await page.inputValue("#prompt"))}`);
        await page.keyboard.type("@notes-gu");
        await waitFor("the @ menu again", () => page.locator('#composer-menu [data-pick="@docs/notes-guide.md"]').isVisible());
        await pick();
      }
      await page.focus("#prompt");
      await page.keyboard.press("End");
      await page.keyboard.type("please");
      await sendPrompt(await page.inputValue("#prompt"));
      await waitTerm(/echo: look at @docs\/notes-guide\.md please/);
      await page.fill("#prompt", "$");
      await waitFor("the $ menu", () => page.locator('#composer-menu[data-kind="skill"] [data-pick="$tidy-up"]').isVisible());
      await page.fill("#prompt", "$tid");
      await waitFor("the $ menu", () => page.locator('#composer-menu[data-kind="skill"] [data-pick="$tidy-up"]').isVisible());
      await pick();
      check((await text("#prompt-mirror .chip.skill")) === "$tidy-up", "the skill isn't a chip");
      await sendPrompt(await page.inputValue("#prompt"));
      await waitTerm(/echo: \/tidy-up/);
      await page.fill("#prompt", "see #");
      await waitFor("the # menu", () => page.locator('#composer-menu[data-kind="pr"] [data-pick="#42"]').isVisible());
      const prs = await page.locator("#composer-menu [data-pick]").evaluateAll((els) => els.map((e) => e.textContent));
      check(prs.join("|") === "#42Add notes|#41Draft the notes", `pull requests: ${prs.join("|")}`);
      await pick();
      check((await text("#prompt-mirror .chip.pr")) === "#42", "the pull request isn't a chip");
      await sendPrompt(await page.inputValue("#prompt"));
      await waitTerm(/echo: see https:\/\/github\.com\/example\/alpha\/pull\/42/);
      if (touch) return prs.join(", ");
      const rows = await page.locator("#thread-list .thread-row").count();
      await page.fill("#prompt", "spin off");
      await page.press("#prompt", "Meta+Alt+Enter");
      await waitFor("a new thread after ⌘⌥Enter", async () => (await page.locator("#thread-list .thread-row").count()) === rows + 1 && (await page.evaluate(() => active?.key !== window.composerThread)), 10000);
      await waitTerm(/fake claude in .*work\/alpha/);
      await page.evaluate(() => closeTab(active));
      await waitFor("the thread to close", async () => (await page.locator("#thread-list .thread-row").count()) === rows);
      await page.evaluate(() => activate(tabs.find((t) => t.key === window.composerThread)));
      await waitTerm(/echo: spin off/);
      return prs.join(", ");
    } finally {
      fs.rmSync(path.join(HOME, "work/alpha/docs"), { recursive: true, force: true });
      fs.rmSync(path.join(HOME, ".claude/skills"), { recursive: true, force: true });
    }
  });

  await step("composer: claude's questions: header, 1/2, checks, Previous, Next question and Submit answers", async () => {
    const pill = () => text("#pill-actions .pill");
    // A narrow composer, as on a phone, shortens the pill as T3 Code's does.
    const folded = await page.evaluate(() => composer.dataset.compact === "true");
    await sendPrompt("/questions");
    await waitFor("the first question", async () => (await text("#approval .q-header")) === "Color" && (await text("#approval .q-count")) === "1/2");
    check((await pill()) === (folded ? "Next" : "Next question") && !(await page.locator('#pill-actions [data-q="previous"]').isVisible()), `pills: ${await text("#pill-actions")}`);
    check(!(await page.locator("#send").isVisible()), "Send shows beside the question's pill");
    await press(page.locator("#approval .choice", { hasText: "Red" }));
    check(await page.locator('#approval .choice[data-picked] .tick').isVisible(), "the picked option shows no check");
    await waitFor("the second question", async () => (await text("#approval .q-header")) === "Sizes" && (await text("#approval .q-count")) === "2/2");
    check((await text("#approval .q-multi")) === "Select one or more options.", "the multi-select question doesn't say so");
    check((await pill()) === (folded ? "Submit" : "Submit answers"), `the last question's pill reads ${await pill()}`);
    await press(page.locator("#approval .choice", { hasText: "Small" }));
    await waitFor("Small checked", () => page.locator("#approval .choice[data-picked]", { hasText: "Small" }).isVisible());
    await press(page.locator("#approval .choice", { hasText: "Large" }));
    await waitFor("Large checked", () => page.locator("#approval .choice[data-picked]", { hasText: "Large" }).isVisible());
    await press(page.locator('#pill-actions [data-q="previous"]'));
    await waitFor("back on the first question", async () => (await text("#approval .q-header")) === "Color");
    check(await page.locator("#approval .choice[data-picked]", { hasText: "Red" }).isVisible(), "the answered option has no check");
    await press(page.locator("#pill-actions .pill"));
    await waitFor("on to the second question", async () => (await text("#approval .q-header")) === "Sizes");
    await press(page.locator("#approval .q-head"));
    await waitFor("the folded question", async () => (await text("#approval .q-text")) === "Which sizes?" && !(await page.locator("#approval .q-body").isVisible()));
    await press(page.locator("#approval .q-header"));
    await waitFor("the question open again", () => page.locator("#approval .q-body").isVisible());
    await press(page.locator("#pill-actions .pill"));
    await waitTerm(/answered: Which color\? → Red · Which sizes\? → Small, Large/);
    await waitFor("the banner to go", async () => !(await page.locator("#approval").isVisible()));
    check(await page.locator("#send").isVisible(), "Send didn't come back");
  });

  await step("composer: claude's plan offers Refine, Implement, and Implement in a new thread", async () => {
    await sendPrompt("/plan-ready");
    await waitFor("the plan banner", async () => (await page.locator("#approval").isVisible()) && (await text("#approval .kind")) === "Plan ready");
    check((await text("#pill-actions .pill")) === "Implement", `the plan's pill reads ${await text("#pill-actions .pill")}`);
    await page.fill("#prompt", "shorter please");
    await waitFor("Refine", async () => (await text("#pill-actions .pill")) === "Refine");
    await page.fill("#prompt", "");
    await waitFor("Implement", async () => (await text("#pill-actions .pill")) === "Implement");
    const rows = await page.locator("#thread-list .thread-row").count();
    await press(page.locator('#pill-actions [data-plan="more"]'));
    const before = await page.evaluate(() => active?.key);
    await press(page.locator("#composer-menu button", { hasText: "Implement in a new thread" }));
    await waitFor("the new thread", async () => (await page.locator("#thread-list .thread-row").count()) === rows + 1 && (await page.evaluate((key) => active?.key !== key, before)), 10000);
    await waitTerm(/PLEASE IMPLEMENT THIS PLAN:/, 15000);
    check(/1\. Add a hello comment to a\.py\./.test(await termText()), "the plan didn't go to the new thread");
    await showSidebar();
    check(await page.locator("#thread-list .thread-row.active", { hasText: "Implement plan" }).isVisible(), "the new thread isn't titled Implement plan");
    await page.evaluate(() => closeTab(active));
    await waitFor("the thread to close", async () => (await page.locator("#thread-list .thread-row").count()) === rows);
    await page.evaluate(() => activate(tabs.find((t) => t.key === window.composerThread)));
    await waitTerm(/answered: esc/);
    await page.evaluate(() => closeTab(active));
    await waitFor("the composer's thread to close", async () => (await page.locator("#thread-list .thread-row").count()) === rows - 1);
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
    await waitFor("alpha", async () => /alpha/.test(await text("#chat-head .crumb")));
  });

  await step("composer: an empty thread asks what to build in its project, which the headline switches", async () => {
    const rows = await page.locator("#thread-list .thread-row").count();
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/beta"))));
    const before = await page.evaluate(() => active?.key);
    await page.evaluate(() => newThread(true));
    await waitFor("an empty thread in beta", async () => (await page.locator("#thread-list .thread-row").count()) === rows + 1 && (await page.evaluate((key) => active?.key !== key, before)), 10000);
    await waitFor("the headline", () => page.locator("#hero-head h1").isVisible());
    const said = (await page.locator("#hero-head h1").innerText()).replace(/\s+/g, " ").trim();
    check(/^What should we build (in )?beta ?\?$/.test(said) || said === "What should we build? beta", `the headline reads ${said}`);
    await press(page.locator("[data-hero-project]"));
    await waitFor("the projects", () => page.locator('#composer-menu[data-kind="projects"] [data-path$="/alpha"]').isVisible());
    check((await page.getAttribute('#composer-menu[data-kind="projects"] .search', "placeholder")) === "Search projects...", "the project search isn't T3's");
    check(await page.locator("#composer-menu [data-add]").isVisible(), "no Add project");
    await page.fill('#composer-menu[data-kind="projects"] .search', "zzz");
    check((await text("#composer-menu .none")) === "No matching projects.", "no empty state");
    await page.fill('#composer-menu[data-kind="projects"] .search', "alp");
    await press(page.locator('#composer-menu [data-path$="/alpha"]'));
    await waitTerm(/fake claude in .*work\/alpha/);
    await waitFor("alpha's thread in place of beta's empty one", async () => (await page.locator("#thread-list .thread-row").count()) === rows + 1 && /alpha/.test(await text("#chat-head .crumb")));
    await page.evaluate(() => closeTab(active));
    await waitFor("the thread to close", async () => (await page.locator("#thread-list .thread-row").count()) === rows);
    return said;
  });

  await step("composer: before the first prompt the strip moves the thread to a new worktree and back, ⌘⇧X, ⌘⇧L", async () => {
    const beta = path.join(HOME, "work/beta");
    const git = (...args) => require("child_process").execFileSync("git", ["-C", beta, ...args]).toString().trim();
    const rows = await page.locator("#thread-list .thread-row").count();
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/beta"))));
    const before = await page.evaluate(() => active?.key);
    await page.evaluate(() => newThread(true));
    await waitFor("a new thread in beta", async () => (await page.locator("#thread-list .thread-row").count()) === rows + 1 && (await page.evaluate((key) => active?.key !== key, before)), 10000);
    await waitFor("the open workspace menu", () => page.locator("#composer-footer button.where").isVisible());
    check((await text("#composer-footer .where .label")) === "sweep-node", `the strip names ${await text("#composer-footer .where .label")}`);
    await tapOrClick("#composer-footer button.where");
    const menu = await page.locator('#composer-menu[data-kind="workspace"] :is(.heading, .label)').allTextContents();
    check(menu.join("|") === "Run on|sweep-node|Workspace|Current checkout|New worktree", `workspace menu: ${menu.join("|")}`);
    await press(page.locator("#composer-menu button", { hasText: "New worktree" }));
    await waitTerm(/fake claude in .*work\/beta\/\.claude\/worktrees\/pinned-/, 15000);
    await waitFor("New worktree in the strip", async () => (await page.getAttribute("#composer-footer .where .ws-icon", "title")) === "New worktree");
    const tree = await page.evaluate(() => active.view.cwd);
    check(git("worktree", "list").includes(tree.split("/").pop()), "git has no such worktree");
    check(!git("status", "--porcelain").includes(".claude"), "the worktree shows in beta's status");
    await tapOrClick("#composer-footer .branch");
    await waitFor("the ref picker", () => page.locator("#composer-menu .from-remote").isVisible());
    check((await page.getAttribute("#composer-menu .search", "placeholder")) === "Search refs...", "the search isn't T3's");
    await page.fill("#composer-menu .search", "#42");
    check((await text("#composer-menu [data-pr-checkout]")) === "Checkout PR #42", "no Checkout PR row");
    await page.fill("#composer-menu .search", "brand-new-ref");
    check((await text("#composer-menu [data-create]")) === 'Create new ref "brand-new-ref"', "the create row isn't T3's");
    if (touch) await page.tap("#composer-footer .branch");
    else await page.keyboard.press("Escape");
    if (touch) await tapOrClick("#composer-footer button.where");
    else await page.keyboard.press("Meta+Shift+X");
    await press(page.locator("#composer-menu button", { hasText: "Current checkout" }));
    await waitFor("back in beta's checkout", async () => (await page.evaluate(() => active.view.cwd)).endsWith("/work/beta"), 15000);
    await waitFor("the unused worktree to go", async () => !fs.existsSync(tree));
    await tapOrClick("#composer-footer button.where");
    await press(page.locator("#composer-menu button", { hasText: "New worktree" }));
    await waitFor("a worktree again", async () => /\.claude\/worktrees\/pinned-/.test(await page.evaluate(() => active.view.cwd)), 15000);
    const kept = await page.evaluate(() => active.view.cwd);
    await waitFor("claude in the second worktree", async () => (await termText()).includes(`${kept.split("/").pop()}\n`), 15000);
    await sendPrompt("work here");
    await waitTerm(/echo: work here/);
    await waitFor("the strip locks", async () => !(await page.locator("#composer-footer button.where").isVisible()) && (await page.getAttribute("#composer-footer .where .ws-icon", "title")) === "Worktree");
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/beta"))));
    await page.evaluate(() => newThread(true));
    await waitFor("another thread in beta", async () => (await page.locator("#thread-list .thread-row").count()) === rows + 2, 10000);
    await waitFor("the open workspace menu", () => page.locator("#composer-footer button.where").isVisible());
    await tapOrClick("#composer-footer button.where");
    check(await page.locator("#composer-menu button", { hasText: "Previous worktree (pinned/" }).isVisible(), "no Previous worktree");
    if (touch) await press(page.locator("#composer-menu button", { hasText: "Previous worktree" }));
    else {
      await page.keyboard.press("Escape");
      await page.keyboard.press("Meta+Shift+L");
    }
    await waitFor("the previous worktree", async () => (await page.evaluate(() => active.view.cwd)) === kept, 15000);
    for (let i = 0; i < 2; i++) await page.evaluate(() => closeTab(tabs.findLast((t) => t.view.cwd.includes("/.claude/worktrees/"))));
    await waitFor("the threads to close", async () => (await page.locator("#thread-list .thread-row").count()) === rows);
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
    return menu.join(" | ");
  });

  await step("composer: Send says Connecting or Environment disconnected, and the placeholder is T3's", async () => {
    check((await page.getAttribute("#prompt", "placeholder")) === "Ask anything, @tag files/folders, $use skills, or / for commands", `placeholder: ${await page.getAttribute("#prompt", "placeholder")}`);
    await page.fill("#prompt", "while offline");
    try {
      await page.evaluate(() => {
        document.body.dataset.hub = "offline";
        renderComposer(active);
      });
      check((await page.getAttribute("#send", "title")) === "Connecting" && (await page.isDisabled("#send")), "Send doesn't wait for the hub");
      check(await page.evaluate(() => composer.dataset.spinning === "true"), "no spinner while connecting");
      await page.evaluate(() => {
        document.body.dataset.hub = "online";
        machines.get(active.machine).view.online = false;
        renderComposer(active);
      });
      check((await page.getAttribute("#send", "title")) === "Environment disconnected", `send says ${await page.getAttribute("#send", "title")}`);
      check((await page.getAttribute("#prompt", "placeholder")) === "Ask for changes, send follow-ups, or attach images", "the offline placeholder isn't T3's");
    } finally {
      await page.evaluate(() => {
        document.body.dataset.hub = "online";
        machines.get(active.machine).view.online = true;
        renderComposer(active);
      });
      await page.fill("#prompt", "");
    }
    check(!(await page.isDisabled("#send")), "Send stayed off");
  });

  await step("composer: the header title shows a chevron and opens the thread menu under it", async () => {
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
    await waitFor("alpha", async () => /alpha/.test(await text("#chat-head .crumb")));
    if (!touch) {
      await page.hover("#chat-head .crumb .title");
      await waitFor("the chevron", async () => (await page.locator("#chat-head .crumb .title .chev").evaluate((el) => getComputedStyle(el).opacity)) === "1");
    }
    await press(page.locator("#chat-head .crumb .title"));
    await waitFor("the thread menu", () => page.locator("#side-menu").isVisible(), 2000);
    const [menu, title] = [await rect("#side-menu"), await rect("#chat-head .crumb .title")];
    check(Math.round(menu.left) === Math.round(Math.max(8, title.left)) && Math.round(menu.top) === Math.round(title.bottom + 4), `the menu opened at ${menu.left},${menu.top}, under a title at ${title.left},${title.bottom}`);
    const items = await page.locator("#side-menu button").allTextContents();
    check(items.some((i) => /Rename/.test(i)), `menu: ${items.join(" | ")}`);
    await page.keyboard.press("Escape");
    if (touch) await page.tap("#thread");
    await waitFor("the menu to close", async () => !(await page.locator("#side-menu").isVisible()));
    if (!touch) {
      await page.dblclick("#chat-head .crumb .title");
      await waitFor("the rename field", () => page.locator("#chat-head .crumb input.rename").isVisible());
      await sleep(700);
      check(!(await page.locator("#side-menu").isVisible()), "a double-click opened the menu too");
      await page.keyboard.press("Escape");
    }
    return items.slice(0, 4).join(" | ");
  });

  await step("composer: project actions: import from t3.json, run in a drawer terminal, add with a shortcut, delete", async () => {
    // A login zsh in a home without a .zshrc stops to offer its setup wizard.
    if (!fs.existsSync(path.join(HOME, ".zshrc"))) fs.writeFileSync(path.join(HOME, ".zshrc"), "");
    const t3 = path.join(HOME, "work/alpha/t3.json");
    fs.writeFileSync(t3, JSON.stringify({ scripts: [{ name: "Lint", command: "echo lint-$((40+2))", icon: "lint" }] }));
    // A phone reaches the actions through the thread details card, as T3 Code's.
    const details = async () => touch && !(await page.locator("#details-card").isVisible()) && (await tapOrClick("#toggle-details"));
    const openMenu = async () => {
      await details();
      await tapOrClick(touch ? '#details-card [data-act="scripts"]' : (await page.locator("#scripts-control .pick").isVisible()) ? "#scripts-control .pick" : "#scripts-control button");
    };
    try {
      await page.evaluate(() => scriptsOf.clear());
      await page.evaluate(() => renderComposer(active));
      await waitFor("the Actions button", async () => (await text("#scripts-control")) === "Actions");
      await openMenu();
      const menu = await page.locator("#header-menu :is(.heading, .label)").allTextContents();
      check(menu.join("|") === "From t3.json|Lint|Add action", `scripts menu: ${menu.join("|")}`);
      await press(page.locator("#header-menu button", { hasText: "Lint" }));
      await waitFor("Lint as the main action", async () => (await page.getAttribute("#scripts-control .main", "aria-label")) === "Run Lint");
      await details();
      await tapOrClick(touch ? '#details-card [data-act="run"]' : "#scripts-control .main");
      await waitFor("the drawer's Lint terminal", () => page.locator("#drawer-tabs button[aria-pressed='true']", { hasText: "Lint" }).isVisible());
      await waitFor("the script's output", () => page.evaluate(() => { const b = scriptShells.at(-1).term.buffer.active; for (let y = 0; y < b.length; y++) if (b.getLine(y).translateToString(true).trim() === "lint-42") return true; return false; }), 10000);
      await press(page.locator("#drawer-tabs button", { hasText: "claude" }));
      check(await page.evaluate(() => !active.el.hidden), "claude's terminal didn't come back");
      await openMenu();
      await press(page.locator("#header-menu button", { hasText: "Add action" }));
      await waitFor("the action dialog", () => page.locator("#composer-dialog form").isVisible());
      check((await text("#composer-dialog h2")) === "Add Action", "the dialog isn't T3's Add Action");
      await page.fill("#composer-dialog [name=name]", "Greet");
      await press(page.locator("#composer-dialog .icon-pick"));
      await press(page.locator('#composer-dialog [data-icon="test"]'));
      if (!touch) {
        await page.focus("#composer-dialog [name=keybinding]");
        await page.keyboard.press("Meta+Shift+Y");
        check((await page.inputValue("#composer-dialog [name=keybinding]")) === "⌘⇧Y", `shortcut: ${await page.inputValue("#composer-dialog [name=keybinding]")}`);
      }
      await page.fill("#composer-dialog [name=command]", "echo greet-$((1+1))");
      await press(page.locator("#composer-dialog [type=submit]"));
      await waitFor("the dialog to close", async () => !(await page.locator("#composer-dialog").isVisible()));
      const scriptsFile = () => {
        try {
          return JSON.parse(fs.readFileSync(path.join(DIR, "node", "scripts.json"), "utf8"));
        } catch {
          return null;
        }
      };
      await waitFor("Greet saved", () => JSON.stringify(scriptsFile() ?? {}).includes("Greet"));
      const greet = Object.values(scriptsFile())[0].find((s) => s.name === "Greet");
      check(greet?.icon === "test" && greet.command === "echo greet-$((1+1))", `saved: ${JSON.stringify(greet)}`);
      if (!touch) {
        await page.focus("#prompt");
        await page.keyboard.press("Meta+Shift+Y");
        await waitFor("the shortcut's run", () => page.locator("#drawer-tabs button[aria-pressed='true']", { hasText: "Greet" }).isVisible());
        await waitFor("its output", () => page.evaluate(() => { const b = scriptShells.at(-1).term.buffer.active; for (let y = 0; y < b.length; y++) if (b.getLine(y).translateToString(true).trim() === "greet-2") return true; return false; }), 10000);
      }
      await openMenu();
      await press(page.locator('#header-menu [data-edit="greet"]'));
      await waitFor("the edit dialog", async () => (await text("#composer-dialog h2")) === "Edit Action");
      await press(page.locator("#composer-dialog [data-delete]"));
      check((await text("#composer-dialog [data-delete]")) === 'Delete action "Greet"?', "delete didn't ask first");
      await press(page.locator("#composer-dialog [data-delete]"));
      await waitFor("Greet gone", async () => !JSON.stringify(scriptsFile() ?? { Greet: 1 }).includes("Greet"));
      for (let left = await page.locator("#drawer-tabs .close").count(); left; left--) await press(page.locator("#drawer-tabs .close").first());
      check(!(await page.locator("#drawer-tabs .close").count()), "a script's terminal stayed after its ×");
      if (await page.evaluate(() => document.body.dataset.drawer === "open")) await tapOrClick("#toggle-terminal");
      if (await page.locator("#details-card").isVisible()) await tapOrClick("#toggle-details");
      return menu.join(", ");
    } finally {
      fs.rmSync(t3, { force: true });
    }
  });

  await step("composer: the Git action commits with a written message, pushes and opens a PR, as T3 Code's picks it", async () => {
    const beta = path.join(HOME, "work/beta");
    const bare = path.join(DIR, "beta-remote.git");
    const git = (...args) => require("child_process").execFileSync("git", ["-C", beta, "-c", "user.email=s@example.test", "-c", "user.name=s", ...args]).toString().trim();
    require("child_process").execFileSync("git", ["init", "-q", "--bare", bare]);
    git("remote", "set-url", "origin", bare);
    const hints = await page.evaluate(() => [
      quickAction(null, { repo: false }, false).label,
      quickAction({ refs: [{ name: "x", current: true }], dirty: false, upstream: null, remote: false, ahead: 1, behind: 0 }, { repo: true }, false).label,
      quickAction({ refs: [{ name: "x", current: true }], dirty: false, upstream: "origin/x", remote: true, ahead: 1, behind: 2 }, { repo: true }, false).hint,
      quickAction({ refs: [], detached: true }, { repo: true }, false).hint,
    ]);
    check(JSON.stringify(hints) === JSON.stringify(["Initialize Git", "Publish repository", "Branch has diverged from upstream. Rebase/merge first.", "Create and checkout a ref before pushing or opening a PR."]), `quick actions: ${JSON.stringify(hints)}`);
    try {
      await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/beta"))));
      await waitFor("beta", async () => /beta/.test(await text("#chat-head .crumb")));
      fs.writeFileSync(path.join(beta, "keep.txt"), "kept");
      fs.writeFileSync(path.join(beta, "leave.txt"), "left out");
      await page.evaluate(() => Object.assign(threadOf(active), { gitFor: null }) && renderComposer(active));
      await waitFor("the Git action", async () => (await text("#git-action .main")) === "Commit, push & PR");
      // The label reads the same before the files were written, so the status that lists them is waited for.
      await waitFor("the written files in the Git status", () => page.evaluate(() => !!threadOf(active).git?.changed.includes("leave.txt")));
      // A phone commits from its Options menu and runs the Git action from the thread details card, as T3 Code's.
      if (touch) {
        await tapOrClick("#head-more");
        await press(page.locator("#panel-menu button", { hasText: /^Commit$/ }));
      } else {
        await tapOrClick("#git-action .more");
        await press(page.locator("#header-menu button", { hasText: "Commit" }));
      }
      await waitFor("the commit dialog", async () => (await text("#composer-dialog h2")) === "Commit changes");
      await page.uncheck('#composer-dialog .files label:has-text("leave.txt") input');
      await page.fill("#composer-dialog [name=message]", "Keep the kept file");
      await press(page.locator("#composer-dialog [type=submit]"));
      await waitFor("the commit", () => git("log", "-1", "--format=%s") === "Keep the kept file", 10000);
      check(git("status", "--porcelain").includes("leave.txt"), "the unchecked file went into the commit");
      await page.evaluate(() => Object.assign(threadOf(active), { gitFor: null }) && renderComposer(active));
      await waitFor("Commit, push & PR again", async () => (await text("#git-action .main")) === "Commit, push & PR");
      if (touch) {
        if (!(await page.locator("#details-card").isVisible())) await tapOrClick("#toggle-details");
        await tapOrClick('#details-card [data-act="git"]');
      } else await tapOrClick("#git-action .main");
      await waitFor("the progress label", async () => /\.\.\./.test(await text("#git-action .main .progress").catch(() => "")), 5000);
      await waitFor("gh to open the pull request", async () => {
        // A step that fails says why in the composer's note, which a bare timeout would hide.
        const failed = await page.evaluate(() => !composerNote.hidden && composerNote.hasAttribute("data-error") && composerNote.textContent);
        check(!failed, `the Git action failed: ${failed}`);
        return /^pr create --fill$/m.test(readLog("gh.log"));
      }, 20000);
      await waitFor("the action to end", async () => !(await page.evaluate(() => !!gitRun)), 10000);
      check(git("log", "-1", "--format=%s") === "Write the sweep's notes", `the written message: ${git("log", "-1", "--format=%s")}`);
      const branch = git("symbolic-ref", "--short", "HEAD");
      check(require("child_process").execFileSync("git", ["-C", bare, "branch"]).toString().includes(branch), "nothing was pushed");
      await page.evaluate(() => openPublishDialog(active));
      check((await text("#composer-dialog h2")) === "Publish repository", "no publish dialog");
      await page.fill("#composer-dialog [name=name]", "beta-pub");
      await press(page.locator("#composer-dialog [type=submit]"));
      await waitFor("gh repo create", async () => /repo create beta-pub --private --source \. --remote origin --push/.test(readLog("gh.log")));
      return hints.slice(0, 2).join(", ");
    } finally {
      if (await page.locator("#details-card").isVisible()) await tapOrClick("#toggle-details");
      git("remote", "set-url", "origin", "https://github.com/example/beta.git");
      await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
    }
  });

  await step("composer: thread details (⌘⇧B) hold the workspace, editor, actions, branch, Git action and PRs", async () => {
    check((await page.getAttribute("#open-in .main svg", "viewBox")) === "0 0 96 96", "the Open button doesn't show Zed's icon");
    if (touch) await tapOrClick("#toggle-details");
    else await page.keyboard.press("Meta+Shift+B");
    await waitFor("the details", () => page.locator("#details-card").isVisible());
    const rows = (await page.locator("#details-card .row .label").allTextContents()).map((r) => r.trim());
    check(rows.includes("Open in Zed") && rows.includes("Lint") && rows.some((r) => /^#4[12]$/.test(r)) && rows.includes("master"), `rows: ${rows.join(" | ")}`);
    const opened = readLog("open.log").trim().split("\n").length;
    await press(page.locator('#details-card [data-act="open-in"]'));
    await waitFor("Zed to open", async () => readLog("open.log").trim().split("\n").length === opened + 1);
    if (touch) await tapOrClick("#toggle-details");
    else await page.keyboard.press("Escape");
    await waitFor("the details to close", async () => !(await page.locator("#details-card").isVisible()));
    return rows.join(" | ");
  });

  // The next steps talk to claude in a thread of their own, so alpha's keeps its conversation and terminal.
  const inNewThread = async (run) => {
    const rows = await page.locator("#thread-list .thread-row").count();
    const before = await page.evaluate(() => active?.key);
    await page.evaluate(() => newThread(true));
    await waitFor("a new thread in alpha", async () => (await page.locator("#thread-list .thread-row").count()) === rows + 1 && (await page.evaluate((key) => active?.key !== key && active.view.running, before)), 10000);
    await waitTerm(/fake claude in .*work\/alpha/);
    try {
      return await run();
    } finally {
      await page.evaluate(() => closeTab(active));
      await waitFor("the thread to close", async () => (await page.locator("#thread-list .thread-row").count()) === rows);
      await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
      await waitFor("alpha", async () => /alpha/.test(await text("#chat-head .crumb")));
    }
  };

  await step("composer: in light themes the input and its strip cast T3 Code's composer shadow", async () => {
    const was = await page.evaluate(() => themeState().mode);
    const shadows = () => page.evaluate(() => [getComputedStyle(document.querySelector("#composer .box")).boxShadow, getComputedStyle(document.getElementById("composer-footer")).boxShadow]);
    try {
      await page.evaluate(() => setAppearanceMode("light"));
      const [box, strip] = await shadows();
      check(/0px 12px 28px -18px/.test(box) && /rgba\(0, 0, 0, 0\.4\)/.test(box), `the light composer's shadow is ${box}`);
      check(/0px 12px 28px -18px/.test(strip), `the light strip's shadow is ${strip}`);
      await page.evaluate(() => setAppearanceMode("dark"));
      const [darkBox, darkStrip] = await shadows();
      check(!/12px 28px/.test(darkBox) && /0px 14px 32px -18px/.test(darkStrip) && /0\.75\)/.test(darkStrip), `dark: ${darkBox} / ${darkStrip}`);
      return `${box} | ${strip}`;
    } finally {
      await page.evaluate((mode) => setAppearanceMode(mode), was);
    }
  });

  await step("composer: a thread without a project asks what we should work on, over its project picker", async () => {
    const rows = await page.locator("#thread-list .thread-row").count();
    await page.evaluate(() => newThreadWithoutProject());
    await waitFor("a thread without a project", async () => (await page.locator("#thread-list .thread-row").count()) === rows + 1 && (await page.evaluate(() => !!threadOf(active).workspace?.scratch)), 10000);
    await waitFor("the headline", () => page.locator("#hero-head h1").isVisible());
    const said = (await page.locator("#hero-head h1").innerText()).replace(/\s+/g, " ").trim();
    check(said === "What should we work on?", `the headline reads ${said}`);
    check((await text("#hero-head [data-hero-project]")) === "No project", `the picker reads ${await text("#hero-head [data-hero-project]")}`);
    check(!(await page.locator("#hero-head [data-hero-scratch]").count()), "it still offers to start without a project");
    const cwd = await page.evaluate(() => active.view.cwd);
    check(/\/scratch$/.test(cwd) && !cwd.includes("/work/"), `the thread runs in ${cwd}`);
    check(/No project/.test(await text("#chat-head .crumb")), `the header names ${await text("#chat-head .crumb")}`);
    await press(page.locator("[data-hero-project]"));
    await waitFor("the projects", () => page.locator('#composer-menu[data-kind="projects"] [data-path$="/alpha"]').isVisible());
    check(!(await page.locator('#composer-menu[data-kind="projects"] [data-path$="/scratch"]').count()), "the scratch folder is listed as a project");
    await press(page.locator('#composer-menu [data-path$="/alpha"]'));
    await waitFor("alpha's thread in place of the empty one", async () => (await page.locator("#thread-list .thread-row").count()) === rows + 1 && /alpha/.test(await text("#chat-head .crumb")), 10000);
    await page.evaluate(() => closeTab(active));
    await waitFor("the thread to close", async () => (await page.locator("#thread-list .thread-row").count()) === rows);
    await page.evaluate(() => activate(tabs.find((t) => t.view.cwd.endsWith("/alpha"))));
    await waitFor("alpha", async () => /alpha/.test(await text("#chat-head .crumb")));
    return `${said} · ${cwd.split("/").slice(-2).join("/")}`;
  });

  await step("composer: a permission offers Decline and Approve, with Cancel and Always allow under …", () => inNewThread(async () => {
    await sendPrompt("/ask");
    await waitFor("the banner", () => page.locator("#approval").isVisible());
    const actions = await page.locator("#approval .actions button").allTextContents();
    check(actions.map((a) => a.trim()).join("|") === "Decline|Approve|", `actions: ${actions.join(" | ")}`);
    check((await page.getAttribute("#approval [data-approval-more]", "aria-label")) === "More approval options", "no … button");
    check((await page.locator("#approval .options button").count()) === 0, "claude's options still show as buttons");
    check((await page.getAttribute('#approval [data-approval="accept"]', "class")) === "primary" && (await page.getAttribute('#approval [data-approval="decline"]', "class")) === "outline", "Approve isn't the primary");
    await press(gui("#approval [data-approval-more]"));
    await waitFor("the menu", () => page.locator('#composer-menu[data-kind="approval"]').isVisible());
    const entries = (await page.locator('#composer-menu[data-kind="approval"] button .label').allTextContents()).map((l) => l.trim());
    const hint = await text('#composer-menu[data-kind="approval"] button .hint');
    check(entries[0] === "Cancel" && entries[1] === "Always allow" && hint === "Yes, and don't ask again for rm commands", `menu: ${entries.join(" | ")} (${hint})`);
    const menu = await rect('#composer-menu[data-kind="approval"]');
    const more = await rect("#approval [data-approval-more]");
    check(menu.bottom <= more.top + 1, `the menu opens over the … button, not under it (${Math.round(menu.bottom)} > ${Math.round(more.top)})`);
    const seen = async (what) => ((await termText()).match(new RegExp(what, "g")) ?? []).length;
    const answers = async (choice, pick) => {
      const [chose, ran] = [await seen(`chose ${choice}`), await seen("ran it")];
      await pick();
      await waitFor(`claude to take ${choice}`, async () => (await seen(`chose ${choice}`)) === chose + 1);
      await waitFor("the banner to go", async () => !(await page.locator("#approval").isVisible()));
      await waitFor("claude to finish", async () => (await seen("ran it")) === ran + 1, 8000);
    };
    await answers("esc", () => press(page.locator('#composer-menu[data-kind="approval"] button', { hasText: "Cancel" })));
    await sendPrompt("/ask");
    await waitFor("the banner again", () => page.locator("#approval [data-approval]").first().isVisible());
    await answers(3, () => press(gui('#approval [data-approval="decline"]')));
    await sendPrompt("/ask");
    await waitFor("the banner a third time", () => page.locator("#approval [data-approval]").first().isVisible());
    await answers(1, () => press(gui('#approval [data-approval="accept"]')));
    return `${actions.map((a) => a.trim() || "…").join(" | ")}; … ${entries.join(" | ")}`;
  }));

  await step("composer: Claude's TodoWrite list is the Tasks tab, not a row in the thread", () => inNewThread(async () => {
    await sendPrompt("todos");
    await waitFor("the Tasks tab", () => page.locator("#composer-tasks").isVisible());
    check((await text("#composer-tasks .name")) === "Tasks" && (await text("#composer-tasks .current")) === "Fix the off-by-one" && (await text("#composer-tasks .count")) === "1/3", `the tab reads ${await text("#composer-tasks .row")}`);
    check((await page.getAttribute("#composer-tasks [data-tasks-toggle]", "aria-label")) === "Tasks: 1 of 3 complete. Current task: Fix the off-by-one", "the tab's label");
    if (!touch) check((await page.locator("#composer-tasks .segments span").count()) === 3 && (await page.locator("#composer-tasks .segments").isVisible()), "no progress segments");
    await waitFor("the answer", async () => /Working through the list/.test(await text("#thread")));
    check(!/TodoWrite/.test(await page.locator("#thread").innerText()), "the TodoWrite call shows in the thread");
    await press(page.locator("#composer-tasks [data-tasks-toggle]"));
    await waitFor("the list", () => page.locator("#composer-tasks .steps").isVisible());
    const steps = await page.locator("#composer-tasks .steps li").evaluateAll((els) => els.map((li) => `${li.dataset.status}:${li.querySelector(".text").textContent}`));
    check(steps.join("|") === "completed:Completed: Read the parser|in_progress:Running: Fix the off-by-one|pending:Pending: Add a test", `steps: ${steps.join(" | ")}`);
    await press(page.locator("#composer-tasks [data-tasks-toggle]"));
    await waitFor("the list to fold", async () => !(await page.locator("#composer-tasks .steps").count()));
    await sendPrompt("todos-done");
    await waitFor("a finished list to go once the turn is over", async () => !(await page.locator("#composer-tasks").isVisible()));
    await sendPrompt("todos-live");
    await waitFor("the finished list while claude works", async () => (await page.locator("#composer-tasks").isVisible()) && (await text("#composer-tasks .count")) === "3/3", 4000);
    check((await page.getAttribute("#composer-tasks .count", "class")) === "count done", "the finished count isn't green");
    await waitFor("the list to go with the turn", async () => !(await page.locator("#composer-tasks").isVisible()), 8000);
    return steps.join(" | ");
  }));

  await step("composer: a usage limit shows T3 Code's banner, which holds the resume or snoozes the thread", () => inNewThread(async () => {
    const key = await page.evaluate(() => active.key);
    await sendPrompt("limited");
    await waitFor("the banner", () => page.locator("#composer-banners .banner").isVisible());
    check((await text("#composer-banners .title")) === "Usage limit reached", `title: ${await text("#composer-banners .title")}`);
    check(/^Resets /.test(await text("#composer-banners .description")), `description: ${await text("#composer-banners .description")}`);
    check((await page.getAttribute("#composer-banners .banner", "data-variant")) === "warning", "the banner isn't a warning");
    const buttons = async () => (await page.locator("#composer-banners .actions button").allTextContents()).map((b) => b.trim()).join("|");
    check((await buttons()) === "Cancel auto-resume|Snooze until reset", `actions: ${await buttons()}`);
    await press(page.locator('#composer-banners [data-banner-act="resume"]'));
    await waitFor("the resume held", async () => (await buttons()) === "Resume at reset|Snooze until reset" && (await page.evaluate(() => active.view.held === true)));
    await press(page.locator('#composer-banners [data-banner-act="resume"]'));
    await waitFor("the resume back on", async () => (await buttons()) === "Cancel auto-resume|Snooze until reset" && (await page.evaluate(() => !active.view.held)));
    await press(page.locator('#composer-banners [data-banner-act="snooze"]'));
    await waitFor("the thread snoozed until the reset", () => page.evaluate((k) => sideMarks.snoozed[threadKey(tabByKey(k))]?.until === tabByKey(k).view.resume_at, key));
    await page.evaluate((k) => {
      wake(tabByKey(k));
      activate(tabByKey(k));
    }, key);
    await waitFor("the thread back", () => page.evaluate((k) => active?.key === k, key));
    await sendPrompt("spent");
    await waitFor("a limit with no reset time", async () => (await text("#composer-banners .description")) === "Reset time unavailable; retry manually");
    check(!(await page.locator("#composer-banners .actions").count()), "a limit with no reset time offers to resume at it");
    await sendPrompt("after the limit");
    await waitFor("the banner to go with the next turn", async () => !(await page.locator("#composer-banners").isVisible()), 8000);
    return "Usage limit reached · Resets …";
  }));

  await step("composer: a checkout moved off the thread's branch shows Branch changed from, which Restore branch undoes", async () => {
    const alphaDir = path.join(HOME, "work/alpha");
    const git = (...args) => require("child_process").execFileSync("git", ["-C", alphaDir, "-c", "user.email=s@example.test", "-c", "user.name=s", ...args]).toString().trim();
    const head = git("symbolic-ref", "--short", "HEAD");
    const born = (() => { try { return !!git("rev-parse", "-q", "--verify", "HEAD"); } catch { return false; } })();
    if (!born) git("commit", "-q", "--allow-empty", "-m", "base");
    git("branch", "elsewhere");
    const rows = await page.locator("#thread-list .thread-row").count();
    const before = await page.evaluate(() => active?.key);
    await page.evaluate(() => newThread(true));
    await waitFor("a new thread in alpha", async () => (await page.locator("#thread-list .thread-row").count()) === rows + 1 && (await page.evaluate((key) => active?.key !== key && active.view.running, before)), 10000);
    await waitTerm(/fake claude in .*work\/alpha/);
    try {
      await sendPrompt("elsewhere");
      await waitFor("the branch notice", () => page.locator("#composer-banners .banner").isVisible());
      const title = (await page.locator("#composer-banners .title").innerText()).replace(/\s+/g, " ").trim();
      check(title === "Branch changed from elsewhere", `the notice reads ${title}`);
      check((await page.getAttribute("#composer-banners .title code", "title")) === `This thread last ran on elsewhere. Sending will continue on ${head}.`, "the branch's tooltip");
      check((await page.getAttribute('#composer-banners [data-banner-act="dismiss"]', "aria-label")) === "Dismiss branch change notice", "no dismiss button");
      await press(page.locator('#composer-banners [data-banner-act="restore"]'));
      if (await page.locator("#composer-dialog form").isVisible()) {
        check((await text("#composer-dialog h2")) === "Switch to elsewhere?", `the confirm asks ${await text("#composer-dialog h2")}`);
        await press(page.locator('#composer-dialog [data-answer="yes"]'));
      }
      await waitFor("the checkout back on the thread's branch", async () => git("symbolic-ref", "--short", "HEAD") === "elsewhere");
      await waitFor("the notice to go", async () => !(await page.locator("#composer-banners").isVisible()));
      git("checkout", "-q", head);
      await page.evaluate(() => Object.assign(threadOf(active), { workspaceFor: null, gitFor: null }) && fetchWorkspace(active).then(() => renderComposer(active)));
      await waitFor("the notice back", () => page.locator("#composer-banners .banner").isVisible());
      await press(page.locator('#composer-banners [data-banner-act="dismiss"]'));
      await waitFor("the notice dismissed", async () => !(await page.locator("#composer-banners").isVisible()));
      return title;
    } finally {
      git("symbolic-ref", "HEAD", `refs/heads/${head}`);
      git("branch", "-D", "elsewhere");
      if (!born) git("update-ref", "-d", `refs/heads/${head}`);
      await page.evaluate(() => closeTab(active));
      await waitFor("the thread to close", async () => (await page.locator("#thread-list .thread-row").count()) === rows);
      await page.evaluate(() => {
        for (const t of tabs) Object.assign(threadOf(t), { workspaceFor: null, gitFor: null });
        activate(tabs.find((t) => t.view.cwd.endsWith("/alpha")));
      });
      await waitFor("alpha", async () => /alpha/.test(await text("#chat-head .crumb")));
    }
  });

  await step("composer: on a phone the unfocused composer collapses to one Ask anything... row", async () => {
    const state = () => page.evaluate(() => composer.dataset.collapsed);
    await page.evaluate(() => (prompt.blur(), rest()));
    if (touch) await tapOutsideComposer();
    if (!touch) {
      check((await state()) === "false", "a desktop composer collapsed");
      return "a desktop composer stays open";
    }
    await waitFor("the collapsed row", async () => (await state()) === "true");
    check((await page.getAttribute("#prompt", "placeholder")) === "Ask anything...", `placeholder: ${await page.getAttribute("#prompt", "placeholder")}`);
    check(!(await page.locator("#composer-attach").isVisible()) && (await page.locator("#send").isVisible()), "the row shows more than the prompt and Send");
    const row = await rect("#composer .box");
    check(row.height <= 52, `the row is ${Math.round(row.height)}px tall`);
    await page.fill("#prompt", "a draft");
    await waitFor("the composer to open on focus", async () => (await state()) === "false" && (await page.locator("#composer-attach").isVisible()));
    await page.evaluate(() => prompt.blur());
    await waitFor("the row again, with the draft", async () => (await state()) === "true" && (await page.inputValue("#prompt")) === "a draft");
    await tapOrClick("#prompt");
    await waitFor("the composer to open on a tap", async () => (await state()) === "false");
    await page.fill("#prompt", "");
    return `${Math.round(row.height)}px row`;
  });

  await step("composer: a fenced block in the prompt draws as a highlighted code block with T3 Code's language picker", async () => {
    try {
      await page.fill("#prompt", "Look at this:\n```ts\nconst answer = 42;\n```\nthanks");
      await waitFor("the code block", () => page.locator("#prompt-mirror .pblock").isVisible());
      await waitFor("the code colored", async () => (await page.locator("#prompt-mirror .pblock .code span[style*='color']").count()) > 0, 8000);
      const fit = await page.evaluate(() => [document.getElementById("prompt-mirror").scrollHeight, prompt.scrollHeight, [...document.querySelectorAll("#prompt-mirror .pl")].map((l) => Math.round(l.offsetTop))]);
      check(Math.abs(fit[0] - fit[1]) <= 1 && fit[2].length === 5, `the drawn lines don't sit on the textarea's: ${JSON.stringify(fit)}`);
      check((await page.locator("#prompt-mirror .fence").evaluateAll((els) => els.map((e) => getComputedStyle(e).color))).every((c) => /rgba\(.*, 0\)|transparent/.test(c)), "the fences show");
      const picker = page.locator("#prompt-blocks .block-lang");
      check((await picker.innerText()).trim() === "TypeScript" && (await picker.getAttribute("aria-label")) === "Code language: TypeScript", `the picker reads ${await picker.innerText()}`);
      // The picker is placed apart from the mirror's redraw, so it can trail it.
      const offFence = async () => {
        const [button, fence] = [await rect("#prompt-blocks .block-lang"), await rect("#prompt-mirror .fence-open")];
        return Math.abs(button.top + button.height / 2 - (fence.top + fence.height / 2));
      };
      await waitFor("the picker on the fence line", async () => (await offFence()) <= 3, 2000).catch(async () => check(false, `the picker sits ${await offFence()}px off the fence line`));
      await press(picker);
      await waitFor("the languages", () => page.locator('#composer-menu[data-kind="languages"]').isVisible());
      check((await page.getAttribute('#composer-menu[data-kind="languages"] .search', "placeholder")) === "Search languages", "the search isn't T3's");
      check(await page.locator('#composer-menu [data-lang="typescript"] .check').count() === 1, "the current language has no check");
      await page.fill('#composer-menu[data-kind="languages"] .search', "zzz");
      check((await text('#composer-menu[data-kind="languages"] .none')) === "No matching languages.", "no empty state");
      await page.fill('#composer-menu[data-kind="languages"] .search', "pyth");
      await press(page.locator('#composer-menu [data-lang="python"]'));
      await waitFor("the fence rewritten", async () => (await page.inputValue("#prompt")) === "Look at this:\n```python\nconst answer = 42;\n```\nthanks");
      await waitFor("the picker to follow", async () => (await picker.innerText()).trim() === "Python");
      await page.fill("#prompt", "```js");
      await page.press("#prompt", "Enter");
      check((await page.inputValue("#prompt")) === "```js\n\n```" && (await page.evaluate(() => prompt.selectionStart)) === 6, `Enter on a fence: ${JSON.stringify(await page.inputValue("#prompt"))}`);
      await page.keyboard.type("if (x) {");
      await page.press("#prompt", "Enter");
      await page.press("#prompt", "Tab");
      await page.keyboard.type("y();");
      check((await page.inputValue("#prompt")) === "```js\nif (x) {\n  y();\n```", `Enter and Tab in the block: ${JSON.stringify(await page.inputValue("#prompt"))}`);
      await page.press("#prompt", "Enter");
      check((await page.inputValue("#prompt")) === "```js\nif (x) {\n  y();\n  \n```", `Enter kept the indent: ${JSON.stringify(await page.inputValue("#prompt"))}`);
      await page.evaluate(() => prompt.setSelectionRange(prompt.value.length, prompt.value.length));
      await page.press("#prompt", "Enter");
      check((await page.inputValue("#prompt")) === "```js\nif (x) {\n  y();\n  \n```\n", `Enter on the closing fence: ${JSON.stringify(await page.inputValue("#prompt"))}`);
      check(!/echo: ```js/.test(await termText()), "Enter in the block sent the prompt");
      await page.fill("#prompt", "plain words");
      check(!(await page.locator("#prompt-mirror .pblock").count()) && !(await page.locator("#prompt-blocks .block-lang").count()), "a prompt without a fence drew a block");
      return "TypeScript → Python";
    } finally {
      await page.evaluate(() => (composerMenu.hidden = true));
      await page.fill("#prompt", "");
    }
  });

  await step("app view: the terminal drawer shows the same session", async () => {
    // A prompt of its own: the one sent about a hundred steps back is gone from the drawer by now.
    await sendPrompt("seen in the drawer");
    await waitFor("the answer", async () => (await page.locator("#thread .assistant", { hasText: "echo: seen in the drawer" }).count()) > 0);
    await tapOrClick("#toggle-terminal");
    await waitFor("the drawer", () => page.locator(".term:not([hidden])").evaluate((el) => getComputedStyle(el).visibility === "visible" && el.getBoundingClientRect().height > 60));
    check(/seen in the drawer/.test(await termText()), "the drawer's terminal lacks the prompt sent from the composer");
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
    } else await page.evaluate(() => closeTab(active));
    await waitFor("two threads", async () => (await rows()) === 2);
    if (touch) return "touch: ⌘⇧T and the header's project need a keyboard and a wide screen";
    await page.keyboard.press("Meta+Shift+T");
    await waitFor("the reopened thread", async () => (await rows()) === 3, 10000);
    await waitTerm(new RegExp(`resumed ${session}`));
    await waitFor("its conversation", async () => (await page.locator("#thread .bubble", { hasText: "before closing" }).count()) > 0);
    await page.evaluate(() => closeTab(active));
    await waitFor("two threads", async () => (await rows()) === 2);

    await openRow(await keyFor("alpha"));
    await page.click("#chat-head .crumb [data-new-here]");
    await waitFor("a thread from the header", async () => (await rows()) === 3, 10000);
    await waitFor("the header's new thread to open", async () => (await text("#chat-head .crumb .title")) === "New thread", 10000);
    check((await crumbProject()) === "alpha", `the header opened a thread in ${await crumbProject()}`);
    await page.evaluate(() => closeTab(active));
    await waitFor("two threads", async () => (await rows()) === 2);

    await tapOrClick("#add-project");
    await press(page.locator("#palette .item", { hasText: "Local folder" }));
    await waitFor("the path field", async () => (await page.getAttribute("#palette input", "placeholder")) === "Enter path (e.g. ~/projects/my-app)");
    await page.keyboard.press("Escape");
    await waitFor("the palette to close", () => page.locator("#palette").isHidden());
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
    await settingsNav("general");
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
    // Alpha linked #41 (a draft) and #42 (merged), which T3 Code's badge reads as "+2", open overall.
    const badge = `${rowSel(alpha)} .pr`;
    await waitFor("alpha's badge to read open overall", async () => (await prState(badge)) === "open" && /#42 - Merged/.test((await attr(badge, "aria-label")) ?? ""), 10000);
    check((await text(badge)) === "+2", `the badge reads ${await text(badge)}`);
    check((await tone(badge)) === "rgba(110, 231, 183, 0.9)", `the badge is ${await tone(badge)}`);
    check((await attr(badge, "aria-label")) === "PR #42 - Merged: Add notes, and 1 more linked; overall open", `aria-label: ${await attr(badge, "aria-label")}`);
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
      await page.hover(`${rowSel(alpha)} .l2`);
      await waitFor("the row's hover card", () => page.locator("#hover-card").isVisible(), 3000);
      const [row, card] = [await rect(rowSel(alpha)), await rect("#hover-card")];
      await page.screenshot({ path: path.join(shots, "row-draft-pr-stats.png"), clip: { x: 0, y: row.top - 8, width: card.right + 8, height: Math.max(row.bottom, card.bottom) - row.top + 16 } });
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
    const betaItem = page.locator("#project-menu .item").filter({ has: page.locator(".label", { hasText: /^beta$/ }) });
    if (touch) await press(betaItem);
    else await page.press("#project-menu input", "Enter");
    await waitFor("only beta's rows", async () => (await sideOrder()).every((p) => p === "beta") && (await text("#project-filter .label")) === "beta");
    check(await page.evaluate(() => load("gui-project") === projectKeyOf(tabs.find((t) => t.view.cwd.endsWith("/beta")))), "the filter wasn't saved");
    await tapOrClick("#project-filter");
    await press(betaItem.locator("[data-gear]"));
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
    await page.evaluate(() => document.body.dataset.page === "settings" || toggleSettings());
    await settingsNav("archive");
    await waitFor("it on the Archive page", async () => /to archive/.test(await text("#archive-settings .archived")));
    await press(gui("#archive-settings [data-unarchive]"));
    await waitFor("Unarchive to bring it back", async () => (await rows()) === 3 && (await text("#chat-head .crumb .title")) === "to archive", 15000);
    await waitFor("its conversation", async () => (await page.locator("#thread .bubble", { hasText: "to archive" }).count()) > 0, 10000);
    await rowMenu("#thread-list .thread-row.active");
    await menuPick("Delete");
    await waitFor("the delete dialog", async () => (await text("#side-dialog h3")) === 'Delete thread "to archive"?');
    await press(gui("#side-dialog .destructive"));
    await waitFor("the deleted thread gone", async () => (await rows()) === 2);
    await page.evaluate(() => document.body.dataset.page === "settings" || toggleSettings());
    await settingsNav("archive");
    check(/No archived threads/.test(await text("#archive-settings")), "the archive isn't empty");
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
    await tapOrClick("#composer-footer .pr");
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

  // T3 Code's sidebar chrome and chat header, round three.
  const tip = async (selector) => {
    await page.mouse.move(0, 0);
    await page.hover(selector);
    await waitFor(`the tooltip of ${selector}`, () => page.locator("#side-tip").isVisible(), 3000);
    return { text: await text("#side-tip"), box: await rect("#side-tip"), at: await rect(selector) };
  };
  const filterLabels = async () => {
    await showSidebar();
    await press(page.locator("#project-filter"));
    await waitFor("the project menu", () => page.locator("#project-menu").isVisible());
    const labels = await page.locator("#project-menu .item .label").allTextContents();
    await page.press("#project-menu input", "Escape");
    return labels;
  };
  const paletteHeads = () => page.locator("#palette .group .label").allTextContents();
  const paletteNames = () => page.locator("#palette .item .name").allTextContents();
  const rowsNow = () => page.locator("#thread-list .thread-row").count();
  const tabAt = (folder) => page.evaluate((f) => tabs.find((t) => t.view.cwd.endsWith(f))?.key, folder);
  const openThreadIn = async (folder) => {
    const before = await rowsNow();
    await page.evaluate((p) => openTab(tabs[0].machine, p), path.join(HOME, folder));
    await waitFor(`a thread in ${folder}`, async () => (await rowsNow()) === before + 1 && !!(await tabAt(`/${folder}`)), 10000);
    return tabAt(`/${folder}`);
  };

  await step("app view: every load opens the sidebar, the phone's sheet spans the screen, T3's tooltips, and Back returns to the thread", async () => {
    await openRow(await keyFor("alpha"));
    if (!touch) {
      await page.keyboard.press("Meta+b");
      await waitFor("the sidebar closed", async () => !(await page.locator("#sidebar").isVisible()));
    }
    await page.reload();
    await page.waitForFunction(() => document.body.dataset.hub === "online");
    await waitFor("the app view again", () => page.locator("#chat").isVisible(), 10000);
    await waitFor("the threads again", async () => (await page.evaluate(() => tabs.length)) >= 2, 10000);
    check((await page.locator("#sidebar").isVisible()) === !touch, `after a reload the sidebar is ${touch ? "open on a phone" : "closed"}`);
    await openRow(await keyFor("alpha"));
    await showSidebar();
    if (touch) {
      const width = await page.evaluate(() => innerWidth);
      const sheet = await rect("#sidebar");
      check(Math.round(sheet.width) === width - 12, `the sheet is ${sheet.width}px on a ${width}px screen`);
    } else {
      const usage = await tip('#sidebar footer [data-open-page="usage"]');
      check(usage.text === "Usage" && usage.box.bottom <= usage.at.top, `Usage's tooltip: ${usage.text} at ${usage.box.top}`);
      check((await tip('#sidebar footer [data-open-page="pull-requests"]')).text === "Pull Requests", "Pull Requests has no tooltip");
      check((await page.getAttribute('#sidebar footer [data-open-page="usage"]', "title")) === null, "Usage keeps a native tooltip");
      const pencil = await tip("#new-thread");
      check(pencil.text === "New thread (⌘N)New thread in current project: Shift+click (⌘⇧N)", `the pencil's tooltip: ${pencil.text}`);
      check((await tip("#add-project")).text === "Add project", "Add project has no tooltip");
      await page.mouse.move(0, 0);
    }
    await press(gui('#sidebar footer [data-open-page="pull-requests"]'));
    await waitFor("the Pull requests page", () => page.evaluate(() => document.body.dataset.page === "pull-requests"));
    await page.evaluate(() => openPage("usage"));
    await waitFor("the Usage page", () => page.evaluate(() => document.body.dataset.page === "usage"));
    await showSidebar();
    await press(gui("#sidebar footer .back"));
    await waitFor("Back to land on the thread", async () => await page.locator("#chat").isVisible(), 5000).catch(async () =>
      check(false, `Back went to ${await page.evaluate(() => document.body.dataset.page)}`));
    check((await crumbProject()) === "alpha", `Back opened ${await crumbProject()}`);
    check(!(await page.locator("#chat-head .update").count()), "the header still has the update button");
  });

  await step("app view: the pencil and Add project open T3's palette lists, and the row menu marks unread and copies three things", async () => {
    const alpha = await keyFor("alpha");
    await openRow(alpha);
    await showSidebar();
    await press(gui("#new-thread"));
    await waitFor("the palette's projects", () => page.locator("#palette").isVisible());
    check((await page.getAttribute("#palette input", "placeholder")) === "Search...", `placeholder: ${await page.getAttribute("#palette input", "placeholder")}`);
    const projects = await paletteNames();
    check((await paletteHeads()).join() === "Projects" && projects.includes("No project") && projects.includes("alpha") && projects.includes("beta"), `new thread in: ${projects.join(", ")}`);
    check(await page.locator("#projects").isHidden(), "the ＋ picker opened too");
    await page.press("#palette input", "Escape");
    if (!touch) {
      await page.keyboard.press("Meta+n");
      await waitFor("⌘N's palette", async () => (await page.locator("#palette").isVisible()) && (await paletteHeads()).join() === "Projects");
      await page.press("#palette input", "Escape");
    }
    await showSidebar();
    await press(gui("#add-project"));
    await waitFor("the palette's sources", async () => (await page.locator("#palette").isVisible()) && (await paletteHeads()).includes("Sources"));
    const sources = await paletteNames();
    check(sources.includes("Local folder") && sources.includes("New project"), `sources: ${sources.join(", ")}`);
    await press(page.locator("#palette .item", { hasText: "Local folder" }));
    await waitFor("the folder browser", async () => (await page.getAttribute("#palette input", "placeholder")) === "Enter path (e.g. ~/projects/my-app)", 5000);
    await page.keyboard.press("Escape");
    await waitFor("the palette to close", () => page.locator("#palette").isHidden());

    await rowMenu(rowSel(alpha));
    check(!(await page.locator("#side-menu button", { hasText: "Mark unread" }).isDisabled()), "Mark unread is off for a thread that isn't done");
    await menuPick("Copy");
    const copy = await page.locator("#side-menu button:not(.back) .label").allTextContents();
    check(copy.join() === "Path,Branch,Thread ID", `Copy: ${copy.join(", ")}`);
    await page.keyboard.press("Escape");
    await openRow(alpha);
  });

  await step("app view: line 3 as T3's: the branch at muted/40, a running command's terminal pulses, and several pull requests read +N or as a stack", async () => {
    const alpha = await keyFor("alpha");
    const beta = await keyFor("beta");
    await openRow(alpha);
    await showSidebar();
    check((await tone(`${rowSel(alpha)} .l3 .branch`)) === "rgba(129, 129, 129, 0.4)", `the branch is ${await tone(`${rowSel(alpha)} .l3 .branch`)}`);

    // The phone's sidebar sheet covers the chat header's Options menu, which opens the panel there.
    if (touch) await hideSidebar();
    await openPanelTo("terminal", "Terminal");
    await waitFor("the shell", async () => (await page.locator("#panel-body .xterm-rows").count()) === 1, 10000);
    await sleep(500);
    await page.locator("#panel-body .xterm-helper-textarea").focus();
    await page.keyboard.type("sleep 30\n");
    const pulse = `${rowSel(alpha)} .term-run`;
    await showSidebar();
    await waitFor("alpha's terminal to pulse", () => page.locator(pulse).count(), 8000);
    check((await attr(pulse, "aria-label")) === "1 terminal process running. Open terminal", `the pulse reads ${await attr(pulse, "aria-label")}`);
    check((await tone(pulse)) === "rgba(94, 234, 212, 0.9)", `the pulse is ${await tone(pulse)}`);
    check((await page.locator(`${pulse} .i`).evaluate((el) => getComputedStyle(el).animationName)) === "terminal-pulse", "the glyph doesn't pulse");
    await closePanelIfOpen();
    await openRow(beta);
    await showSidebar();
    await press(page.locator(pulse));
    await waitFor("alpha's terminal shown", async () => (await crumbProject()) === "alpha" && (await openTab("terminal").isVisible()), 5000);
    await page.locator("#panel-body .xterm-helper-textarea").focus();
    await page.keyboard.press("Control+c");
    await waitFor("the pulse to stop", async () => !(await page.locator(pulse).count()), 8000);
    if (touch) await hideSidebar();
    await press(gui('#panel-tabs .ptab[data-panel="terminal"] .lead'));
    await closePanelIfOpen();

    const badge = `${rowSel(alpha)} .pr[data-pr-list]`;
    await showSidebar();
    check((await text(badge)) === "+2", `alpha's two pull requests read ${await text(badge)}`);
    check(/^PR #4[12] - \w+: .+, and 1 more linked; overall open$/.test(await attr(badge, "aria-label")), `label: ${await attr(badge, "aria-label")}`);
    check((await tone(badge)) === "rgba(110, 231, 183, 0.9)", `the badge is ${await tone(badge)}`);
    await page.evaluate((k) => {
      const tab = tabByKey(k);
      linkPr(tab, "https://github.com/example/alpha/pull/43");
      unlinkPr(tab, "https://github.com/example/alpha/pull/41");
      renderSidebar();
    }, alpha);
    await waitFor("#42 and #43 as a stack", async () => /^Stack of 2 pull requests, \w+$/.test((await attr(badge, "aria-label")) ?? ""), 10000);
    check((await text(badge)) === "2" && (await page.locator(`${badge} .i`).innerHTML()).includes("m16.02 12"), `the stack reads ${await text(badge)}`);
    check((await attr(badge, "aria-label")).endsWith(await attr(badge, "data-state")), "the stack's color isn't its state's");
    await page.evaluate((k) => {
      const tab = tabByKey(k);
      linkPr(tab, "https://github.com/example/alpha/pull/41");
      unlinkPr(tab, "https://github.com/example/alpha/pull/43");
      renderSidebar();
    }, alpha);
    await waitFor("+2 again", async () => (await text(badge)) === "+2");
    await press(page.locator(badge));
    await waitFor("the Linked pull requests list", () => openTab("prs").isVisible(), 5000);
    await closePanelIfOpen();
    await openRow(alpha);
  });

  await step("app view: settings: project grouping and order, snooze limited threads, and the confirmations", async () => {
    const alpha = await keyFor("alpha");
    const beta = await keyFor("beta");
    await sideSetting("grouping");
    const on = (id) => page.locator(`#side-${id}`).isChecked();
    check((await on("grouping")) && !(await on("snoozeLimited")) && !(await on("confirmUnpin")) && !(await on("confirmArchive")) && (await on("confirmDelete")), "T3's defaults: grouping on, snooze off, ask only before delete");
    check((await page.inputValue("#side-projectOrder")) === "updated_at", `project order: ${await page.inputValue("#side-projectOrder")}`);
    await closeSettings();

    await openRow(beta);
    await sendPrompt("limited");
    const status = `${rowSel(beta)} .stamp.status`;
    await showSidebar();
    await waitFor("beta Limited", async () => (await page.locator(status).count()) && /Limited/.test(await text(status)), 10000);
    check((await tone(status)) === "rgb(254, 154, 0)", `Limited reads in ${await tone(status)}`);
    await sideSetting("snoozeLimited", true);
    await closeSettings();
    await showSidebar();
    // The shelf is open since the snooze step pressed it, so its head counts nothing; beta's row shows under it either way.
    await waitFor("beta snoozed until the limit lifts", async () => /^Snoozed( \(1\))?$/.test(await text('#thread-list .settled-head[data-shelf="snoozed"]')) && (await page.locator(`${rowSel(beta)} .wake-at`).count()) === 1, 5000);
    await rowMenu(rowSel(beta));
    await menuPick("Wake thread");
    await waitFor("beta awake", async () => !(await page.locator('#thread-list .settled-head[data-shelf="snoozed"]').count()));
    check(!(await page.locator('#thread-list .settled-head[data-shelf="snoozed"]').count()), "the limit snoozed beta again");
    await sideSetting("snoozeLimited", false);
    await closeSettings();
    await openRow(beta);
    await sendPrompt("past the limit");
    await answered("echo: past the limit");

    check((await filterLabels()).join() === "All projects,beta,alpha", `by last message: ${(await filterLabels()).join()}`);
    await sideSetting("grouping");
    await page.selectOption("#side-projectOrder", "manual");
    await closeSettings();
    check((await filterLabels()).join() === "All projects,alpha,beta", `manual: ${(await filterLabels()).join()}`);
    await sideSetting("grouping");
    await page.selectOption("#side-projectOrder", "updated_at");
    await closeSettings();

    const two = path.join(HOME, "work/alpha-two");
    if (!fs.existsSync(two)) {
      require("child_process").execFileSync("git", ["init", "-q", two]);
      require("child_process").execFileSync("git", ["-C", two, "remote", "add", "origin", "https://github.com/example/alpha.git"]);
    }
    const copy = await openThreadIn("work/alpha-two");
    await waitFor("alpha's two checkouts as one project", async () => [...(await filterLabels())].sort().join() === "All projects,alpha,beta", 10000);
    await sideSetting("grouping", false);
    await closeSettings();
    const apart = await filterLabels();
    check([...apart].sort().join() === "All projects,alpha,alpha-two,beta", `kept apart: ${apart.join()}`);
    await sideSetting("grouping", true);
    await closeSettings();
    await press(page.locator("#project-filter"));
    await press(page.locator("#project-menu .item").filter({ has: page.locator(".label", { hasText: /^alpha$/ }) }));
    await waitFor("both checkouts under alpha", async () => (await rowsNow()) === 2 && (await page.locator(rowSel(copy)).count()) === 1 && (await page.locator(rowSel(alpha)).count()) === 1);
    await press(page.locator("#project-filter"));
    await press(page.locator("#project-menu .item", { hasText: "All projects" }));

    await sideSetting("confirmArchive", true);
    await closeSettings();
    await rowMenu(rowSel(copy));
    await menuPick("Archive thread");
    await waitFor("the archive question", () => page.locator("#side-dialog").isVisible());
    check(/^Archive thread ".+"\?$/.test(await text("#side-dialog h3")), `asks: ${await text("#side-dialog h3")}`);
    await press(gui("#side-dialog .primary"));
    await waitFor("the copy archived", async () => !(await page.locator(rowSel(copy)).count()), 10000);
    // The Archive page's step expects to find the archive empty.
    await page.evaluate(() => {
      archived.splice(archived.findIndex((a) => a.cwd.endsWith("alpha-two")), 1);
      saveArchived();
    });
    await sideSetting("confirmArchive", false);

    const loose = await openThreadIn("work");
    await showSidebar();
    await waitFor("its workspace", () => page.evaluate((k) => !!threadOf(tabByKey(k)).workspace, loose));
    await page.evaluate(() => renderSidebar());
    check((await text(`${rowSel(loose)} .l3 .branch`)) === "" && !(await page.locator(`${rowSel(loose)} .l3 .branch .i`).count()), `a thread outside git shows ${await text(`${rowSel(loose)} .l3`)}`);
    await sideSetting("confirmDelete", false);
    await closeSettings();
    await rowMenu(rowSel(loose));
    await menuPick("Delete");
    await waitFor("the thread deleted without asking", async () => !(await page.locator(rowSel(loose)).count()), 10000);
    check(!(await page.locator("#side-dialog").isVisible()), "Delete asked with its confirmation off");
    await sideSetting("confirmDelete", true);

    await sideSetting("confirmUnpin", true);
    await closeSettings();
    await rowMenu(rowSel(alpha));
    await menuPick("Pin thread");
    const pin = `${rowSel(alpha)} .pin`;
    await waitFor("alpha pinned", () => page.locator(pin).count());
    await press(page.locator(pin));
    await waitFor("the unpin question", () => page.locator("#side-dialog").isVisible());
    check(/^Unpin thread ".+"\?$/.test(await text("#side-dialog h3")), `asks: ${await text("#side-dialog h3")}`);
    await press(gui("#side-dialog [data-cancel]"));
    await sleep(300);
    check((await page.locator(pin).count()) === 1, "Cancel unpinned alpha");
    await press(page.locator(pin));
    await press(gui("#side-dialog .primary"));
    await waitFor("alpha unpinned", async () => !(await page.locator(pin).count()));
    await sideSetting("confirmUnpin", false);
    await closeSettings();
    await openRow(alpha);
  });

  await step("app view: the header as T3's: the project on a phone, the chevron on the title only, icon-only split buttons, disabled Git actions explained, a folding parent crumb", async () => {
    const alpha = await keyFor("alpha");
    await openRow(alpha);
    check(await page.locator("#chat-head .crumb .project").isVisible(), "the header hides the project");
    check(!(await page.locator("#chat-head .crumb .sub-back").count()), "the header has a back button");
    for (const label of ["#open-in .main .label", "#git-action .main .label", "#scripts-control .main .label"]) {
      if (!(await page.locator(label).count())) continue;
      const box = await rect(label);
      check(box.width <= 1, `${label} shows (${box.width}px)`);
    }
    if (!touch) {
      await page.hover("#chat-head .crumb .project");
      await sleep(300);
      check((await page.locator("#chat-head .crumb .title .chev").evaluate((el) => getComputedStyle(el).opacity)) === "0", "the chevron shows over the project");
      await page.hover("#chat-head .crumb button.title");
      await waitFor("the chevron", async () => (await page.locator("#chat-head .crumb .title .chev").evaluate((el) => getComputedStyle(el).opacity)) === "1");
      check((await page.getAttribute("#chat-head .crumb button.title", "title")) === null, "the title keeps a native tooltip");
    }
    if (await page.locator("#git-action .more").isVisible()) {
      await press(gui("#git-action .more"));
      await waitFor("the Git menu", () => page.locator("#header-menu").isVisible());
      const why = page.locator("#header-menu .why").first();
      if (await why.count()) {
        const reason = await why.getAttribute("data-tip");
        if (touch) check((await why.locator(".note").textContent()) === reason, "a touch screen doesn't say why");
        else {
          const shown = await tip("#header-menu .why");
          check(shown.text === reason && shown.box.right <= shown.at.left + 1, `the reason: ${shown.text} at ${shown.box.left}`);
        }
      }
      await page.keyboard.press("Escape");
    }

    const beta = await keyFor("beta");
    await openRow(beta);
    await sendPrompt("agent");
    await answered("The agent reported back.");
    await press(lastTurn(".work > summary"));
    await press(gui("#thread .turn:last-of-type [data-subagent]"));
    await waitFor("the subagent's thread", () => page.locator("#chat-head .crumb .parent").isVisible(), 10000);
    check((await tone("#chat-head .crumb .parent")) === "rgb(129, 129, 129)", `the parent crumb is ${await tone("#chat-head .crumb .parent")}`);
    check((await text("#chat-head .crumb h2.title")) === "Look for the bug", `the title reads ${await text("#chat-head .crumb h2.title")}`);
    await page.evaluate(() => (document.querySelector("#chat-head .crumb").style.maxWidth = "150px"));
    await waitFor("the parent folded", () => page.locator("#chat-head .crumb .parent.folded").count());
    check((await page.locator("#chat-head .crumb .parent").evaluate((el) => getComputedStyle(el, "::after").content)) === '"..."', "the folded parent doesn't read ...");
    await page.evaluate(() => (document.querySelector("#chat-head .crumb").style.maxWidth = ""));
    await waitFor("the parent unfolded", async () => !(await page.locator("#chat-head .crumb .parent.folded").count()));
    await press(gui("#chat-head .crumb .parent"));
    await waitFor("the thread again", () => page.locator("#composer").isVisible());
    await openRow(alpha);
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
    await page.fill("#palette input", "choose project");
    await waitFor("the sub-page item", async () => (await paletteItems())[0] === "New thread in...");
    await page.keyboard.press("Enter");
    await waitFor("the projects sub-page", async () => (await page.getAttribute("#palette input", "placeholder")) === "Search...");
    const nested = await paletteItems();
    check(nested[1] === "No project" && nested.includes("alpha") && nested.includes("beta"), `sub-page: ${nested.join(" | ")}`);
    const hints = await page.locator("#palette .item .hint").allTextContents();
    check(hints.slice(0, 3).join(" ") === "⌘1 ⌥⌘N ⌘2", `sub-page hints: ${hints.join(" ")}`);
    await page.keyboard.press("Backspace");
    await waitFor("the root again", async () => (await page.inputValue("#palette input")) === "choose project");
    await page.fill("#palette input", "zzqx");
    await waitFor("the empty state", async () => (await text("#palette .empty")) === "No matching commands, projects, or threads.");
    await page.keyboard.press("Escape");
    check(!(await page.locator("#palette").isVisible()), "Escape left the palette open");
    await page.keyboard.press("Meta+k");
    await page.keyboard.type("find");
    await waitFor("Find in current thread", async () => (await paletteItems())[0] === "Find in current thread");
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
  // At phone width the metric and period are T3 Code's selects: a button and its menu.
  const USAGE_LABELS = { cost: "Cost", tokens: "Tokens", limits: "Limits", 1: "Past 24h", 7: "7 days", 30: "30 days", 90: "90 days" };
  const usagePick = async (seg, value) => {
    if (touch) await hideSidebar();
    if (await page.locator(`#page-view .seg[data-seg="${seg}"]`).isVisible()) return press(page.locator(`#page-view .seg[data-seg="${seg}"] button[data-value="${value}"]`));
    await press(page.locator(`#page-view .pick[data-pick="${seg}"]`));
    await press(page.locator("#usage-menu button", { hasText: USAGE_LABELS[value] }));
    await waitFor(`${USAGE_LABELS[value]} picked`, async () => (await text(`#page-view .pick[data-pick="${seg}"]`)) === USAGE_LABELS[value]);
  };
  const tile = (label) => text(`#page-view .tiles div:has(dt:text-is("${label}")) dd`);
  await step("shell: the Usage page shows limits, cost and tokens from the transcripts", async () => {
    seedUsage();
    await page.evaluate(() => save("gui-usage-page", null));
    await showSidebar();
    await tapOrClick('#sidebar footer [data-open-page="usage"]');
    await waitFor("the Usage page", () => page.locator("#page-view h1", { hasText: "Usage" }).isVisible());
    check(!(await page.locator("#chat").isVisible()), "the thread still shows beside Usage");
    await showSidebar();
    check(await page.locator("#sidebar footer .back").isVisible(), "the footer has no Back button");
    await waitFor("four limit windows", async () => (await page.locator("#page-view .limit-card").count()) === 4, 15000);
    const left = await page.locator("#page-view .limit-card .figure b").allTextContents();
    check(left.join() === "77%,26%,60%,75%", `limits left: ${left.join()}`);
    const labels = await page.locator("#page-view .limit-card .limit-text .label").allTextContents();
    check(labels.join() === "Session,Weekly,Weekly · Fable,Monthly spend", `windows: ${labels.join()}`);
    check(/↻ \d+[hm]/.test(await text('#page-view .limit-card[data-window="five_hour"] .chip')), "the session window has no reset time");
    if (touch) await usagePick("metric", "cost");
    else await page.keyboard.press("c");
    await waitFor("the total cost", async () => (await text("#page-view .total")) === "$3.13", 15000);
    check((await text("#page-view .headline > p")) === "2 sessions · API estimate", `under the total: ${await text("#page-view .headline > p")}`);
    await press(page.locator("#page-view .info"));
    await waitFor("the unpriced note", async () => /^API estimate excludes 33\.3% unpriced records\.$/.test(await text("#usage-pop")));
    await press(page.locator("#page-view .info"));
    check(!(await page.locator("#usage-pop").isVisible()), "a second click left the unpriced note open");
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
      await showSidebar();
      await tapOrClick("#sidebar footer .back");
    } else await page.keyboard.press("Escape");
    await waitFor("the thread again", () => page.locator("#chat").isVisible());
    return "limits 77%/26%/60%/75%, $3.13 in 30 days, $2.88 and 2.15M in 24h";
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
    await waitFor("the save to land", async () => (await text("#shell-dialog .save-status")) === "sweep-nodeSaved");
    await press(page.locator("#shell-dialog .close"));
    await waitFor("the priced total", async () => (await text("#page-view .total")) === "$3.14", 15000);
    check(!(await page.locator("#page-view .info").count()), "the unpriced note stayed");
    await press(page.locator('#page-view tr[data-model="claude-mystery-1"]'));
    await waitFor("the priced model", async () => (await stat("Cost")) === "$0.01");
    if (touch) await press(page.locator("#shell-dialog .close"));
    else await page.keyboard.press("Escape");
    check(!(await page.locator("#shell-dialog").isVisible()), "the details stayed open");
    check(await page.locator("#page-view h1", { hasText: "Usage" }).isVisible(), "closing the details left Usage");
    await press(page.locator("#page-view .env-filter"));
    await press(page.locator("#usage-menu button", { hasText: "Model prices" }));
    await press(page.locator('#shell-dialog tr[data-row="model:claude-mystery-1"] [data-reset]'));
    await press(page.locator("#shell-dialog [data-save]"));
    await waitFor("the reset to land", async () => (await text("#shell-dialog .save-status")) === "sweep-nodeSaved");
    await press(page.locator("#shell-dialog .close"));
    await waitFor("the list price again", async () => (await text("#page-view .total")) === "$3.13", 15000);
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    if (touch) {
      await showSidebar();
      await tapOrClick("#sidebar footer .back");
    } else await page.keyboard.press("Escape");
    await waitFor("the thread again", () => page.locator("#chat").isVisible());
    return "Unpriced, then $0.01 at $2/$10 per 1M";
  });

  const openUsage = async (metric, days) => {
    if (!(await page.locator("#page-view h1", { hasText: "Usage" }).isVisible())) {
      await showSidebar();
      await tapOrClick('#sidebar footer [data-open-page="usage"]');
      await waitFor("the Usage page", () => page.locator("#page-view h1", { hasText: "Usage" }).isVisible());
    }
    await usagePick("metric", metric);
    if (days) await usagePick("days", days);
  };
  const leaveUsage = async () => {
    if (touch) {
      await showSidebar();
      await tapOrClick("#sidebar footer .back");
    } else await page.keyboard.press("Escape");
    await waitFor("the thread again", () => page.locator("#chat").isVisible());
  };
  const seedDir = path.join(HOME, ".claude", "projects", "-sweep-usage");
  await step("usage: the chart's axis, scale and labels are T3 Code's, and an empty window draws one 0 line", async () => {
    await openUsage("cost", "30");
    await waitFor("the 30-day cost", async () => (await text("#page-view .total")) === "$3.13", 15000);
    const ticks = await page.locator("#page-view .y-axis span").allTextContents();
    check(ticks[0] === "0" && ticks.length >= 4 && ticks.length <= 6 && ticks.slice(1).every((t) => /^\$[\d,]+\.\d\d$/.test(t)), `y labels: ${ticks.join(" ")}`);
    check((await page.locator("#page-view .plot line.grid").count()) === ticks.length, "a gridline per label");
    const xs = await page.locator("#page-view .x-axis span").allTextContents();
    const want = await page.evaluate(() => [0, 15, 29].map((i) => monthDay(usageBuckets(30)[i])));
    check(xs.join() === want.join(), `x labels: ${xs.join()} where T3 Code's middle is the 16th day: ${want.join()}`);
    check((await page.locator("#page-view .provider-row .line").textContent()).trim() === "100.0% of cost · 2.16M tokens", `provider line: ${await text("#page-view .provider-row .line")}`);
    // With the transcripts gone, Refresh reads an empty window: one gridline at 0, no provider row, the tables' headers kept.
    fs.renameSync(seedDir, path.join(HOME, "usage-away"));
    try {
      await page.evaluate(() => {
        window.sawBusy = false;
        new MutationObserver(() => (window.sawBusy ||= !!document.querySelector('#page-view .refresh.spinning[aria-busy="true"]:disabled'))).observe(document.getElementById("page-view"), { subtree: true, childList: true, attributes: true });
      });
      await press(page.locator("#page-view .controls:not([hidden]) .refresh").filter({ visible: true }));
      await waitFor("the empty window", async () => (await text("#page-view .total")) === "$0.00", 15000);
      check(await page.evaluate(() => window.sawBusy), "Refresh didn't spin or disable while it read");
      check(!(await page.locator("#page-view .refresh.spinning").count()), "Refresh still spins");
      const empty = await page.locator("#page-view .y-axis span").allTextContents();
      check(empty.join() === "0" && (await page.locator("#page-view .plot line.grid").count()) === 1, `empty axis: ${empty.join()}`);
      check(!(await page.locator("#page-view .provider-row").count()), "the provider row shows with no usage");
      check((await page.locator("#page-view table.by-model th").count()) === 5 && (await text("#page-view table.by-model tbody")) === "No activity in this window.", "the model table lost its headers");
      await usagePick("breakdown", "day");
      const heads = await page.locator("#page-view table.by-time th").allTextContents();
      check(heads.join() === "Day,Total,Tokens" && (await text("#page-view table.by-time tbody")) === "No activity in this window.", `day table: ${heads.join()}`);
      await usagePick("breakdown", "model");
    } finally {
      fs.renameSync(path.join(HOME, "usage-away"), seedDir);
    }
    await press(page.locator("#page-view .refresh").filter({ visible: true }));
    await waitFor("the usage back", async () => (await text("#page-view .total")) === "$3.13", 15000);
    return `${ticks.join(" ")}; ${xs.join(", ")}`;
  });

  await step("usage: share bars list parts above zero with tooltips, and Tokens shows token shares", async () => {
    await openUsage("tokens", "30");
    await waitFor("tokens", async () => (await text("#page-view .total")) === "2.16M", 15000);
    const share = (model) => page.locator(`#page-view tr[data-model="${model}"] td.share-col`).textContent();
    check((await share("claude-mystery-1")) === "<0.1%" && (await share("claude-opus-5-5")) === "100.0%", `token shares: ${await share("claude-opus-5-5")}, ${await share("claude-mystery-1")}`);
    check((await page.locator("#page-view td.model .i.claude").count()) === 2, "model names lack the provider's icon");
    check(/max\(8px/.test(await page.getAttribute('#page-view tr[data-model="claude-mystery-1"] td.model i', "style")), "a small model's bar has no minimum");
    check((await text("#page-view .share h3")) === "Tokens by type" && (await page.locator("#page-view .share").count()) === 1, "Tokens mode shows cost shares");
    await usagePick("metric", "cost");
    check((await share("claude-mystery-1")) === "", `an unpriced model's cost share reads ${await share("claude-mystery-1")}`);
    await press(page.locator('#page-view tr[data-model="claude-mystery-1"]'));
    await waitFor("the model's details", () => page.locator("#shell-dialog .dialog-head", { hasText: "claude-mystery-1" }).isVisible());
    const legend = await page.locator("#shell-dialog .share .legend span").allTextContents();
    check(legend.join() === "Output1K", `legend: ${legend.join(" | ")}`);
    const segment = page.locator("#shell-dialog .share-bar span").first();
    if (touch) await segment.tap();
    else await segment.hover();
    await waitFor("the segment's tooltip", async () => (await text("#usage-pop")) === "Output · 1K · 100.0%");
    if (touch) await press(page.locator("#shell-dialog .close"));
    else await page.keyboard.press("Escape");
    if (await page.locator("#shell-dialog").isVisible()) await press(page.locator("#shell-dialog .close"));
    await waitFor("the details closed", async () => !(await page.locator("#shell-dialog").isVisible()));
    return legend.join();
  });

  await step("usage: the header's controls name their keys, fold into menus when narrow, and the keys skip fields and dialogs", async () => {
    await openUsage("cost", "30");
    const wide = await page.locator("#page-view .controls.wide").isVisible();
    if (wide) {
      check((await page.getAttribute('#page-view .seg[data-seg="metric"] [data-value="cost"]', "title")) === "Cost (C)", "Cost has no key in its title");
      check((await page.getAttribute('#page-view .seg[data-seg="days"] [data-value="7"]', "title")) === "7 days (⇧⌘2)", `7 days' title: ${await page.getAttribute('#page-view .seg[data-seg="days"] [data-value="7"]', "title")}`);
    } else {
      check(!(await page.locator("#page-view select").count()), "a native select in the header");
      await press(page.locator('#page-view .pick[data-pick="days"]'));
      check((await page.getAttribute("#usage-menu button >> nth=1", "title")) === "7 days (⇧⌘2)", "the period menu has no key titles");
      check((await page.locator('#usage-menu [aria-checked="true"]').textContent()).trim() === "30 days", "the menu doesn't mark 30 days");
      await press(page.locator("#usage-menu button", { hasText: "7 days" }));
      await waitFor("7 days", async () => (await text('#page-view .pick[data-pick="days"]')) === "7 days");
      await usagePick("days", "30");
    }
    const range = await page.locator("#page-view .range").isVisible();
    check(!range, "the window's range shows on a header narrower than 1280px");
    if (!touch) {
      await press(page.locator("#page-view .env-filter"));
      await press(page.locator("#usage-menu button", { hasText: "Model prices" }));
      await waitFor("the prices dialog", () => page.locator("#shell-dialog table.prices").isVisible());
      await press(page.locator("#shell-dialog [data-add]"));
      await waitFor("the new row's field focused", () => page.evaluate(() => document.activeElement?.dataset.field === "model"));
      await page.keyboard.press("Meta+Shift+1");
      await page.keyboard.press("t");
      check((await page.locator("#page-view .total").textContent()).trim() === "$3.13", "a key behind the dialog changed the page");
      await page.keyboard.press("Escape");
      await waitFor("the dialog to close", async () => !(await page.locator("#shell-dialog").isVisible()));
      check(await page.locator("#page-view h1", { hasText: "Usage" }).isVisible(), "Escape in the dialog left Usage");
      // T3 Code's useEscapeToGoBack: Escape in a focused field still goes back, and blurs it.
      await page.evaluate(() => {
        const field = document.createElement("input");
        field.id = "usage-field";
        document.querySelector("#page-view .page-wrap").prepend(field);
        field.focus();
      });
      await page.keyboard.press("Escape");
      await waitFor("Escape from a field to leave Usage", () => page.locator("#chat").isVisible());
      check(await page.evaluate(() => document.activeElement?.id !== "usage-field"), "the field kept focus");
      await openUsage("cost");
    }
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    return wide ? "segments with key titles" : "menus at phone width";
  });

  await step("usage: the environment filter checks machines, shows each one's status, and the page says when none is chosen or none can read", async () => {
    await openUsage("cost", "30");
    await waitFor("the 30-day cost", async () => (await text("#page-view .total")) === "$3.13", 15000);
    check((await text("#page-view .env-filter")) === "All environments", `filter label: ${await text("#page-view .env-filter")}`);
    await press(page.locator("#page-view .env-filter"));
    const node = page.locator("#usage-menu button", { hasText: "sweep-node" });
    check((await node.getAttribute("aria-checked")) === "true" && (await node.locator(".status").textContent()) === "Ready", `sweep-node: ${await node.textContent()}`);
    await press(page.locator("#usage-menu button", { hasText: "All environments" }));
    check(await page.locator("#usage-menu").isVisible(), "a checkbox closed the menu");
    await waitFor("no environment", async () => (await text("#page-view .usage")) === "Select an environment to see usage.");
    // Checking every machine again reads as all of them.
    const label = (await page.evaluate(() => machines.size > 1)) ? "sweep-node" : "All environments";
    await press(page.locator("#usage-menu button", { hasText: "sweep-node" }));
    await waitFor("sweep-node alone", async () => (await text("#page-view .env-filter")) === label && (await text("#page-view .total")) === "$3.13");
    if (label !== "All environments") {
      await press(page.locator("#usage-menu button", { hasText: "All environments" }));
      await waitFor("all again", async () => (await text("#page-view .env-filter")) === "All environments");
    }
    await page.keyboard.press("Escape").catch(() => {});
    if (await page.locator("#usage-menu").isVisible()) await press(page.locator("#page-view .env-filter"));
    // A machine that can't answer: the hub refuses a malformed read, and the page shows that environment's error.
    const id = await page.evaluate(() => {
      const id = onlineMachines().find((m) => machineName(m) === "sweep-node");
      const t = machines.get(id).transport;
      t.realCall = t.call;
      t.call = (call) => t.realCall(call.cmd === "token_usage" ? { ...call, since: "never" } : call);
      return id;
    });
    try {
      await press(page.locator("#page-view .refresh").filter({ visible: true }));
      await waitFor("the error", async () => /can't answer that/.test(await text("#page-view .env-errors")), 15000);
      check(await page.locator("#page-view .env-filter .alert").isVisible(), "no alert on the filter");
      await press(page.locator("#page-view .env-filter"));
      check((await page.locator("#usage-menu button", { hasText: "sweep-node" }).locator(".status").textContent()) === "Update required", "the status isn't Update required");
      check(/sweep-node: .*can't answer that/.test(await text("#usage-menu .note.notes")), "no coverage note");
      await press(page.locator("#page-view .env-filter"));
    } finally {
      await page.evaluate((m) => {
        const t = machines.get(m).transport;
        t.call = t.realCall;
      }, id);
    }
    await press(page.locator("#page-view .refresh").filter({ visible: true }));
    await waitFor("the usage back", async () => (await text("#page-view .total")) === "$3.13", 15000);
    await leaveUsage();
    return `filtered to ${label}`;
  });

  await step("usage: Limits shows every window claude reports, checks again on Refresh, and a segment's popover names the account", async () => {
    const checks = () => readLog("claude-control.log").split("\n").filter((l) => l === "get_usage").length;
    await openUsage("limits");
    await waitFor("four limit windows", async () => (await page.locator("#page-view .limit-card").count()) === 4, 15000);
    check(checks() >= 1, "opening Limits didn't ask claude for its limits");
    const before = checks();
    await press(page.locator("#page-view .refresh").filter({ visible: true }));
    await waitFor("Refresh to check the limits again", async () => checks() > before, 15000);
    await waitFor("the check to finish", async () => !(await page.locator("#page-view .refresh.spinning").count()), 15000);
    check(await page.locator('#page-view .limit-card[data-window="five_hour"] .pace').count() === 1, "the session window has no pace icon");
    check(!(await page.locator('#page-view .limit-card[data-window="monthly_spend"] :is(.tick, .pace)').count()), "the spend budget shows a pace");
    await press(page.locator('#page-view .limit-card[data-window="five_hour"] .segments > :is(.segment, .legend-row)').filter({ visible: true }).first());
    await waitFor("the segment's popover", () => page.locator("#usage-pop .segment-pop").isVisible());
    const rows = await page.locator("#usage-pop .pop-row").evaluateAll((els) => els.map((el) => [...el.children].map((c) => c.textContent).join(": ")));
    check(rows[0] === "Plan: Claude Max Subscription" && rows[1] === "Signed in: sweep-node" && rows[2] === "Left: 77%", `popover: ${rows.join(" | ")}`);
    check(/^Resets: .+ · in (\d+d )?(\d+h )?\d+[hm]$/.test(rows[3]) && /^Even pace: \d+% left by now$/.test(rows[4]), `popover times: ${rows.slice(3).join(" | ")}`);
    const email = page.locator("#usage-pop .redacted");
    check((await email.textContent()) !== "sweep@example.test" && (await email.textContent()).length === "sweep@example.test".length, "the email isn't hidden");
    await press(email);
    check((await email.textContent()) === "sweep@example.test" && (await email.getAttribute("title")) === "Click to hide email", "a click didn't reveal the email");
    await press(page.locator("#page-view .limit-card .label").first());
    await waitFor("the popover to close", async () => !(await page.locator("#usage-pop").isVisible()));
    return rows.slice(0, 3).join(", ");
  });

  await step("usage: the prices dialog maps a model to another, flags a bad row, and resets one to automatic", async () => {
    const models = () => page.locator("#page-view table.by-model tbody td.model span").allTextContents();
    const openPrices = async () => {
      await press(page.locator("#page-view .env-filter"));
      await press(page.locator("#usage-menu button", { hasText: "Model prices" }));
      await waitFor("the prices dialog", () => page.locator("#shell-dialog table.prices").isVisible(), 15000);
    };
    await openUsage("cost", "30");
    await waitFor("the 30-day cost", async () => (await text("#page-view .total")) === "$3.13", 15000);
    await openPrices();
    check((await text("#shell-dialog tbody")) === "No custom prices or mappings. Add a row to set one.", `empty table: ${await text("#shell-dialog tbody")}`);
    check((await text("#shell-dialog .unit")) === "USD / million tokens" && (await text("#shell-dialog .apply-to")) === "All environments", "no unit or Apply to");
    check(await page.locator("#shell-dialog [data-save]").isDisabled() && !(await page.locator("#shell-dialog [data-discard]").count()), "Save is on with nothing changed");
    await press(page.locator("#shell-dialog [data-add]"));
    await page.fill('#shell-dialog tbody tr:last-child [data-field="model"]', "claude-mystery-1");
    await page.fill('#shell-dialog tbody tr:last-child [data-field="alias"]', "claude-mystery-1");
    await waitFor("the row's error", async () => (await text("#shell-dialog .row-error")) === "Map to a different model.");
    check(await page.locator("#shell-dialog [data-save]").isDisabled(), "Save is on with a bad row");
    await page.fill('#shell-dialog tbody tr:last-child [data-field="alias"]', "claude-opus-5-5");
    await waitFor("the mapping", async () => (await text("#shell-dialog tbody tr:last-child .counted")) === "Counted as claude-opus-5-5");
    check((await text("#shell-dialog .dialog-foot .muted")) === "Changes apply to sweep-node", `destination: ${await text("#shell-dialog .dialog-foot .muted")}`);
    check((await text("#shell-dialog [data-discard]")) === "Discard changes" && (await text("#shell-dialog [data-save]")) === "Save changes", "the buttons aren't T3 Code's");
    await press(page.locator("#shell-dialog [data-save]"));
    await waitFor("the save to land", async () => (await text("#shell-dialog .save-status")) === "sweep-nodeSaved", 15000);
    await press(page.locator("#shell-dialog .close"));
    await waitFor("one model left", async () => (await models()).join() === "claude-opus-5-5", 15000);
    check(!(await page.locator("#page-view .info").count()), "the mapped model still counts as unpriced");
    await openPrices();
    check((await page.inputValue('#shell-dialog tr[data-row="model:claude-mystery-1"] [data-field="alias"]')) === "claude-opus-5-5", "the saved mapping isn't shown");
    const reset = page.locator('#shell-dialog tr[data-row="model:claude-mystery-1"] [data-reset]');
    await press(reset);
    check((await text('#shell-dialog tr[data-row="model:claude-mystery-1"]')) === "claude-mystery-1Automatic pricing after saving", `reset row: ${await text('#shell-dialog tr[data-row="model:claude-mystery-1"]')}`);
    check((await reset.getAttribute("aria-label")) === "Undo reset for claude-mystery-1", "no Undo on the reset row");
    await press(reset);
    check((await reset.getAttribute("aria-label")) === "Reset price for claude-mystery-1 to automatic" && (await page.locator("#shell-dialog [data-save]").isDisabled()), "Undo didn't restore the row");
    await press(reset);
    await press(page.locator("#shell-dialog [data-save]"));
    await waitFor("the reset to land", async () => (await text("#shell-dialog .save-status")) === "sweep-nodeSaved", 15000);
    await press(page.locator("#shell-dialog .close"));
    await waitFor("both models again", async () => (await models()).join() === "claude-opus-5-5,claude-mystery-1", 15000);
    return "mapped, then reset to automatic";
  });

  await step("usage: a first read draws T3 Code's skeleton, a refresh fades the figures, and with no environment the page says to connect one", async () => {
    await openUsage("cost", "30");
    await waitFor("the 30-day cost", async () => (await text("#page-view .total")) === "$3.13", 15000);
    const id = await page.evaluate(() => {
      const id = onlineMachines().find((m) => machineName(m) === "sweep-node");
      const t = machines.get(id).transport;
      t.realCall = t.call;
      t.call = async (call) => {
        if (call.cmd === "token_usage") await new Promise((r) => setTimeout(r, 2500));
        return t.realCall(call);
      };
      return id;
    });
    try {
      await press(page.locator("#page-view .refresh").filter({ visible: true }));
      await waitFor("the figures to fade", async () => Number(await page.locator("#page-view .headline > p .fig").evaluate((el) => getComputedStyle(el).opacity)) < 0.5);
      check(Number(await page.locator("#page-view .shares").evaluate((el) => getComputedStyle(el).opacity)) < 0.5, "the share bars don't fade");
      await waitFor("the refresh to end", async () => !(await page.locator("#page-view .usage[data-loading]").count()), 15000);
      await page.evaluate(() => {
        usageState.envs.clear();
        fetchUsage();
      });
      await waitFor("the skeleton", () => page.locator("#page-view .overview.skeleton").isVisible());
      const labels = await page.locator("#page-view .tiles dt").allTextContents();
      check(labels.join() === "Processed tokens,Cached input,Uncached input,Output,Cache savings", `skeleton totals: ${labels.join()}`);
      check(await page.locator("#page-view .skeleton .provider-row").count() === 1 && (await text("#page-view .breakdown-head h2")) === "Breakdown" && (await page.locator("#page-view .shares .ghost").count()) === 3, "the skeleton lacks the provider row, share bar or Breakdown");
      await waitFor("the usage back", async () => (await text("#page-view .total")) === "$3.13", 15000);
    } finally {
      await page.evaluate((m) => {
        const t = machines.get(m).transport;
        t.call = t.realCall;
      }, id);
    }
    const none = await page.evaluate(() => {
      const saved = [...machines];
      machines.clear();
      renderUsagePage();
      const shown = document.querySelector("#page-view .usage").textContent.trim();
      for (const [k, v] of saved) machines.set(k, v);
      renderUsagePage();
      return shown;
    });
    check(none === "Connect an environment to see usage.", `with no environment: ${none}`);
    await leaveUsage();
    return none;
  });

  const iconIs = (selector, name) => page.evaluate(([sel, n]) => {
    const drawn = Object.assign(document.createElement("div"), { innerHTML: icon(n) }).firstElementChild;
    return document.querySelector(sel)?.innerHTML === drawn.innerHTML;
  }, [selector, name]);
  const prNumbers = (group) => page.locator(`#page-view ${group ? `.pr-group[data-group="${group}"] ` : ""}.pr-row[data-url] .num`).allTextContents();
  const prFilter = async (kind, label) => {
    await tapOrClick('#page-view [data-pr="filters"]');
    await press(page.locator("#pr-menu button", { hasText: kind }));
    await press(page.locator("#pr-submenu button", { has: page.locator(`.label:text-is("${label}")`) }));
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
    // The page shows the rows it last loaded while it searches again, so the search can land after them.
    const searched = () => readLog("gh.log").match(/ -f (q=.*)$/m)?.[1];
    await waitFor("gh's search of the open projects", () => /^q=is:pr is:open sort:updated-desc repo:example\/\w+ repo:example\/\w+$/.test(searched() ?? ""), 10000);
    const busy = '#page-view .pr-row[data-url$="/pull/43"]';
    check(await page.locator(`${busy} .conflict`).isVisible(), "#43's conflict isn't marked");
    check((await page.getAttribute(`${busy} .checks`, "title")) === "Checks failing", "#43's failing checks aren't shown");
    check((await text(`${busy} .stat`)) === "+120 -30", `#43's size: ${await text(`${busy} .stat`)}`);
    const chips = await page.locator(`${busy} .label`).filter({ visible: true }).count();
    const more = (await page.locator(`${busy} .label .more`).filter({ visible: true }).allTextContents()).join();
    check(touch ? chips === 1 && more === "+3" : chips === 3 && more === "+1", `#43 shows ${chips} labels and ${more}`);
    check(await iconIs('#page-view .pr-group[data-group="authored"] h3 svg', "pen-line"), "Authored doesn't wear T3's pen-line");
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
      await showSidebar();
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
    await waitFor("the header's search box", () => page.locator('#page-view .page-head .pr-search[data-at="top"] input').isVisible());
    check((await page.inputValue('#page-view .page-head .pr-search input')) === "label:many", "the header's search box lost the query");
    check(!(await page.locator('#page-view .page-head [data-pr="search-top"]').count()), "the header offers a search button beside its search box");
    if (!touch) {
      await page.evaluate(() => document.activeElement?.blur());
      await page.keyboard.press("Meta+f");
      check(await page.evaluate(() => document.activeElement?.closest?.('.pr-search[data-at="top"]') && document.activeElement.selectionStart === 0 && document.activeElement.selectionEnd === 10), "⌘F didn't select the header's search");
    }
    await page.fill("#page-view .pr-search input", "");
    await waitFor("every row again", async () => (await prNumbers()).length === 3, 15000);
    const num = page.locator('#page-view .pr-row[data-url$="/pull/42"] .num');
    if (touch) {
      const box = await num.boundingBox();
      await num.dispatchEvent("contextmenu", { clientX: box.x + 4, clientY: box.y + 4 });
    } else await num.click({ button: "right" });
    await waitFor("the number's menu", () => page.locator("#side-menu").isVisible());
    const items = await page.locator("#side-menu button .label").allTextContents();
    check(items.join() === "Copy link,Open on GitHub", `menu: ${items.join()}`);
    await press(page.locator("#side-menu button", { hasText: "Copy link" }));
    const copied = await clipboard();
    check(copied === null || copied === "https://github.com/example/alpha/pull/42", `copied ${copied}`);
    if (!touch) {
      await page.evaluate(() => document.activeElement?.blur());
      await page.keyboard.press("Meta+f");
      check(await page.evaluate(() => document.activeElement?.closest?.('.pr-search[data-at="flow"]') && document.activeElement.selectionStart === 0 && document.activeElement.selectionEnd === 0), "⌘F didn't reach the toolbar's search");
      await page.locator("#page-view .pr-row").first().focus();
    }
    await press(page.locator('#page-view .pr-row[data-url$="/pull/43"] .checks'));
    await waitFor("the checks popover", () => page.locator("#pr-popover").isVisible());
    check((await text("#pr-popover .title")) === "Some checks were not successful", `the popover's title: ${await text("#pr-popover .title")}`);
    await waitFor("the checks", () => page.locator('#pr-popover li:has-text("build")').isVisible(), 10000);
    check((await text('#pr-popover li:has-text("build") .status')) === "Passed" && (await text('#pr-popover li:has-text("build") [data-details]')) === "Details", "the check's result or Details link is missing");
    if (touch) await page.evaluate(() => closePrPopover());
    else await page.keyboard.press("Escape");
    check(!(await page.locator("#pr-popover").isVisible()), "the checks popover stayed open");
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
      await showSidebar();
      await tapOrClick("#sidebar footer .back");
    }
    await waitFor("the thread again", () => page.locator("#chat").isVisible());
    return "100, then 150 rows";
  });

  const runFromPalette = async (query, title) => {
    await page.keyboard.press("Meta+k");
    await waitFor("the palette", () => page.locator("#palette").isVisible());
    await page.keyboard.type(query);
    await waitFor(`“${title}” in the palette`, async () => (await paletteItems())[0] === title);
    await page.keyboard.press("Enter");
  };
  await step("follow: ⌘K's Go to file and Search project contents open the panel's searches, and Escape comes back to the palette", async () => {
    if (touch) return "touch: no keyboard";
    await openRow(await keyFor("alpha"));
    await page.keyboard.press("Meta+k");
    await page.keyboard.type("go to file");
    await waitFor("Go to file", async () => (await paletteItems())[0] === "Go to file");
    check((await text("#palette .item[data-i='0'] .hint")) === "⌘P", `its hint: ${await text("#palette .item[data-i='0'] .hint")}`);
    await page.keyboard.press("Enter");
    await waitFor("the file picker", async () => (await page.getAttribute("#file-picker .query", "placeholder")) === "Search files…" && (await page.locator("#file-picker .query").isVisible()));
    check(!(await page.locator("#palette").isVisible()), "the palette stayed over the file picker");
    await page.keyboard.press("Escape");
    await waitFor("the palette again", () => page.locator("#palette").isVisible());
    check(!(await page.locator("#file-picker .query").isVisible()), "the file picker stayed open");
    check((await page.inputValue("#palette input")) === "go to file", `the palette's query came back as “${await page.inputValue("#palette input")}”`);
    await page.fill("#palette input", "search project contents");
    await waitFor("Search project contents", async () => (await paletteItems())[0] === "Search project contents");
    check((await text("#palette .item[data-i='0'] .hint")) === "⇧⌘F", `its hint: ${await text("#palette .item[data-i='0'] .hint")}`);
    await page.keyboard.press("Enter");
    await waitFor("the content search", async () => (await page.getAttribute("#file-picker .query", "placeholder")) === "Search project contents…");
    await page.keyboard.press("Escape");
    await waitFor("the palette after the content search", async () => (await page.inputValue("#palette input")) === "search project contents");
    await page.keyboard.press("Escape");
    check(!(await page.locator("#palette").isVisible()), "Escape left the palette open");
  });

  await step("follow: ⌘K links a pull request to the thread, then shows the thread's linked pull requests", async () => {
    if (touch) return "touch: no keyboard";
    await runFromPalette("link pull request", "Link pull request to thread");
    await waitFor("the Link pull request dialog", () => page.locator("#link-pr input").isVisible());
    await page.fill("#link-pr input", "https://github.com/example/alpha/pull/43");
    await page.press("#link-pr input", "Enter");
    await waitFor("the dialog to close", async () => !(await page.locator("#link-pr").count()), 10000);
    await runFromPalette("show linked pull requests", "Show linked pull requests");
    await waitFor("the Pull requests tab", () => gui('#panel-tabs .ptab.active[data-panel="prs"]').isVisible());
    await waitFor("#43 among them", () => gui('#panel-body .pr-row[data-prs-open$="/pull/43"]').isVisible(), 10000);
    await page.evaluate(() => unlinkPr(active, "https://github.com/example/alpha/pull/43"));
    await page.evaluate(() => closePanel());
  });

  await step("follow: ⌘K restarts the agent session on the thread's conversation", async () => {
    if (touch) return "touch: no keyboard";
    const session = await page.evaluate(() => active.view.session);
    check(!!session, "alpha has no conversation to resume");
    fs.writeFileSync(path.join(HOME, "claude-launch.log"), "");
    await runFromPalette("restart agent", "Restart agent session");
    await waitFor("claude to start again on the conversation", () => readLog("claude-launch.log").includes(`--resume ${session}`), 10000);
    await waitFor("the notice", async () => (await text("#side-notice")) === "Agent session will restart Your next message starts a fresh session.");
    await waitFor("the session running again", () => page.evaluate((s) => active.view.running && active.view.session === s, session), 10000);
    return readLog("claude-launch.log").trim();
  });

  await step("follow: ⌘K's New project makes a repository with a first commit, opens a thread in it, and publishes it", async () => {
    if (touch) return "touch: no keyboard";
    const gitconfig = path.join(HOME, ".gitconfig");
    const hadConfig = fs.existsSync(gitconfig);
    if (!hadConfig) fs.writeFileSync(gitconfig, "[user]\n\tname = Sweep\n\temail = sweep@example.test\n");
    fs.writeFileSync(path.join(HOME, "gh.log"), "");
    try {
      await runFromPalette("new project", "New project");
      await waitFor("the name field", async () => (await page.getAttribute("#palette input", "placeholder")) === "Project name");
      check((await text("#palette .summary .sub")) === "Goes in ~/pinned/", `summary: ${await text("#palette .summary .sub")}`);
      check(await page.locator("#palette .accessory [data-submit]").isDisabled(), "Create works without a name");
      check((await paletteGroups()).join() === "Options", `groups: ${(await paletteGroups()).join()}`);
      await page.keyboard.type("Sweep Café");
      await waitFor("the path preview", async () => (await text("#palette .summary .sub")) === "Creates ~/pinned/sweep-cafe");
      check(!(await page.locator("#palette .item.hi").count()), "a row is highlighted, so Enter would run it");
      await page.click("#palette .item:has-text('Create private repository on GitHub')");
      await waitFor("the checked box", () => page.locator("#palette .checkbox.on").isVisible());
      check((await text("#palette .item:has-text('Create private repository') .sub")) === "sweep-cafe", "the repository's name isn't shown");
      await page.keyboard.press("Meta+Enter");
      await waitFor("a thread in the new project", async () => (await crumbProject()) === "sweep-cafe", 15000);
      const dir = path.join(HOME, "pinned", "sweep-cafe");
      const log = require("child_process").execFileSync("git", ["log", "--format=%s"], { cwd: dir, encoding: "utf8" }).trim();
      check(log === "Initial commit", `git log: ${log}`);
      check(fs.readFileSync(path.join(dir, "README.md"), "utf8").includes("# Sweep Café") && fs.existsSync(path.join(dir, "assets", "icon.svg")), "README.md or the icon is missing");
      await waitFor("gh to publish it", () => /^repo create sweep-cafe --private --source \. --remote origin --push$/m.test(readLog("gh.log")), 10000);
      await waitFor("the published notice", async () => (await text("#side-notice")) === "Published to GitHub: sweeper/sweep-cafe");
      await page.evaluate(() => closeTab(active));
      return log;
    } finally {
      if (!hadConfig) fs.rmSync(gitconfig, { force: true });
    }
  });

  await step("follow: ⌘K's Open WSL folder browses the folders of a machine running under WSL", async () => {
    if (touch) return "touch: no keyboard";
    await openRow(await keyFor("alpha"));
    const machine = await page.evaluate(() => active.machine);
    await page.keyboard.press("Meta+k");
    await page.keyboard.type("open wsl");
    await waitFor("no WSL item off WSL", async () => !(await paletteItems()).includes("Open WSL folder"));
    await page.keyboard.press("Escape");
    await page.evaluate((m) => (machines.get(m).view.os = "wsl"), machine);
    try {
      await runFromPalette("open wsl", "Open WSL folder");
      await waitFor("the folder browser", async () => (await page.getAttribute("#palette input", "placeholder")) === "Enter path (e.g. ~/projects/my-app)");
      check((await page.inputValue("#palette input")) === "~/", "the browser doesn't start at ~/");
      await waitFor("the home folder's folders", async () => (await paletteGroups()).join() === "Directories");
      await page.keyboard.press("Escape");
    } finally {
      await page.evaluate((m) => (machines.get(m).view.os = "macos"), machine);
    }
  });

  await step("follow: ⌘K copies a prompt link, and the link opens a thread in its repository with the prompt in the composer", async () => {
    if (touch) return "touch: no keyboard";
    await openRow(await keyFor("alpha"));
    await page.evaluate(() => {
      window.copiedLinks = [];
      navigator.clipboard.writeText = async (t) => void window.copiedLinks.push(t);
    });
    await page.fill("#prompt", "Fix the flaky test");
    await runFromPalette("copy prompt link", "Copy prompt link");
    await page.keyboard.press("Meta+k");
    await page.keyboard.type("copy prompt link with repo");
    await waitFor("the link with repo and model", async () => (await paletteItems())[0] === "Copy prompt link with repo and model");
    await page.keyboard.press("Enter");
    const links = await page.evaluate(() => (delete navigator.clipboard.writeText, window.copiedLinks));
    const plain = links.find((l) => !l.includes("repo="));
    const full = links.find((l) => l.includes("repo="));
    check(plain === `${HUB}/new?prompt=Fix+the+flaky+test`, `copied ${plain}`);
    check(new URL(full).searchParams.get("repo") === "example/alpha" && new URL(full).searchParams.get("provider") === "claude", `copied ${full}`);
    await page.fill("#prompt", "");
    await page.dispatchEvent("#prompt", "input");
    const other = await context.newPage();
    const opened = await page.evaluate(() => tabs.map((t) => t.key));
    try {
      await other.goto(full);
      await other.waitForFunction(() => document.querySelector("#prompt")?.value === "Fix the flaky test" && location.pathname === "/", null, { timeout: 20000 });
      const landed = await other.evaluate(() => [projectName(active), document.querySelector("#side-notice").textContent.trim()]);
      // alpha-two, from the settings step, is a second checkout of example/alpha; the link takes the one used last.
      check(["alpha", "alpha-two"].includes(landed[0]), `the link opened in ${landed[0]}`);
      check(landed[1].startsWith("Prompt added from a link. Read it before you send."), `notice: ${landed[1]}`);
      await other.goto(`${HUB}/new?prompt=hi&repo=example/nowhere`);
      await other.waitForFunction(() => document.querySelector("#page-view h2")?.textContent === "You don't have example/nowhere", null, { timeout: 20000 });
      check((await other.locator("#page-view [data-link]").allTextContents()).join() === "Clone example/nowhere,Use without project", "the missing repository's buttons");
      await other.goto(`${HUB}/new?prompt=${"x".repeat(8001)}`);
      await other.waitForFunction(() => document.querySelector("#page-view p")?.textContent === "This prompt is longer than 8,000 characters, so Pinned did not open it.", null, { timeout: 20000 });
      await other.goto(`${HUB}/new?prompt=%E2%80%8B%E2%80%8B`);
      await other.waitForFunction(() => /only hidden characters/.test(document.querySelector("#page-view p")?.textContent ?? ""), null, { timeout: 20000 });
    } finally {
      await other.close();
      await page.evaluate((keep) => tabs.filter((t) => !keep.includes(t.key)).forEach((t) => closeTab(t)), opened);
    }
    return full;
  });

  const threadRows = () => page.locator("#thread-list .thread-row").count();
  // A step that failed may leave the palette or Settings over the app, which would block the next one's clicks.
  const focusClaudeTerm = () => waitFor("claude's terminal focused", () => page.evaluate(() => (active.term.focus(), !!document.activeElement?.closest(".xterm"))));
  const calm = () => page.evaluate(() => {
    closePalette();
    if (settingsOpen()) toggleSettings();
    composerMenu.hidden = true;
  });
  const paletteRow = (title) => page.locator("#palette .item", { has: page.locator(`.name:text-is("${title}")`) }).first();
  const openPaletteKeys = async () => {
    await page.keyboard.press("Meta+k");
    await waitFor("the palette", () => page.locator("#palette").isVisible());
  };
  await step("palette: T3's actions in its order, with its icons and ⇧⌘ hints; another chord or the backdrop closes it", async () => {
    if (touch) return "touch: no keyboard";
    await calm();
    await openRow(await keyFor("alpha"));
    await openPaletteKeys();
    const actions = await page.locator("#palette .group").first().locator(".item .name").allTextContents();
    const at = (t) => actions.indexOf(t);
    check(at("Change theme") > 0 && at("Change theme") < at("Open pull requests") && at("Open usage") < at("Open settings") && at("Open settings") < at("Project settings"), `order: ${actions.join(" | ")}`);
    check(!actions.includes("Switch to terminal view"), "Pinned's own Switch to terminal view is listed");
    const copy = actions.filter((a) => a === "Copy PR link" || a === "Copy thread ID");
    check(copy.length === 1, `copy rows: ${actions.join(" | ")}`);
    const hint = async (title) => (await paletteRow(title).locator(".hint").allTextContents()).join();
    check((await hint(copy[0])) === "⇧⌘C", `${copy[0]}'s hint: ${await hint(copy[0])}`);
    check((await hint("Change theme")) === "⌥⌘A", `Change theme's hint: ${await hint("Change theme")}`);
    check((await hint("New thread without a project")) === "⌥⌘N", `New thread without a project's hint: ${await hint("New thread without a project")}`);
    check((await hint("Open settings")) === "", `Open settings has a hint: ${await hint("Open settings")}`);
    const icons = await page.evaluate((copy) => {
      const want = { "New thread without a project": "message-square-dashed", "Find in current thread": "text-search", [copy]: "link", "Link pull request to thread": "link-2", "Open pull requests": "git-pull-request-arrow", "Open usage": "chart-no-axes-column" };
      const drawn = (html) => Object.assign(document.createElement("span"), { innerHTML: html }).innerHTML;
      return Object.entries(want).filter(([title, name]) => [...document.querySelectorAll("#palette .item")].find((el) => el.querySelector(".name").textContent === title)?.querySelector(".ic").innerHTML !== drawn(icon(name))).map(([title]) => title);
    }, copy[0]);
    check(!icons.length, `icons unlike T3's: ${icons.join(", ")}`);
    check((await page.locator("#palette footer").textContent()).replace(/\s+/g, "") === "NavigateEnterSelectEscClose", `footer: ${await page.locator("#palette footer").textContent()}`);
    const sidebar = await page.evaluate(() => document.body.dataset.sidebar);
    await page.keyboard.press("Meta+b");
    await waitFor("⌘B to close the palette and toggle the sidebar", async () => !(await page.locator("#palette").isVisible()) && (await page.evaluate(() => document.body.dataset.sidebar)) !== sidebar);
    await page.keyboard.press("Meta+b");
    await waitFor("the sidebar back", async () => (await page.evaluate(() => document.body.dataset.sidebar)) === sidebar);
    await openPaletteKeys();
    await page.mouse.click(8, 8);
    await waitFor("the backdrop to close the palette", async () => !(await page.locator("#palette").isVisible()));
    check(await page.evaluate(() => document.activeElement === prompt), "the composer didn't get the focus back");
    return actions.join(" | ");
  });

  await step("palette: search terms, thread rows' status, subtitle and PR number, and a thread found by its messages with the excerpt", async () => {
    if (touch) return "touch: no keyboard";
    await calm();
    const alpha = await keyFor("alpha");
    await openRow(alpha);
    await openPaletteKeys();
    for (const [query, title] of [["clone", "Add project"], ["github", "Add project"], ["keybindings", "Open settings"], ["draft", "New thread in alpha"]]) {
      await page.fill("#palette input", query);
      await waitFor(`“${query}” to find ${title}`, async () => (await page.locator("#palette .group").first().locator(".item .name").allTextContents()).includes(title));
    }
    await page.fill("#palette input", "");
    const title = await page.evaluate((k) => threadTitle(tabByKey(k)), alpha);
    const row = page.locator("#palette .group:has(.label:text-is('Recent Threads')) .item", { has: page.locator(`.name:text-is("${title}")`) }).first();
    const sub = (await row.locator(".sub").textContent()).replace(/\s+/g, "");
    const machine = await page.evaluate((k) => machineLabel(tabByKey(k).machine), alpha);
    check(sub.startsWith(`AAalpha·${machine}·`) && sub.endsWith("··Currentthread"), `subtitle: ${sub}`);
    check(await row.locator(".sub .agent").isVisible(), "no Claude icon in the subtitle");
    await page.fill("#palette input", "#42");
    await waitFor("#42's thread with its PR number", async () => (await page.locator("#palette .item .pr-num").allTextContents()).includes("#42"));
    await page.fill("#palette input", "one check failed");
    await waitFor("a thread found by its answer", () => page.locator("#palette .item .excerpt").first().isVisible(), 10000);
    const excerpt = (await text("#palette .item .excerpt")).replace(/\s+/g, " ");
    check(excerpt === "Agent: Looked around; one check failed.", `excerpt: ${excerpt}`);
    check((await text("#palette .item .excerpt mark")) === "one check failed", "the match isn't marked");
    await page.keyboard.press("Escape");
    return excerpt;
  });

  // A new thread, since by now every open one has linked a pull request.
  await step("palette: arrows pass over a disabled row, and ⌘K opens over Settings", async () => {
    if (touch) return "touch: no keyboard";
    await calm();
    return inNewThread(async () => {
      check(await page.evaluate(() => !linkedPrs(active).length), "the new thread has linked pull requests");
      await openPaletteKeys();
      await page.keyboard.type(">pull request");
      await waitFor("the disabled row", () => paletteRow("Show linked pull requests").evaluate((el) => el.getAttribute("aria-disabled") === "true"));
      const items = await paletteItems();
      const lit = [];
      for (let i = 0; i < items.length + 1; i++) {
        await page.keyboard.press("ArrowDown");
        lit.push(...(await page.locator("#palette .item.hi .name").allTextContents()));
      }
      check(lit.length === items.length + 1 && !lit.includes("Show linked pull requests"), `highlighted: ${lit.join(", ")} among ${items.join(" | ")}`);
      await page.keyboard.press("Escape");
      await page.evaluate(() => toggleSettings());
      await waitFor("Settings", () => page.locator("#settings-page").isVisible());
      await page.keyboard.press("Meta+k");
      await waitFor("the palette over Settings", () => page.locator("#palette").isVisible());
      await page.keyboard.press("Escape");
      await page.evaluate(() => settingsOpen() && toggleSettings());
    });
  });

  await step("palette: ⌘N with several projects lists them with ⌘1, ⌥⌘N and the rest, numbered before a query", async () => {
    if (touch) return "touch: no keyboard";
    await calm();
    await openRow(await keyFor("alpha"));
    const before = await threadRows();
    await page.keyboard.press("Meta+n");
    await waitFor("New thread in…", async () => (await page.getAttribute("#palette input", "placeholder")) === "Search..." && (await paletteItems()).length > 2);
    const names = await paletteItems();
    const hints = await page.locator("#palette .item .hint").allTextContents();
    check(names[1] === "No project" && hints.slice(0, 3).join(" ") === "⌘1 ⌥⌘N ⌘2", `rows: ${names.join(" | ")} hints: ${hints.join(" ")}`);
    const last = names.at(-1);
    const lastHint = await paletteRow(last).locator(".hint").textContent();
    await page.keyboard.type(last);
    await waitFor("the query", async () => (await paletteItems())[0] === last);
    check((await paletteRow(last).locator(".hint").textContent()) === lastHint, `${last} renumbered from ${lastHint}`);
    await page.keyboard.press("Escape");
    check((await threadRows()) === before, "⌘N started a thread instead of asking where");
    return names.join(" | ");
  });

  await step("palette: Add project lists T3's sources and clones a Git URL into the folder picked in the palette", async () => {
    if (touch) return "touch: no keyboard";
    await calm();
    const { execFileSync } = require("child_process");
    const source = path.join(HOME, "remotes", "widget");
    fs.mkdirSync(source, { recursive: true });
    execFileSync("git", ["init", "-q", source]);
    execFileSync("git", ["-C", source, "-c", "user.name=Sweep", "-c", "user.email=sweep@example.test", "commit", "-q", "--allow-empty", "-m", "first"]);
    const opened = await page.evaluate(() => tabs.map((t) => t.key));
    try {
      await runFromPalette("add project", "Add project");
      await waitFor("the Sources", async () => (await paletteGroups()).join() === "Sources");
      check((await paletteItems()).join() === "New project,Local folder,Git URL,GitHub repository", `sources: ${(await paletteItems()).join(" | ")}`);
      await press(paletteRow("Git URL"));
      await waitFor("the URL field", async () => (await page.getAttribute("#palette input", "placeholder")) === "Enter Git clone URL");
      await page.keyboard.type(`file://${source}`);
      await page.keyboard.press("Enter");
      await waitFor("the destination", async () => (await page.inputValue("#palette input")) === "~/widget");
      check((await text("#palette .summary-head")) === "Repository", "no Repository summary");
      await waitFor("Create & Clone", async () => (await text("#palette .accessory [data-submit]")).startsWith("Create & Clone"));
      await page.keyboard.press("Enter");
      await waitFor("a thread in the clone", async () => (await crumbProject()) === "widget", 20000);
      check(fs.existsSync(path.join(HOME, "widget", ".git")), "nothing cloned in ~/widget");
      await runFromPalette("add project", "Add project");
      await press(paletteRow("Local folder"));
      await page.fill("#palette input", "~/fresh/app");
      await waitFor("Create & Add", async () => (await text("#palette .accessory [data-submit]")).startsWith("Create & Add"));
      await page.keyboard.press("Enter");
      await waitFor("a thread in the made folder", async () => (await crumbProject()) === "app", 20000);
      check(fs.statSync(path.join(HOME, "fresh", "app")).isDirectory(), "~/fresh/app wasn't made");
      return execFileSync("git", ["-C", path.join(HOME, "widget"), "log", "--format=%s"], { encoding: "utf8" }).trim();
    } finally {
      await page.evaluate((keep) => tabs.filter((t) => !keep.includes(t.key)).forEach((t) => closeTab(t)), opened);
    }
  });

  await step("palette: New project's Add existing project opens the Sources, and the theme pages search as T3's", async () => {
    if (touch) return "touch: no keyboard";
    await calm();
    await runFromPalette("new project", "New project");
    await waitFor("the name field", async () => (await page.getAttribute("#palette input", "placeholder")) === "Project name");
    check((await text("#palette .accessory [data-submit]")).replace(/\s+/g, " ") === "CreateEnter", `Create: ${await text("#palette .accessory [data-submit]")}`);
    await press(paletteRow("Add existing project"));
    await waitFor("the Sources", async () => (await paletteGroups()).join() === "Sources");
    await page.keyboard.press("Escape");
    for (const view of ["Change theme", "Change appearance"]) {
      await runFromPalette(view.toLowerCase(), view);
      await waitFor(`${view}'s page`, async () => (await page.getAttribute("#palette input", "placeholder")) === "Search...");
      await page.keyboard.press("Escape");
    }
  });

  await step("palette: a pull request's linked-thread count opens the palette on its link with Linked thread rows", async () => {
    if (touch) return "touch: no keyboard";
    await calm();
    await openRow(await keyFor("alpha"));
    await page.evaluate(() => openPr(active, "https://github.com/example/alpha/pull/42"));
    await waitFor("#42's thread links", () => gui("#panel-body [data-pr-threads]").isVisible(), 15000);
    await press(gui("#panel-body [data-pr-threads]"));
    await waitFor("the palette on the link", async () => (await page.inputValue("#palette input")) === "https://github.com/example/alpha/pull/42");
    await waitFor("a Linked thread row", async () => (await page.locator("#palette .item .sub").allTextContents()).includes("Linked thread"));
    check((await paletteGroups()).join() === "Threads", `groups: ${(await paletteGroups()).join()}`);
    await page.keyboard.press("Escape");
    await closePanelIfOpen();
  });

  await step("palette: settings search opens a setting where it sits", async () => {
    if (touch) return "touch: no keyboard";
    await calm();
    await openPaletteKeys();
    await page.keyboard.type("chat width");
    await waitFor("the setting", async () => (await page.locator("#palette .group:has(.label:text-is('Settings')) .item .name").allTextContents())[0] === "Chat width");
    check((await text("#palette .group:has(.label:text-is('Settings')) .item .sub")) === "Settings · Appearance", `subtitle: ${await text("#palette .group:has(.label:text-is('Settings')) .item .sub")}`);
    await press(paletteRow("Chat width"));
    await waitFor("the Appearance page", async () => (await text("#settings-page h2")) === "Appearance");
    await waitFor("the Chat width row in focus", () => page.evaluate(() => document.activeElement?.id === "chat-width"));
    await page.evaluate(() => settingsOpen() && toggleSettings());
  });

  await step("shortcuts: ⌘T and ⌘W leave a closed panel alone and work an open one, ⌘← stays in the composer, labels read ⇧⌘", async () => {
    if (touch) return "touch: no keyboard";
    await calm();
    const alpha = await keyFor("alpha");
    await openRow(alpha);
    await closePanelIfOpen();
    const before = await threadRows();
    await page.keyboard.press("Meta+t");
    await page.keyboard.press("Meta+w");
    await page.waitForTimeout(300);
    check(!(await page.locator("#projects").isVisible()), "⌘T opened the project picker");
    check((await threadRows()) === before && (await page.evaluate(() => active.key)) === alpha, "⌘W closed the thread");
    await openPanelTo("files", "Files");
    const surfaces = () => page.evaluate(() => panelOf(active).surfaces.length);
    const open = await surfaces();
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press("Meta+w");
    await waitFor("⌘W to close the panel's tab", async () => (await surfaces()) < open);
    await closePanelIfOpen();
    await page.fill("#prompt", "hello world");
    await page.focus("#prompt");
    await page.keyboard.press("Meta+ArrowLeft");
    check((await page.evaluate(() => [active.key, prompt.selectionStart])).join() === `${alpha},0`, "⌘← left the composer's line start");
    await page.fill("#prompt", "");
    await page.dispatchEvent("#prompt", "input");
    const hints = await page.evaluate(() => rowMenuItems(active).map((i) => i.hint).filter(Boolean));
    check(hints.includes("⇧⌘P") && hints.includes("⇧⌘S"), `row menu hints: ${hints.join(" ")}`);
  });

  await step("shortcuts: a focused terminal keeps its keys: ⌘K stays out, ⌘D and ⌘⇧D split it, ⌘N opens another, ⌘W closes it", async () => {
    if (touch) return "touch: no keyboard";
    await calm();
    await openRow(await keyFor("alpha"));
    await closePanelIfOpen();
    const before = await threadRows();
    const shells = () => page.evaluate(() => scriptShells.filter((s) => s.tab === active).map((s) => ({ hidden: s.el.hidden, box: s.el.getBoundingClientRect().toJSON() })));
    const claudeBox = () => page.evaluate(() => (active.el.hidden ? null : active.el.getBoundingClientRect().toJSON()));
    await page.keyboard.press("Meta+j");
    await waitFor("the drawer", async () => (await page.evaluate(() => document.body.dataset.drawer)) === "open");
    await focusClaudeTerm();
    await page.keyboard.press("Meta+k");
    await page.waitForTimeout(300);
    check(!(await page.locator("#palette").isVisible()), "⌘K opened the palette over a focused terminal");
    await page.keyboard.press("Meta+d");
    await waitFor("a terminal beside claude's", async () => (await shells()).filter((s) => !s.hidden).length === 1 && !!(await claudeBox()));
    const [a, b] = [await claudeBox(), (await shells()).find((s) => !s.hidden).box];
    check(Math.abs(a.width - b.width) < 24 && b.left >= a.right - 2 && Math.abs(a.top - b.top) < 2, `not side by side: ${JSON.stringify([a, b])}`);
    check(!(await page.evaluate(() => panelOf(active).open)), "⌘D opened the Diff");
    await waitFor("the new terminal focused", () => page.evaluate(() => !!document.activeElement?.closest(".term.shell")));
    await page.keyboard.press("Meta+w");
    await waitFor("the split closed", async () => !(await shells()).length);
    await focusClaudeTerm();
    await page.keyboard.press("Meta+Shift+d");
    await waitFor("a terminal under claude's", async () => (await shells()).filter((s) => !s.hidden).length === 1 && !!(await claudeBox()));
    const [c, d] = [await claudeBox(), (await shells()).find((s) => !s.hidden).box];
    check(d.top >= c.bottom - 2 && Math.abs(c.left - d.left) < 2, `not stacked: ${JSON.stringify([c, d])}`);
    await waitFor("the stacked terminal focused", () => page.evaluate(() => !!document.activeElement?.closest(".term.shell")));
    await page.keyboard.press("Meta+w");
    await waitFor("the stacked one closed", async () => !(await shells()).length);
    await focusClaudeTerm();
    await page.keyboard.press("Meta+n");
    await waitFor("another terminal", async () => (await shells()).length === 1 && !(await claudeBox()));
    check((await threadRows()) === before, "⌘N in a terminal started a thread");
    await waitFor("it has the focus", () => page.evaluate(() => !!document.activeElement?.closest(".term.shell")));
    await page.keyboard.press("Meta+w");
    await waitFor("it closed", async () => !(await shells()).length);
    await page.evaluate(() => toggleDrawer(false));
  });

  await step("shortcuts: ⌘1-9 pick a model in the open model picker, each row showing its key", async () => {
    if (touch) return "touch: no keyboard";
    await calm();
    const alpha = await keyFor("alpha");
    await openRow(alpha);
    const model = () => text("#model-button .name");
    const was = await model();
    await page.keyboard.press("Meta+Shift+m");
    await waitFor("the model picker", () => page.locator('#composer-menu[data-kind="models"]').isVisible());
    const jumps = await page.locator("#composer-menu .model .jump").allTextContents();
    check(jumps[0] === "⌘1" && jumps[1] === "⌘2", `keys: ${jumps.join(" ")}`);
    const names = await page.locator("#composer-menu .list .model[data-row]:not(.legacy) .name > span:first-child").allTextContents();
    const pick = names[0].endsWith(was) ? 2 : 1;
    await page.keyboard.press(`Meta+${pick}`);
    await waitFor("the model picked", async () => names[pick - 1].endsWith(await model()) && !(await page.locator("#composer-menu").isVisible()), 10000);
    check((await page.evaluate(() => active.key)) === alpha, "⌘1-9 jumped to another thread");
    if (!names.some((n) => n.endsWith(was))) return `${names[pick - 1]}, after ${was}`;
    await page.keyboard.press("Meta+Shift+m");
    await waitFor("the model picker again", () => page.locator('#composer-menu[data-kind="models"]').isVisible());
    await page.evaluate((name) => [...composerMenu.querySelectorAll(".list .model[data-row]:not(.legacy)")].find((row) => row.querySelector(".name > span").textContent.endsWith(name)).click(), was);
    await waitFor("the model back", async () => (await model()) === was, 10000);
    return `${names[pick - 1]}, then ${was}`;
  });

  await step("shortcuts: ⌘⇧H opens T3's Run on picker of the machines", async () => {
    if (touch) return "touch: no keyboard";
    await calm();
    const alpha = await keyFor("alpha");
    await openRow(alpha);
    const opened = await page.evaluate(() => tabs.map((t) => t.key));
    try {
      await page.evaluate(() => newThreadIn(active));
      await waitFor("a new thread", async () => (await page.evaluate(() => active && workspaceOpen(active))), 15000);
      await page.keyboard.press("Meta+Shift+h");
      await waitFor("the Run on picker", () => page.locator('#composer-menu[data-kind="hosts"]').isVisible());
      check((await text("#composer-menu .heading")) === "Run on", `heading: ${await text("#composer-menu .heading")}`);
      check((await page.locator("#composer-menu button.checked .label").allTextContents()).join() === "sweep-node", "this machine isn't checked");
      await page.keyboard.press("Escape");
    } finally {
      // Closing a tab moves to its neighbour, which is beta, not the thread this started from.
      await page.evaluate((keep) => tabs.filter((t) => !keep.includes(t.key)).forEach((t) => closeTab(t)), opened);
      await page.evaluate((k) => activate(tabByKey(k)), alpha);
      await waitFor("alpha again", () => page.evaluate((k) => active?.key === k, alpha));
    }
  });

  await step("settings: Keybindings records a shortcut, takes a when clause and resets to the default", async () => {
    if (touch) return "touch: no keyboard";
    await calm();
    const showPage = async () => {
      await page.evaluate(() => settingsOpen() || toggleSettings());
      await press(page.locator('#settings-nav [data-page="keybindings"]'));
      await waitFor("the Keybindings page", async () => (await text("#settings-page h2")) === "Keybindings");
      await page.fill("#keybindings .kb-search input", "command palette");
      await waitFor("its one row", async () => (await page.locator("#keybindings .kb-row .kb-title").allTextContents()).map((t) => t.replace(/Custom$/, "")).join() === "Command Palette: Toggle");
    };
    const row = '#keybindings .kb-row:has(.kb-title:text("Command Palette: Toggle"))';
    const paletteFrom = async (keys) => {
      await page.evaluate(() => settingsOpen() && toggleSettings());
      await page.focus("#prompt");
      await page.keyboard.press(keys);
      await page.waitForTimeout(300);
      const open = await page.locator("#palette").isVisible();
      if (open) await page.keyboard.press("Escape");
      return open;
    };
    await showPage();
    check((await page.locator(`${row} .kb-pill kbd`).allTextContents()).join("") === "⌘K", "⌘K isn't the default");
    await press(page.locator(`${row} [data-kb-record]`));
    await waitFor("recording", async () => (await page.getAttribute(`${row} [data-keybinding-capture]`, "placeholder")) === "Press shortcut");
    await page.keyboard.press("Meta+Shift+y");
    await press(page.locator(`${row} [data-kb-save]`));
    await waitFor("the Custom rule", () => page.locator(`${row} .kb-badge`).isVisible());
    check((await page.locator(`${row} .kb-pill kbd`).allTextContents()).join("") === "⌘⇧Y", "the recorded chord");
    check(await paletteFrom("Meta+Shift+y"), "⌘⇧Y didn't open the palette");
    check(!(await paletteFrom("Meta+k")), "⌘K still opens the palette");
    await showPage();
    await press(page.locator(`${row} [data-kb-when]`));
    await page.fill(".kb-popover .kb-expr input", "false");
    await page.keyboard.press("Escape");
    await press(page.locator(`${row} [data-kb-save]`));
    await waitFor("the when saved", async () => (await text(`${row} [data-kb-when] span`)) === "false");
    check(!(await paletteFrom("Meta+Shift+y")), "a false when clause let ⌘⇧Y through");
    await showPage();
    await press(page.locator(`${row} [data-kb-menu]`));
    await press(page.locator(".kb-popover button", { hasText: "Reset to default" }));
    await waitFor("the default back", async () => !(await page.locator(`${row} .kb-badge`).count()) && (await text(`${row} [data-kb-when] span`)) === "!terminalFocus");
    check(await paletteFrom("Meta+k"), "⌘K didn't come back");
    await page.evaluate(() => settingsOpen() && toggleSettings());
  });

  await step("follow: Archive is its own settings page, newest first, with when each thread was archived and created", async () => {
    await page.evaluate(() => {
      const base = { machine: active.machine, cwd: active.view.cwd, session: null, project: "alpha" };
      archived.push({ ...base, name: "Older archive", at: nowS() - 7200, created: nowS() - 3 * 86400 }, { ...base, name: "Newer archive", at: nowS() - 120, created: nowS() - 7200 });
      saveArchived();
    });
    if (touch) {
      await page.evaluate(() => document.body.dataset.page === "settings" || toggleSettings());
      await settingsNav("archive");
    } else {
      await page.keyboard.press("Meta+k");
      await page.keyboard.type("archive");
      await waitFor("the Archive settings item", async () => (await paletteItems())[0] === "Archive");
      await page.keyboard.press("Enter");
    }
    await waitFor("the Archive page", async () => (await text("#settings-page h2")) === "Archive");
    check((await page.locator('#settings-nav [data-page="archive"].active').count()) === 1, "the nav doesn't mark Archive");
    check((await page.locator("#archive-settings .archived label").allTextContents()).join() === "Newer archive,Older archive", "not newest first");
    check((await text("#archive-settings .archived .meta")) === "Archived 2m ago · Created 2h ago", `meta: ${await text("#archive-settings .archived .meta")}`);
    check(!(await page.locator("#side-settings .archived").count()), "General still lists archived threads");
    for (const name of ["Newer archive", "Older archive"]) {
      const row = page.locator("#archive-settings .archived", { hasText: name });
      const box = await row.boundingBox();
      await row.dispatchEvent("contextmenu", { clientX: box.x + 24, clientY: box.y + 12 });
      await waitFor("the row's menu", async () => (await page.locator("#side-menu button .label").allTextContents()).join() === "Unarchive,Delete");
      await press(page.locator("#side-menu button", { hasText: "Delete" }));
    }
    await waitFor("the empty archive", async () => /No archived threads/.test(await text("#archive-settings")));
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    await page.evaluate(() => document.body.dataset.page === "settings" && toggleSettings());
  });

  await step("follow: the Device tab fills the panel's height", async () => {
    if (touch) await page.evaluate(() => toggleSidebar(false));
    await openPanelTo("device", "Device");
    await waitFor("the device list", async () => (await page.locator("#panel-body .device-list").count()) === 1, 20000);
    const [frame, body] = await page.evaluate(() => [document.querySelector("#panel-body .device-panel").getBoundingClientRect().height, document.querySelector("#panel-body").getBoundingClientRect().height]);
    check(frame > body - 80, `the Device tab is ${Math.round(frame)}px tall in a ${Math.round(body)}px panel`);
    await page.locator('#panel-tabs .ptab[data-panel="device"] .lead').dispatchEvent("click");
    await closePanelIfOpen();
    return `${Math.round(frame)}px of ${Math.round(body)}px`;
  });

  const openPulls = async () => {
    await showSidebar();
    await tapOrClick('#sidebar footer [data-open-page="pull-requests"]');
    await waitFor("the Pull requests page", () => page.locator("#page-view h1", { hasText: "Pull Requests" }).isVisible());
  };
  const leavePulls = async () => {
    if (await page.evaluate(() => document.body.dataset.panel === "open")) await page.evaluate(() => closePanel());
    if (touch) {
      await showSidebar();
      await tapOrClick("#sidebar footer .back");
    } else {
      await page.evaluate(() => document.activeElement?.blur());
      await page.keyboard.press("Escape");
    }
    await waitFor("the thread again", () => page.locator("#chat").isVisible());
  };
  const prRow = (n) => `#page-view .pr-row[data-url$="/pull/${n}"]`;
  await step("follow: Pull requests show a stacked pull request's stack, and a row matched only in its description says so", async () => {
    await openPulls();
    await waitFor("the rows", async () => (await prNumbers()).length === 3, 15000);
    check((await text(`${prRow(42)} .stack`)) === "1/2" && (await text(`${prRow(41)} .stack`)) === "2/2", "the stack badges are wrong");
    check((await page.getAttribute(`${prRow(42)} .stack`, "title")) === "View stack #7, layer 1 of 2", "the stack badge's tooltip is wrong");
    check(!(await page.locator(`${prRow(43)} .stack`).count()), "#43 has a stack badge");
    check((await page.locator(`${prRow(42)} [data-quick="merge"]`).getAttribute("title")) === "Open this pull request to merge its stack", "a stacked pull request merges from the list");
    await press(page.locator(`${prRow(42)} .stack`));
    await waitFor("the stack's layers", async () => (await page.locator("#side-menu button .label").allTextContents()).join() === "Stack #7,Draft the notes,Add notes,↳ main", 10000);
    check((await text('#side-menu button:has-text("Draft the notes") .hint')) === "#41 · sweep/notes-41 · Draft", "the layer's line is wrong");
    check((await page.locator('#side-menu button:has-text("Add notes") .i').count()) === 2, "the current layer isn't checked");
    check(/stacks\?pull_request=42/.test(readLog("gh.log")) && /stacks\/7/.test(readLog("gh.log")), "gh never read the stack");
    await press(page.locator('#side-menu button:has-text("Draft the notes")'));
    await waitFor("#41 in the side panel", async () => /#41/.test(await text('#panel-tabs .ptab.active[data-panel="pr"]')), 10000);
    await page.evaluate(() => closePanel());
    await page.fill("#page-view .pr-search input", "flaky");
    await waitFor("the description's match", async () => (await prNumbers()).join() === "#42");
    check(await page.locator(`${prRow(42)} .matched`).isVisible(), "no matched in the description pill");
    check((await page.getAttribute(`${prRow(42)} .matched`, "title")) === "Matched in the description", "the pill's tooltip is wrong");
    await page.fill("#page-view .pr-search input", "add notes");
    await waitFor("the title's match", async () => (await prNumbers()).join() === "#42");
    await waitFor("no pill on a title match", async () => !(await page.locator(`${prRow(42)} .matched`).count()));
    await page.fill("#page-view .pr-search input", "");
    await waitFor("every row again", async () => (await prNumbers()).length === 3, 15000);
    const bad = await overflowing();
    check(!bad.length, `wider than the screen: ${bad.join(", ")}`);
    await leavePulls();
  });

  await step("follow: Pull requests: dragging Close across rows closes each open one, and Escape calls it off", async () => {
    if (touch) return "touch: quick actions show while Shift is held";
    fs.writeFileSync(path.join(HOME, "gh.log"), "");
    await openPulls();
    await page.fill("#page-view .pr-search input", "label:many");
    await waitFor("the bulk rows", async () => (await prNumbers()).length >= 100, 15000);
    const center = async (sel) => {
      const box = await page.locator(sel).boundingBox();
      return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
    };
    try {
      await page.keyboard.down("Shift");
      await page.hover(prRow(1000));
      check((await page.getAttribute(`${prRow(1000)} [data-quick="close"]`, "title")) === "Close immediately, or drag across rows to close several", "Close doesn't say it drags");
      const from = await center(`${prRow(1000)} [data-quick="close"]`);
      await page.mouse.move(from.x, from.y);
      await page.mouse.down();
      const to = await center(prRow(1002));
      await page.mouse.move(from.x, to.y, { steps: 8 });
      await waitFor("three rows swept", async () => (await page.locator("#page-view .pr-row.sweeping").count()) === 3);
      await page.mouse.up();
      await waitFor("gh to close the three", () => [1000, 1001, 1002].every((n) => readLog("gh.log").includes(`pr close -- https://github.com/example/alpha/pull/${n}`)), 10000);
      await waitFor("the toast", async () => (await text("#panel-note")) === "Closed 3 pull requests");
      check(!/pull\/1003/.test(readLog("gh.log")), "a row past the sweep closed");
      await waitFor("the list to refresh", () => page.evaluate(() => !pulls.loading), 10000);
      await page.hover(prRow(1004));
      const again = await center(`${prRow(1004)} [data-quick="close"]`);
      await page.mouse.move(again.x, again.y);
      await page.mouse.down();
      await page.mouse.move(again.x, (await center(prRow(1006))).y, { steps: 8 });
      await waitFor("a second sweep", async () => (await page.locator("#page-view .pr-row.sweeping").count()) === 3);
      await page.keyboard.press("Escape");
      check(!(await page.locator("#page-view .pr-row.sweeping").count()), "Escape left rows swept");
      await page.mouse.up();
      await sleep(500);
      check(!/pull\/100[4-6]/.test(readLog("gh.log")), "a called-off sweep closed rows");
    } finally {
      await page.mouse.up();
      await page.keyboard.up("Shift");
      await page.fill("#page-view .pr-search input", "");
      await waitFor("every row again", async () => (await prNumbers()).length === 3, 15000);
      await leavePulls();
    }
    return "closed #1000-#1002";
  });

  await step("follow: Pull requests list an Enterprise Server's pull requests beside github.com's, with a Host filter", async () => {
    const delta = path.join(HOME, "work", "delta");
    if (!fs.existsSync(delta)) {
      require("child_process").execFileSync("git", ["init", "-q", delta]);
      require("child_process").execFileSync("git", ["-C", delta, "remote", "add", "origin", "https://ghe.example.test/example/delta.git"]);
    }
    fs.writeFileSync(path.join(HOME, "gh.log"), "");
    const before = await page.evaluate(() => active.key);
    await page.evaluate((p) => openTab(active.machine, p), delta);
    await waitFor("a thread in delta", async () => (await page.evaluate(() => projectName(active))) === "delta", 10000);
    try {
      await openPulls();
      await waitFor("both hosts' rows", async () => (await prNumbers()).length === 4, 15000);
      check(/auth token --hostname ghe\.example\.test/.test(readLog("gh.log")) && /api graphql --hostname ghe\.example\.test /.test(readLog("gh.log")), "gh never searched the Enterprise Server");
      check((await page.getAttribute(`${prRow(44)} .host`, "title")) === "ghe.example.test" && (await page.getAttribute(`${prRow(42)} .host`, "title")) === "github.com", "the rows don't name their hosts");
      check((await text(`${prRow(44)} .stat`)) === "+1,234 -5,678", `#44's size: ${await text(`${prRow(44)} .stat`)}`);
      await tapOrClick('#page-view [data-pr="filters"]');
      await press(page.locator("#pr-menu button", { hasText: "Host" }));
      check((await page.locator("#pr-submenu button .label").allTextContents()).join() === "All,github.com,ghe.example.test", "the Host filter's choices");
      await press(page.locator("#pr-submenu button", { hasText: "ghe.example.test" }));
      await waitFor("ghe.example.test's only", async () => (await prNumbers()).join() === "#44");
      check((await text('#page-view [data-pr="filters"] .badge')) === "1", "the Filters button doesn't count the host");
      check((await page.getAttribute('#page-view [data-pr="host"]', "aria-label")) === "Filter by provider: ghe.example.test", "the provider button doesn't name the host");
      await tapOrClick('#page-view [data-pr="host"]');
      await press(page.locator("#pr-menu button", { has: page.locator('.label:text-is("All")') }));
      await waitFor("every host again", async () => (await prNumbers()).length === 4);
      await page.evaluate(() => {
        document.head.append(Object.assign(document.createElement("style"), { id: "sweep-tall", textContent: "#page-view .page-wrap.pulls { min-height: 3000px }" }));
        document.querySelector("#page-view .page-body").scrollTop = 600;
      });
      await waitFor("the condensed header", () => page.locator("#page-view .page-head .condensed").isVisible());
      check((await page.locator("#page-view .page-head .condensed .crumb-menu").allTextContents()).join() === "Open,All,All", "the condensed header has no host menu with two hosts");
      await press(page.locator('#page-view .page-head [data-pr="search-top"]'));
      await waitFor("the header's search box", () => page.evaluate(() => !!document.activeElement?.closest?.('#page-view .page-head .pr-search[data-at="top"]')));
      check(await page.locator("#page-view .page-head .condensed").isVisible(), "the search icon scrolled the page back");
      await page.evaluate(() => {
        document.getElementById("sweep-tall").remove();
        document.querySelector("#page-view .page-body").scrollTop = 0;
      });
      await leavePulls();
    } finally {
      await page.evaluate(() => closeTab(active));
      await page.evaluate((k) => tabByKey(k) && activate(tabByKey(k)), before);
    }
  });

  const prSearch = '#page-view .pr-search[data-at="flow"] input';
  const prMenuLabels = (menu) => page.locator(`${menu} button .label`).allTextContents();
  await step("follow: Pull requests open in the page's own right panel, whose header toggle waits for one; ⌘⇧C copies its link and ⌘W closes it", async () => {
    await openPulls();
    await waitFor("the rows", async () => (await prNumbers()).length === 3, 15000);
    await page.evaluate(() => {
      Object.assign(PAGE_PANELS["pull-requests"], { open: false, surfaces: [], active: null });
      renderPanel();
      renderPullsPage();
    });
    const toggle = "#page-view .pulls-head .panel-toggle";
    check(await page.locator(`${toggle} button`).isDisabled(), "the panel toggle works with no pull request to show");
    check((await page.getAttribute(toggle, "title")) === "Select a pull request first", `the toggle says ${await page.getAttribute(toggle, "title")}`);
    const threadPanel = () => page.evaluate(() => (panelStates.get(active.key)?.surfaces ?? []).map((s) => s.id).join());
    const before = await threadPanel();
    await press(page.locator(prRow(43)));
    await waitFor("#43 in the page's panel", async () => /#43/.test(await text('#panel-tabs .ptab.active[data-panel="pr"]')), 10000);
    check(await page.locator(`${prRow(43)}.selected`).count() === 1, "#43's row isn't selected");
    check((await threadPanel()) === before, "the pull request went into the thread's panel");
    if (!touch) {
      await page.evaluate(() => navigator.clipboard?.writeText("").catch(() => {}));
      await page.keyboard.press("Meta+Shift+c");
      const link = await clipboard();
      check(link === null || link === "https://github.com/example/beta/pull/43", `⌘⇧C copied ${link}`);
      await waitFor("the toast", async () => (await text("#panel-note")) === "PR link copied · https://github.com/example/beta/pull/43");
      await tapOrClick("#panel-expand");
      await waitFor("the list to give way", async () => !(await page.locator("#page-view .pulls").isVisible()));
      await tapOrClick("#panel-expand");
      await waitFor("the list back", () => page.locator("#page-view .pulls").isVisible());
      await page.locator(prRow(43)).focus();
      await page.keyboard.press("Escape");
    } else await page.evaluate(() => goBack(-1) || closePage());
    await waitFor("the thread again", () => page.locator("#chat").isVisible());
    check(!(await page.evaluate(() => panelOf(active).surfaces.some((s) => s.id.endsWith("/pull/43")))), "the thread's panel shows the page's pull request");
    await openPulls();
    await waitFor("the page's panel again", async () => /#43/.test(await text('#panel-tabs .ptab.active[data-panel="pr"]')), 10000);
    if (!touch) await page.keyboard.press("Meta+w");
    else await page.locator('#panel-tabs .ptab.active [data-close-surface]').tap();
    await waitFor("the panel to close", () => page.evaluate(() => document.body.dataset.panel === "closed"));
    await waitFor("the toggle to wait again", () => page.locator(`${toggle} button`).isDisabled());
    await leavePulls();
  });

  await step("follow: Pull requests: Sort and Filters carry T3's icons and count, and Author, Labels, Draft, Review, Checks and Project narrow through gh", async () => {
    fs.writeFileSync(path.join(HOME, "gh.log"), "");
    await page.evaluate(() => pulls.states.clear());
    await openPulls();
    await waitFor("the rows", async () => (await prNumbers()).length === 3, 15000);
    const closeMenu = async () => {
      while (await page.locator("#pr-menu").isVisible()) await (touch ? page.evaluate(() => closePullsMenu()) : page.keyboard.press("Escape"));
    };
    await tapOrClick('#page-view [data-pr="sort"]');
    check((await prMenuLabels("#pr-menu")).join() === "Merge readiness,Blocked on me,Recently updated,Newest shown,Oldest shown,Largest shown,Smallest shown", "the sort choices");
    check(await page.locator("#pr-menu button").evaluateAll((bs) => bs.every((b) => b.firstElementChild?.matches("svg.i"))), "a sort choice has no icon");
    await closeMenu();
    check((await text('#page-view [data-pr="filters"]')) === "Filters", `the Filters button reads ${await text('#page-view [data-pr="filters"]')}`);
    check((await text('#page-view [data-pr="host"]')) === "All" && (await iconIs('#page-view [data-pr="host"] svg', "plug-2")), "no All button with a plug");
    await tapOrClick('#page-view [data-pr="filters"]');
    check((await prMenuLabels("#pr-menu")).join() === "State,Involvement,Author,Labels,Draft,Review,Checks,Project", `the filters: ${(await prMenuLabels("#pr-menu")).join()}`);
    check((await text('#pr-menu button:has(.label:text-is("State")) .value')) === "Open", "State doesn't show its choice");
    await press(page.locator("#pr-menu button", { hasText: "Involvement" }));
    check((await prMenuLabels("#pr-submenu")).join() === "All,Reviewing,Authored" && (await page.locator("#pr-submenu button svg.i:first-child").count()) === 3, "the involvement choices lack T3's icons");
    await closeMenu();
    await prFilter("Review", "Approved");
    await waitFor("approved only", async () => (await prNumbers()).join() === "#42", 10000);
    await waitFor("gh to search review:approved", () => / review:approved sort:/.test(readLog("gh.log")), 10000);
    check((await text('#page-view [data-pr="filters"] .badge')) === "1", "the Filters button doesn't count the review filter");
    await prFilter("Review", "All");
    await prFilter("Checks", "Failing");
    await waitFor("failing only", async () => (await prNumbers()).join() === "#43", 10000);
    await waitFor("gh to search status:failure", () => / status:failure sort:/.test(readLog("gh.log")), 10000);
    await prFilter("Checks", "All");
    await waitFor("every row", async () => (await prNumbers()).length === 3, 10000);
    await page.fill(prSearch, "draft:true");
    await waitFor("drafts only", async () => (await prNumbers()).join() === "#41", 10000);
    await waitFor("gh to search draft:true", () => / draft:true sort:/.test(readLog("gh.log")), 10000);
    await tapOrClick('#page-view [data-pr="filters"]');
    check((await text('#pr-menu button:has(.label:text-is("Draft")) .value')) === "Drafts only", "the Draft filter doesn't show the typed draft:true");
    await press(page.locator("#pr-menu button", { hasText: "Draft" }));
    await press(page.locator("#pr-submenu button", { has: page.locator('.label:text-is("All")') }));
    await waitFor("draft:true out of the search", async () => (await page.inputValue(prSearch)) === "" && (await prNumbers()).length === 3, 10000);
    await tapOrClick('#page-view [data-pr="filters"]');
    await press(page.locator("#pr-menu button", { hasText: "Labels" }));
    await waitFor("the label search", () => page.locator("#pr-submenu .menu-search input").isVisible());
    await page.fill("#pr-submenu .menu-search input", "tri");
    await waitFor("the host's label", () => page.locator("#pr-submenu button", { hasText: "triage" }).isVisible(), 10000);
    check(/labels\(first: 100, query: \$search/.test(readLog("gh.log")) && / -f search=tri/.test(readLog("gh.log")), "gh never searched the labels");
    await page.fill("#pr-submenu .menu-search input", "");
    await waitFor("the loaded labels", () => page.locator("#pr-submenu button", { hasText: "build" }).isVisible(), 10000);
    await press(page.locator("#pr-submenu button", { hasText: "build" }));
    await waitFor("build's rows", async () => (await prNumbers()).join() === "#43", 10000);
    await waitFor("gh to search label:\"build\"", () => / label:"build" sort:/.test(readLog("gh.log")), 10000);
    check(await page.locator("#pr-submenu button", { hasText: "build" }).getAttribute("aria-checked") === "true", "the label isn't checked, or the menu closed");
    await press(page.locator("#pr-submenu button", { hasText: "build" }));
    await waitFor("every row", async () => (await prNumbers()).length === 3, 10000);
    await closeMenu();
    await tapOrClick('#page-view [data-pr="filters"]');
    await press(page.locator("#pr-menu button", { hasText: "Author" }));
    await waitFor("the loaded authors", async () => (await text('#pr-submenu button:has(.label:text-is("sweeper")) .aside')) === "1 merges loaded", 10000);
    await page.fill("#pr-submenu .menu-search input", "mona");
    await waitFor("the host's author", () => page.locator("#pr-submenu button", { hasText: "monalisa" }).isVisible(), 10000);
    await press(page.locator("#pr-submenu button", { hasText: "monalisa" }));
    await waitFor("nothing by monalisa", async () => (await text("#page-view .pr-empty h2")) === "Nothing under these filters", 10000);
    await waitFor("gh to search the author", () => / author:"monalisa" sort:/.test(readLog("gh.log")), 10000);
    check((await text("#page-view .pr-empty p")) === "Widen the state, involvement or project filter to see more.", "the filtered empty state's text");
    check((await page.locator("#page-view .pr-empty .branch-art").count()) === 1 && (await text('#page-view .pr-empty [data-pr="refresh"]')) === "Check again", "the filtered empty state lacks T3's drawing or Check again");
    await prFilter("Author", "Anyone");
    await prFilter("Project", "beta");
    await waitFor("beta's only", async () => (await prNumbers()).join() === "#43", 10000);
    await waitFor("gh to search beta alone", () => / -f q=is:pr is:open sort:updated-desc repo:example\/beta$/m.test(readLog("gh.log")), 10000);
    await prFilter("Project", "All projects");
    await waitFor("every row", async () => (await prNumbers()).length === 3, 10000);
    check(!(await page.locator('#page-view [data-pr="filters"] .badge').count()), "a filter stayed on");
    await leavePulls();
  });

  await step("follow: Pull requests: a row names its author in full on hover and splits its +/− by kind, and Shift shows every row's actions, merging by the remembered method", async () => {
    fs.writeFileSync(path.join(HOME, "gh.log"), "");
    await openPulls();
    await waitFor("the rows", async () => (await prNumbers()).length === 3, 15000);
    check((await page.getAttribute(`${prRow(43)} .author`, "title")) === "Hubot Robot (@hubot)", `#43's author: ${await page.getAttribute(`${prRow(43)} .author`, "title")}`);
    check((await page.getAttribute(`${prRow(42)} .author`, "title")) === "sweeper" && (await text(`${prRow(42)} .author .monogram`)) === "S", "a nameless author's hover or monogram");
    if (touch) await page.locator(`${prRow(43)} [data-breakdown]`).focus();
    else await page.hover(`${prRow(43)} [data-breakdown]`);
    await waitFor("the split", async () => (await text("#pr-tip")).startsWith("Code"), 10000);
    const cells = await page.locator("#pr-tip .breakdown > span:not(.sep)").allTextContents();
    check(cells.join(" ") === "Code +80 -20 Tests +30 -5 Generated +10 -5 Total +120 -30", `the split: ${cells.join(" ")}`);
    check(/files\(first: 100.* -F number=43/.test(readLog("gh.log")), "gh never read #43's files");
    if (touch) {
      await page.evaluate(() => document.activeElement?.blur());
      await leavePulls();
      return "touch: quick actions show while Shift is held";
    }
    await page.mouse.move(2, 2);
    const shown = () => page.locator("#page-view .pr-row .quick").filter({ visible: true }).count();
    await page.keyboard.down("Shift");
    try {
      await waitFor("every row's actions", async () => (await shown()) === 3);
      check((await page.locator(`${prRow(41)} .quick button`).allTextContents()).join() === "Close,Ready for review", "#41's actions");
      check((await page.locator(`${prRow(43)} .quick button`).allTextContents()).join() === "Close,Merge", "#43's actions");
      check(await page.locator("#page-view .pr-row .quick button").evaluateAll((bs) => bs.every((b) => b.querySelector("svg.i"))), "an action has no icon");
      await page.evaluate(() => save("gui-merge-method", "squash"));
      await press(page.locator(`${prRow(43)} [data-quick="merge"]`));
      await waitFor("gh to squash #43", () => /pr merge --squash -- https:\/\/github.com\/example\/beta\/pull\/43/.test(readLog("gh.log")), 10000);
      await waitFor("the toast", async () => (await text("#panel-note")) === "Merge requested");
      await page.evaluate(() => save("gui-merge-method", "rebase"));
      await waitFor("#43 free to act again", () => page.locator(`${prRow(43)} [data-quick="merge"]:not([disabled])`).isVisible());
      await press(page.locator(`${prRow(43)} [data-quick="merge"]`));
      await waitFor("gh to merge #43 by the first method beta allows", () => /pr merge --merge -- https:\/\/github.com\/example\/beta\/pull\/43/.test(readLog("gh.log")), 10000);
    } finally {
      await page.keyboard.up("Shift");
      await page.evaluate(() => save("gui-merge-method", null));
      fs.rmSync(path.join(HOME, ".fake-gh-43"), { force: true });
    }
    await waitFor("the actions to hide", async () => (await shown()) === 0);
    await leavePulls();
  });

  await step("follow: Pull requests: a search spins and says it searches every host, refresh spins, Load more says so, and the query is remembered", async () => {
    await openPulls();
    await waitFor("the rows", async () => (await prNumbers()).length === 3, 15000);
    await page.fill(prSearch, "slowly");
    await waitFor("the search's spinner", () => page.locator('#page-view .pr-search[data-at="flow"] .spin').isVisible());
    await waitFor("the searching rows", async () => (await text("#page-view .pr-ghosts .caption")) === "Searching every host for “slowly”");
    const ghost = await page.locator("#page-view .ghost-row").evaluateAll((rows) => rows.map((r) => `${r.querySelectorAll(".dot").length}${r.querySelectorAll(".lines .ghost").length}${r.querySelectorAll(".ends .ghost").length}`));
    check(ghost.length === 5 && ghost.every((g) => g === "122"), `the loading rows: ${ghost.join()}`);
    check(await page.locator('#page-view .pr-toolbar [data-pr="refresh"]').isDisabled() && (await page.locator('#page-view .pr-toolbar [data-pr="refresh"] .spin').count()) === 1, "refresh doesn't spin while loading");
    await waitFor("nothing matches", async () => (await text("#page-view .pr-empty h2")) === "Nothing matches “slowly”", 15000);
    check((await page.locator('#page-view .pr-empty [data-pr="clear"] svg').count()) === 1 && (await page.locator('#page-view .pr-empty [data-pr="refresh"] svg').count()) === 1, "Clear search or Check again has no icon");
    await page.fill(prSearch, "label:many slowly");
    await waitFor("GitHub's first page", async () => (await prNumbers()).length === 100, 20000);
    await press(page.locator('#page-view [data-pr="more"]'));
    await waitFor("Loading more", async () => (await text("#page-view .pr-foot")) === "Loading more" && (await page.locator("#page-view .pr-foot .spin").count()) === 1);
    await waitFor("the second page", async () => (await prNumbers()).length === 150, 20000);
    await page.fill(prSearch, "zzqx");
    await waitFor("nothing matches", async () => (await text("#page-view .pr-empty h2")) === "Nothing matches “zzqx”", 10000);
    if (!touch) {
      await page.focus(prSearch);
      await page.keyboard.press("Escape");
      await waitFor("Escape to leave from the search box", () => page.locator("#chat").isVisible());
    } else await leavePulls();
    await openPulls();
    check((await page.inputValue(prSearch)) === "zzqx", "the search wasn't remembered");
    await waitFor("its answer", async () => (await text("#page-view .pr-empty h2")) === "Nothing matches “zzqx”", 10000);
    await press(page.locator('#page-view [data-pr="clear"]'));
    await waitFor("every row", async () => (await prNumbers()).length === 3, 15000);
    await leavePulls();
  });

  await step("follow: Pull requests: a failed load offers Retry, a failed refresh warns below the rows it kept, and no project offers Add project", async () => {
    await openPulls();
    await waitFor("the rows", async () => (await prNumbers()).length === 3, 15000);
    await page.fill(prSearch, "failhard");
    await waitFor("the warning", async () => /HTTP 502.*Showing the last pull requests loaded\.$/.test(await text("#page-view .pr-warn span")), 10000);
    check(await page.evaluate(() => !!document.querySelector("#page-view .pr-groups + .pr-warn")), "the warning isn't below the rows");
    check((await prNumbers()).length === 3, "the last rows went away");
    await page.evaluate(() => {
      pulls.rows = null;
      fetchPulls();
    });
    await waitFor("the failure", async () => (await text("#page-view .pr-empty.unavailable h2")) === "Could not load pull requests", 10000);
    check((await page.locator("#page-view .pr-empty.unavailable .media svg").count()) === 1 && (await text('#page-view .pr-empty.unavailable [data-pr="refresh"]')) === "Retry" && (await page.locator('#page-view .pr-empty.unavailable [data-pr="refresh"] svg').count()) === 1, "no pull request icon, or Retry without its icon");
    await page.fill(prSearch, "");
    await waitFor("every row", async () => (await prNumbers()).length === 3, 15000);
    await page.evaluate(() => {
      window.realOpenProjects = openProjects;
      openProjects = () => new Map();
      fetchPulls();
    });
    try {
      await waitFor("the empty workspace", async () => (await page.locator("#page-view .pr-empty h2").allTextContents()).join() === "No projects in this workspace");
      check((await text('#page-view .pr-empty button.primary[data-pr="add-project"]')) === "Add project" && (await iconIs('#page-view .pr-empty [data-pr="add-project"] svg', "plus")), "Add project isn't T3's primary button with a plus");
    } finally {
      await page.evaluate(() => {
        openProjects = window.realOpenProjects;
        fetchPulls();
      });
    }
    await waitFor("every row", async () => (await prNumbers()).length === 3, 15000);
    await leavePulls();
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
    await waitFor("listing", async () => (await page.locator("#projects .folders button", { has: page.locator('.name:text-is("alpha")') }).count()) === 1);
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
    for (const [field, button, error] of [["#linear-key", "connect-linear", "#linear-error"], ["#slack-token", "connect-slack", "#slack-error"], ["#vercel-key", "connect-vercel", "#vercel-error"], ["#github-token", "connect-github", "#github-error"]]) {
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
    await waitFor("connected badges", async () => (await page.locator('#settings-page .badge[data-on="true"]').count()) === 4);
    check(/…fake/.test(await text('#settings-page .card:has(.logo[data-tone=vercel]) .connected')), "the Vercel card lacks the key hint");
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

  await step("Workflows: the ＋ after the last step adds a step that is off", async () => {
    await action("open-add-menu");
    const offered = await page.locator("#settings-page .flow .add-menu button").allTextContents();
    check(offered.join() === "Ask a model before the session starts,Move the issue when pinned picks it up", `the menu offers ${offered}`);
    await press(page.locator('#settings-page .add-menu [data-step="picked"]'));
    check((await page.locator("#settings-page .flow .add-menu").count()) === 0, "the menu stays open");
    await page.fill('#settings-page .node-panel [data-bind="columns.picked"]', "Analyzing");
    await action("save-connector");
    await waitFor("Saved", async () => (await text("#save-connector")) === "Saved");
    const saved = (await hubCall({ cmd: "get_settings" })).connectors[0];
    check(saved.columns.picked === "Analyzing", JSON.stringify(saved.columns));
    return offered.join();
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

  await step("Workflows: a saved Slack workflow gets a model that classifies before its session", async () => {
    await action("open-add-menu");
    await press(page.locator('#settings-page .add-menu [data-step="ask"]'));
    check((await page.inputValue('#settings-page .node-panel [data-bind="ask.model"]')) === "typesafe-ai/jev", "the ask step lacks its default model");
    const x = async (id) => (await page.locator(`#settings-page .step[data-step="${id}"]`).boundingBox()).x;
    check((await x("slack-trigger")) < (await x("ask")) && (await x("ask")) < (await x("agent")), "the ask step is not between the trigger and the session");
    await action("save-connector");
    await waitFor("Saved", async () => (await text("#save-connector")) === "Saved");
    const saved = (await hubCall({ cmd: "get_settings" })).connectors.find((c) => c.trigger.kind === "slack-mention");
    check(saved.ask?.model === "typesafe-ai/jev" && saved.ask.questions.map((q) => q.name).join() === "kind,agent", JSON.stringify(saved.ask));
    check(saved.ask.questions[1].stopOn.join() === "no", "the agent question does not stop on no");
    return saved.ask.questions.map((q) => `${q.name}: ${q.choices.join("/")}`).join("; ");
  });

  await step("Workflows: a Slack message workflow routes its categories", async () => {
    await action("close-editor");
    await action("new-connector");
    await press(page.locator('#settings-page [data-action="pick-trigger"][data-kind="slack-message"]'));
    await page.fill("#settings-page .editor-bar .title", "Route support");
    await page.fill('#settings-page [data-bind="trigger.channel"]', "support");
    await press(page.locator('#settings-page .step[data-step="decide"] .node'));
    const names = await page.locator("#settings-page .category input.name").evaluateAll((els) => els.map((el) => el.value));
    check(names.join() === "error,doubt,escalation,ignore", `the default categories are ${names}`);
    check((await page.inputValue('#settings-page [data-bind="trigger.decide.categories.2.mention"]')) === "prod-engg", "escalation does not mention prod-engg");
    await action("remove-category", 3);
    await page.fill('#settings-page [data-bind="trigger.decide.categories.1.description"]', "");
    await action("save-connector");
    await waitFor("the description error", async () => /“doubt”/.test(await text("#editor-error")));
    await page.fill('#settings-page [data-bind="trigger.decide.categories.1.description"]', "Someone asks a question");
    await page.selectOption('#settings-page [data-bind="trigger.decide.categories.0.workflow"]', { label: "＋ New workflow for this category" });
    await waitFor("the new workflow on the error branch", async () => (await text('#settings-page .branch[data-branch="route"] b')) === "Route support: error");
    const ports = await page.locator("#settings-page .flow .port-label").allTextContents();
    const branches = await page.locator("#settings-page .flow .branch b").allTextContents();
    check(ports.join() === "error,doubt,escalation", `branch labels: ${ports}`);
    check(branches.join(" | ") === "Route support: error | Stop | Mention @prod-engg", `branch nodes: ${branches.join(" | ")}`);
    await action("save-connector");
    await waitFor("Saved", async () => (await text("#save-connector")) === "Saved");
    const saved = (await hubCall({ cmd: "get_settings" })).connectors;
    const router = saved.find((k) => k.trigger.kind === "slack-message");
    const called = saved.find((k) => k.trigger.kind === "called");
    const [error, doubt, escalation] = router?.trigger.decide.categories ?? [];
    check(called && error?.workflow === called.id && doubt.workflow === null, `routes: ${JSON.stringify(router?.trigger.decide.categories)}`);
    check(escalation.mention === "prod-engg" && error.mention === null, `mentions: ${JSON.stringify(router.trigger.decide.categories)}`);
    const over = await overflowing();
    check(!over.length, `overflowing: ${over}`);
    return branches.join(" | ");
  });

  const zoom = () => page.evaluate(() => Number(document.querySelector("#settings-page .flow").style.zoom));
  await step("Workflows: zoom in, out and fit, ⌘-wheel zooms and a drag pans", async () => {
    const opened = await zoom();
    await action("zoom-in");
    const zoomedIn = await zoom();
    await action("zoom-out");
    await action("zoom-out");
    const zoomedOut = await zoom();
    check(zoomedIn > opened && zoomedOut < zoomedIn - 0.15, `zoom ${opened} -> in ${zoomedIn} -> out ${zoomedOut}`);
    await action("zoom-fit");
    const fits = await page.evaluate(() => {
      const canvas = document.querySelector("#settings-page .canvas").getBoundingClientRect();
      const flow = document.querySelector("#settings-page .flow").getBoundingClientRect();
      return flow.left >= canvas.left - 1 && flow.right <= canvas.right + 1 && flow.top >= canvas.top - 1 && flow.bottom <= canvas.bottom + 1;
    });
    check(fits || (await zoom()) === 0.5, `Fit leaves the flow outside the canvas at ${await zoom()}`);
    const fitted = await zoom();
    if (touch) return `${opened} -> ${zoomedIn} -> ${zoomedOut} -> fit ${fitted}`;
    const canvas = await rect("#settings-page .canvas");
    await page.mouse.move(canvas.left + canvas.width / 2, canvas.top + canvas.height / 2);
    await page.keyboard.down("Control");
    await page.mouse.wheel(0, -300);
    await page.keyboard.up("Control");
    await waitFor("the wheel to zoom", async () => (await zoom()) > fitted);
    for (let i = 0; i < 12 && !(await page.locator('#settings-page [data-action="zoom-in"]').isDisabled()); i++) await action("zoom-in");
    check((await zoom()) === 1.5, `zoom in stops at ${await zoom()}`);
    const ground = await page.evaluate(() => {
      const box = document.querySelector("#settings-page .canvas").getBoundingClientRect();
      for (let y = box.top + 20; y < box.bottom - 80; y += 20) {
        for (let x = box.left + 20; x < box.right - 20; x += 20) {
          if (!document.elementFromPoint(x, y)?.closest("button, .step, .branch, .zoom")) return [x, y];
        }
      }
    });
    const before = await page.evaluate(() => [document.querySelector("#settings-page .canvas").scrollLeft, document.querySelector("#settings-page .canvas").scrollTop]);
    await page.mouse.move(...ground);
    await page.mouse.down();
    await page.mouse.move(ground[0] - 120, ground[1] - 80, { steps: 6 });
    await page.mouse.up();
    const after = await page.evaluate(() => [document.querySelector("#settings-page .canvas").scrollLeft, document.querySelector("#settings-page .canvas").scrollTop]);
    check(after[0] > before[0] && after[1] > before[1], `the drag scrolled ${before} to ${after}`);
    await action("zoom-fit");
    return `${opened} -> ${zoomedIn} -> ${zoomedOut} -> fit ${fitted}, panned ${before} -> ${after}`;
  });

  await step("Workflows: the ＋ after a category that stops picks its route", async () => {
    const ends = await page.locator('#settings-page .flow [data-action="choose-route"]').evaluateAll((els) => els.map((el) => el.dataset.category));
    check(ends.join() === "1,2", `open ends after categories ${ends}`);
    await action("choose-route");
    const focused = await page.evaluate(() => document.activeElement?.dataset.bind);
    check(focused === "trigger.decide.categories.1.workflow", `the ＋ focused ${focused}`);
    return focused;
  });

  await step("Workflows: the unsure fallback offers categories that ping no one, follows a rename, saves and reloads", async () => {
    const fallback = '#settings-page [data-bind="trigger.decide.fallback"]';
    const offered = await page.locator(`${fallback} option`).evaluateAll((els) => els.map((el) => el.textContent));
    check(offered.join() === "Use its best guess,error,doubt", `the fallback offers ${offered}`);
    await page.selectOption(fallback, "doubt");
    await page.fill('#settings-page [data-bind="trigger.decide.categories.1.name"]', "question");
    check((await page.inputValue(fallback)) === "question", `after a rename the fallback is ${await page.inputValue(fallback)}`);
    await page.fill('#settings-page [data-bind="trigger.decide.categories.1.name"]', "doubt");
    await action("save-connector");
    await waitFor("Saved", async () => (await text("#save-connector")) === "Saved");
    const router = () => hubCall({ cmd: "get_settings" }).then((s) => s.connectors.find((k) => k.trigger.kind === "slack-message"));
    check((await router()).trigger.decide.fallback === "doubt", `the hub has the fallback ${(await router()).trigger.decide.fallback}`);
    await action("close-editor");
    await press(page.locator("#settings-page .flow-card .open").filter({ has: page.locator("b", { hasText: /^Route support$/ }) }));
    await press(page.locator('#settings-page .step[data-step="decide"] .node'));
    check((await page.inputValue(fallback)) === "doubt", `reopened, the fallback is ${await page.inputValue(fallback)}`);
    await page.fill('#settings-page [data-bind="trigger.decide.categories.1.mention"]', "support-team");
    const issue = await page.getAttribute('#settings-page .step[data-step="decide"] .warn', "title");
    check(/can't ping a group/.test(issue ?? ""), `the Decide step says ${issue}`);
    await action("save-connector");
    await waitFor("Saved", async () => (await text("#save-connector")) === "Saved");
    const pinged = (await router()).trigger.decide;
    check(pinged.fallback === null && pinged.categories[1].mention === "support-team", `saved ${JSON.stringify(pinged)}`);
    check((await page.inputValue(fallback)) === "", "the select still shows the cleared fallback");
    await page.fill('#settings-page [data-bind="trigger.decide.categories.1.mention"]', "");
    await action("save-connector");
    await waitFor("Saved", async () => (await text("#save-connector")) === "Saved");
    return offered.join();
  });

  await step("Workflows: the called workflow names the workflow that routes to it", async () => {
    await press(page.locator('#settings-page .branch[data-branch="route"] .node'));
    await action("open-route");
    await waitFor("the called workflow", async () => (await page.inputValue("#settings-page .editor-bar .title")) === "Route support: error");
    await press(page.locator('#settings-page .step[data-step="called-trigger"] .node'));
    const said = await text("#settings-page .node-panel .details .hint");
    check(/“Route support” sorts a message into “error”/.test(said), said);
    check((await page.locator('#settings-page .step[data-step="reply"]').count()) === 1, "the called workflow does not reply in the thread");
    return said;
  });

  await step("Workflows: delete them all", async () => {
    if ((await page.locator("#settings-page .editor-bar").count()) > 0) await action("close-editor");
    if ((await page.locator("#settings-page .editor-bar").count()) > 0) await action("close-editor");
    for (let left = (await hubCall({ cmd: "get_settings" })).connectors.length; left > 0; left--) {
      await press(page.locator("#settings-page .flow-card .open").first());
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
    for (const name of ["disconnect-linear", "disconnect-slack", "disconnect-vercel", "disconnect-github"]) {
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

  let spareTab = null;
  await step("Accounts: Settings' nav opens Accounts, where an added account lists signed out", async () => {
    await settingsPage("accounts");
    check((await page.locator('#settings-nav [data-page="accounts"] svg').count()) === 1, "the Accounts link has no icon");
    await page.waitForSelector("#account-list .card", { timeout: 15000 });
    await page.fill(".add-account [name=name]", "Spare");
    await press(page.locator(".add-account .primary"));
    await page.waitForSelector('#account-list .card[data-account="spare"]');
    const card = (await text('#account-list .card[data-account="spare"]')).replace(/\s+/g, " ");
    check(/Not signed in/.test(card), `Spare reads: ${card}`);
    const over = await overflowing();
    check(!over.length, `overflowing: ${over}`);
    return card;
  });

  await step("Accounts: Sign in runs the login in its own tab, which the app view leaves to the terminal", async () => {
    await press(page.locator('#account-list .card[data-account="spare"] [data-action="sign-in-account"]'));
    await waitFor("the sign-in tab", () => page.evaluate(() => active?.view.program === "sign-in"), 10000);
    await waitFor("Spare signed in", () => page.evaluate(() => machines.get(active.machine).accounts?.find((a) => a.id === "spare")?.signed_in), 15000);
    await tapOrClick("#mode-gui");
    await waitFor("the app view", () => page.locator("#chat").isVisible());
    const said = await text("#thread .blank p");
    check(said === "This sign-in runs in the terminal only.", `the thread says "${said}"`);
    check(!(await page.locator("#composer").isVisible()), "the composer of another thread shows");
    check(!(await page.locator("#pr-chip").isVisible()), "a PR chip shows");
    await page.evaluate(() => closeTab(active));
    return said;
  });

  await step("Accounts: a usage limit offers Continue under Spare, which resumes the thread there", async () => {
    const machine = await page.evaluate(() => [...machines.keys()].find((k) => machines.get(k).view.online && machines.get(k).view.name === "sweep-node"));
    spareTab = await page.evaluate(async ([m, p]) => (await openTab(m, p)).key, [machine, path.join(HOME, "work/alpha")]);
    await waitFor("claude running", () => page.evaluate((k) => active?.key === k && !!active.view.running, spareTab), 15000);
    await page.evaluate(() => callOn(active.machine, { cmd: "prompt", tab: active.view.id, text: "/limit" }));
    await page.waitForSelector("#limit-note:not([hidden]) [data-switch-account=spare]", { timeout: 15000 });
    const note = await text("#limit-note");
    check(/hit its usage limit/.test(note), `the note reads "${note}"`);
    // The phone's sidebar sheet, open from the step before, covers the composer's note.
    if (touch) await hideSidebar();
    await press(page.locator("#limit-note [data-switch-account=spare]"));
    await waitFor("the thread under Spare", () => page.evaluate(() => active?.view.account === "spare" && !active.view.resume_at), 15000);
    check(await page.locator("#limit-note").isHidden(), "the limit note stayed");
    const session = await page.evaluate(() => active.view.session);
    const projects = path.join(DIR, "node/accounts/spare/projects");
    check(fs.readdirSync(projects).some((d) => fs.existsSync(path.join(projects, d, `${session}.jsonl`))), `the transcript ${session} is not in Spare's home`);
    await showSidebar();
    const badge = await text(`${rowSel(spareTab)} .account`);
    check(badge === "Spare", `the row's account badge reads "${badge}"`);
    await waitTerm(new RegExp(`account spare, resuming ${session}`), 10000);
    return note;
  });

  const accountsInApp = async () => {
    await showSidebar();
    await tapOrClick("#gui-settings");
    await tapOrClick('#settings-nav [data-page="accounts"]');
  };
  await step("Accounts: Remove waits for the thread to leave Spare, which its row menu moves back", async () => {
    await accountsInApp();
    await page.waitForSelector('#account-list .card[data-account="spare"]', { timeout: 15000 });
    await action("remove-account", await page.locator('#settings-page [data-action="remove-account"]').count() - 1);
    await action("confirm-remove-account");
    await waitFor("the refusal", async () => /close the tabs that run under spare/.test(await text("#account-list .error")));
    const refused = await text("#account-list .error");
    await tapOrClick("#settings-back");
    await rowMenu(rowSel(spareTab));
    await menuPick("Move to account");
    await menuPick("Move to Claude Code");
    await waitFor("the thread under the built-in account", () => page.evaluate((k) => tabByKey(k)?.view.account === "claude", spareTab), 15000);
    await accountsInApp();
    await page.waitForSelector('#account-list .card[data-account="spare"]', { timeout: 15000 });
    await action("remove-account", await page.locator('#settings-page [data-action="remove-account"]').count() - 1);
    await action("confirm-remove-account");
    await page.waitForSelector('#account-list .card[data-account="spare"]', { state: "detached" });
    await tapOrClick("#settings-back");
    await page.evaluate((k) => closeTab(tabByKey(k)), spareTab);
    await showSidebar();
    await tapOrClick("#mode-terminal");
    await waitFor("the tab bar", () => page.locator("#tab-bar").isVisible());
    return refused;
  });

  await step("Settings at phone width: nav and pages fit", async () => {
    for (const name of ["integrations", "accounts", "projects", "connectors", "account", "machines"]) {
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
