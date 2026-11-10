// The RAM bench's walk through the app, the same in the web app and the desktop app. run.js
// evaluates this file in the page: it leaves `ramBench.boot` and `ramBench.walk` on the window.
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

  async function walk(settleMs, memory) {
    let vectors = "not asked";
    if (memory) {
      const machine = tabs[0].machine;
      const path = threadOf(tabs[0]).workspace?.project ?? tabs[0].view.cwd;
      await callOn(machine, { cmd: "memory_save", path, title: "Package manager", text: "Use pnpm, not npm. The lockfile is pnpm-lock.yaml." });
      await until(() => (callOn(machine, { cmd: "memory_status", path }).then((s) => (vectors = s.vectors)), vectors === "ready"), 120000, "the vector model");
    }
    let visited = 0;
    for (const tab of [...tabs]) {
      activate(tab);
      await painted(tab);
      await sleep(settleMs);
      visited++;
    }
    setMode("terminal");
    for (const tab of tabs.slice(0, 3)) {
      activate(tab);
      await sleep(settleMs);
    }
    setMode("gui");
    for (const page of ["usage", "pull-requests", "skills", "taste"]) {
      openPage(page);
      await sleep(1500);
    }
    closePage();
    activate(tabs[0]);
    await painted(tabs[0]);
    return { visited, vectors, ...counts() };
  }

  return { boot, walk };
})();
