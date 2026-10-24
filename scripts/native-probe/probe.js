// Evaluated inside the app's own window by a debug build (see run_probe in src/main.rs).
// Each check posts its result to the server run.sh started; run.sh reads them back.
(async () => {
  const base = "http://127.0.0.1:__PORT__";
  const report = (check, ok, seen) => fetch(`${base}/report`, { method: "POST", body: JSON.stringify({ check, ok, seen }) });
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const win = TAURI.window.getCurrentWindow();
  const size = async () => {
    const s = (await win.innerSize()).toLogical(await win.scaleFactor());
    return { width: Math.round(s.width), height: Math.round(s.height) };
  };
  const roomy = (s) => s.width >= Math.min(1180, screen.availWidth - 40) && s.height >= Math.min(760, screen.availHeight - 40);
  const step = async (check, run) => {
    try {
      const [ok, seen] = await run();
      await report(check, ok, seen);
    } catch (e) {
      await report(check, false, String(e?.stack ?? e));
    }
  };

  if (__PASS__ === 2) {
    await step("a relaunch in the app view opens at the app view's size", async () => {
      await sleep(500);
      const now = await size();
      return [guiOn() && roomy(now) && now.width === __GUI_WIDTH__ && now.height === __GUI_HEIGHT__, { mode: document.body.dataset.mode, now }];
    });
    return fetch(`${base}/done`, { method: "POST" });
  }

  await step("a tab opens in the repository", async () => {
    const tab = await openTab(LOCAL, "__REPO__");
    await sleep(1500);
    return [!!tab?.view.running, { cwd: tab?.view.cwd, running: tab?.view.running }];
  });

  await step("switching views resizes the window and switching back restores it", async () => {
    setMode("terminal");
    await sleep(800);
    const terminal = await size();
    setMode("gui");
    await sleep(800);
    const gui = await size();
    setMode("terminal");
    await sleep(800);
    const back = await size();
    setMode("gui");
    await sleep(800);
    const again = await size();
    const ok = roomy(gui) && back.width === terminal.width && back.height === terminal.height && again.width === gui.width && again.height === gui.height;
    return [ok, { terminal, gui, back, again, screen: { width: screen.availWidth, height: screen.availHeight } }];
  });

  await step("the browser panel shows a localhost page", async () => {
    openPanel("browser");
    setBrowserUrl(`${base}/frame`);
    renderBrowser();
    let ran = false;
    for (let i = 0; i < 20 && !ran; i++) {
      await sleep(250);
      ran = await (await fetch(`${base}/seen?what=frame-ran`)).json();
    }
    const rect = document.querySelector("#panel iframe").getBoundingClientRect();
    return [ran && rect.width > 100 && rect.height > 100, { ran, width: Math.round(rect.width), height: Math.round(rect.height) }];
  });

  await step("opening a link outside reaches the system browser", async () => {
    openOutside(`${base}/opened`);
    let hit = false;
    for (let i = 0; i < 20 && !hit; i++) {
      await sleep(500);
      hit = await (await fetch(`${base}/seen?what=opened`)).json();
    }
    return [hit, { hit }];
  });

  await step("a slow call leaves the main thread free", async () => {
    const tab = active;
    const t0 = performance.now();
    const pr = callOn(tab.machine, { cmd: "pull_request", tab: tab.view.id, pr: "4" }).then((v) => ({ v, ms: performance.now() - t0 }), (e) => ({ e: String(e), ms: performance.now() - t0 }));
    await sleep(50);
    const t1 = performance.now();
    await callOn(tab.machine, { cmd: "resize_pty", tab: tab.view.id, cols: 80, rows: 24 });
    const fast = performance.now() - t1;
    const slow = await pr;
    const ok = slow.v?.number === 4 && fast < 150 && slow.ms > fast * 3;
    return [ok, { prNumber: slow.v?.number, prTitle: slow.v?.title, error: slow.e, prMs: Math.round(slow.ms), fastCallMs: Math.round(fast) }];
  });

  await fetch(`${base}/done`, { method: "POST" });
})();
