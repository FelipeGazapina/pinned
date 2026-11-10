// The RAM bench's walk through the app, the same in the web app and the desktop app. run.js
// evaluates this file in the page: it leaves `ramBench` on the window.
// `boot` waits for every saved session to be open and running, with the first thread painted.
// `walk` is a day of use: it opens every thread once, shows three terminals, opens each page of
// the sidebar's footer, and comes back to the first thread. With `memory`, it first saves one
// project memory, which loads the vector model as the first prompt in a project with memories does.
window.ramBench = (() => {
  const errors = [];
  addEventListener("error", (e) => errors.push(String(e.message)));
  addEventListener("unhandledrejection", (e) => errors.push(String(e.reason)));
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const frame = () => new Promise((resolve) => requestAnimationFrame(() => resolve()));
  const until = async (test, ms, what) => {
    for (const end = Date.now() + ms; Date.now() < end; await sleep(50)) if (test()) return;
    throw new Error(`timed out waiting for ${what}`);
  };
  const painted = async (tab) => {
    const t = threadOf(tab);
    await until(() => t.loaded && !t.fetching, 120000, `the thread of ${tab.key}`);
    await frame();
    await frame();
  };
  const counts = () => ({
    tabs: tabs.length,
    loaded: tabs.filter((t) => threadOf(t).loaded).length,
    items: tabs.reduce((n, t) => n + threadOf(t).items.length, 0),
    nodes: document.getElementsByTagName("*").length,
    errors,
  });

  async function boot(sessions) {
    await until(() => tabs.length >= sessions, 180000, `${sessions} tabs`);
    await until(() => tabs.every((t) => t.view.running), 180000, "every session to run");
    if (!guiOn()) setMode("gui");
    activate(tabs[0]);
    await painted(tabs[0]);
    return counts();
  }

  /** The walk's parts in order, so a run can read memory between them. */
  const steps = {
    async memory() {
      const machine = tabs[0].machine;
      const path = threadOf(tabs[0]).workspace?.project ?? tabs[0].view.cwd;
      let vectors = "loading";
      await callOn(machine, { cmd: "memory_save", path, title: "Package manager", text: "Use pnpm, not npm. The lockfile is pnpm-lock.yaml." });
      await until(() => (callOn(machine, { cmd: "memory_status", path }).then((s) => (vectors = s.vectors)), vectors === "ready"), 120000, "the vector model");
      return { vectors };
    },
    async threads(settleMs) {
      let visited = 0;
      for (const tab of [...tabs]) {
        activate(tab);
        await painted(tab);
        await sleep(settleMs);
        visited++;
      }
      return { visited };
    },
    async terminals(settleMs) {
      setMode("terminal");
      for (const tab of tabs.slice(0, 3)) {
        activate(tab);
        await sleep(settleMs);
      }
      setMode("gui");
    },
    async pages() {
      for (const page of ["usage", "pull-requests", "skills", "taste"]) {
        openPage(page);
        await sleep(1500);
      }
      closePage();
    },
    async home() {
      activate(tabs[0]);
      await painted(tabs[0]);
    },
  };

  /** Opens one thread and says what it drew, for a run that reads memory thread by thread. */
  async function visit(index, settleMs) {
    const tab = tabs[index];
    activate(tab);
    await painted(tab);
    await sleep(settleMs);
    const count = (selector) => threadView.querySelectorAll(selector).length;
    // What WebKit composites: an element that scrolls gets a layer of its own.
    const scrollers = {};
    for (const el of threadView.querySelectorAll("*")) {
      const style = getComputedStyle(el);
      const scrolls = (/auto|scroll/.test(style.overflowX) && el.scrollWidth > el.clientWidth) || (/auto|scroll/.test(style.overflowY) && el.scrollHeight > el.clientHeight);
      if (scrolls) scrollers[`${el.tagName.toLowerCase()}.${el.className}`] = (scrollers[`${el.tagName.toLowerCase()}.${el.className}`] ?? 0) + 1;
    }
    return { items: threadOf(tab).items.length, nodes: count("*"), images: count("img"), blocks: count("pre"), scrollers };
  }

  async function step(name, settleMs) {
    return { ...(await steps[name](settleMs)), ...counts() };
  }

  async function walk(settleMs, memory) {
    const done = { vectors: "not asked" };
    for (const name of Object.keys(steps)) if (name !== "memory" || memory) Object.assign(done, await steps[name](settleMs));
    return { ...done, ...counts() };
  }

  return { boot, walk, step, visit };
})();
