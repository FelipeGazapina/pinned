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
const { chromium, devices } = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const HOME = path.join(DIR, "home");

const PROFILES = {
  phone: { ...devices["Pixel 7"] },
  desktop: { viewport: { width: 1280, height: 800 }, hasTouch: false, isMobile: false },
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
  const profile = PROFILES[profileName];
  const touch = !!profile.hasTouch;
  const shots = path.join(DIR, "shots", profileName);
  fs.mkdirSync(shots, { recursive: true });
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ ...profile, permissions: ["clipboard-read", "clipboard-write"] });
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
    await page.screenshot({ path: path.join(shots, `${String(results.length + 1).padStart(2, "0")}-${name.replace(/\W+/g, "-")}.png`) }).catch(() => {});
    results.push({ step: name, verdict, note, seen: [...new Set(seen)] });
    console.log(`${profileName.padEnd(7)} ${verdict.padEnd(17)} ${name}${note ? `: ${note}` : ""}`);
  }

  await page.goto(HUB);

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

  await step("keys row: Esc Tab ↑ ↓ Ctrl-C Enter", async () => {
    if (!touch) return check(!(await page.locator("#keys").isVisible()), "the keys row shows on desktop");
    check(await page.locator("#keys").isVisible(), "the keys row is hidden on the phone");
    for (const key of ["Esc", "Tab", "↑", "↓", "Ctrl-C"]) await page.tap(`#keys button:text-is("${key}")`);
    await waitTerm(/\[esc\]\[tab\]\[up\]\[down\]\[ctrl-c\]/);
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
    return text("#status-text");
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
    check((await text("#tab-list .tab.active .name")).startsWith("⚡"), "the bypass tab has no ⚡");
  });

  await step("switch tabs", async () => {
    await press(page.locator("#tab-list .tab .name", { hasText: "alpha" }));
    check(/alpha/.test(await text("#title")), "tapping the alpha tab did not show it");
    await waitTerm(/work\/alpha/);
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
    await waitTerm(/fake claude in \S*\/sweep\/home$/m);
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
    check((await page.evaluate(() => navigator.clipboard.readText())) === command, "the clipboard lacks the command");
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
    return text("#login .error");
  });

  await step("a stale token at load shows the sign-in form", async () => {
    await page.evaluate(() => localStorage.setItem("hub-token", "stale"));
    await page.reload();
    await page.waitForSelector("#login", { state: "visible", timeout: 10000 });
    return text("#login .error");
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
