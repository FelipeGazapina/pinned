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

  const page = () => shownSurface(active);
  const until = async (test, ms = 8000) => {
    for (let t = 0; t < ms && !(await test()); t += 100) await sleep(100);
    return test();
  };
  await step("the browser panel shows a localhost page in a native webview", async () => {
    openPanel("browser");
    setBrowserUrl(`${base}/frame`);
    renderPanel();
    let ran = false;
    for (let i = 0; i < 40 && !ran; i++) {
      await sleep(250);
      ran = await (await fetch(`${base}/seen?what=frame-ran`)).json();
    }
    const rect = document.querySelector("#panel .viewport .page").getBoundingClientRect();
    const fills = document.documentElement.clientWidth === window.innerWidth && Math.abs(window.innerWidth - (await size()).width) <= 1;
    return [ran && rect.width > 100 && rect.height > 100 && placedPage === page().id && !document.querySelector("#panel iframe") && fills, { ran, width: Math.round(rect.width), height: Math.round(rect.height), placed: placedPage, fills }];
  });

  await step("the browser tab reads the page's title", async () => {
    await until(() => document.querySelector("#panel-tabs .ptab.active")?.textContent.trim() === "pinned probe page");
    return [document.querySelector("#panel-tabs .ptab.active")?.textContent.trim() === "pinned probe page", { tab: document.querySelector("#panel-tabs .ptab.active")?.textContent.trim(), title: page().title }];
  });

  await step("Back and Forward step through the page's own history", async () => {
    await invoke("browser_eval", { id: page().id, js: "(() => { document.querySelector('a').click(); return true; })()" });
    await until(() => page().url.endsWith("/two") && page().back);
    const after = { url: page().url, back: page().back, title: page().title };
    document.querySelector('#panel-body [data-nav="back"]').click();
    await until(() => page().url.endsWith("/frame") && page().forward);
    const backed = { url: page().url, forward: page().forward };
    document.querySelector('#panel-body [data-nav="forward"]').click();
    await until(() => page().url.endsWith("/two"));
    const forwarded = { url: page().url };
    return [after.url.endsWith("/two") && after.title === "probe two" && backed.url.endsWith("/frame") && backed.forward && forwarded.url.endsWith("/two"), { after, backed, forwarded }];
  });

  await step("devtools open and close for the page", async () => {
    const width = () => invoke("browser_eval", { id: page().id, js: "innerWidth" });
    const before = await width();
    toggleDevtools(page());
    const opened = await until(() => page().shownDevtools === true, 5000);
    await sleep(1500);
    const expanded = document.body.dataset.view === "expanded";
    toggleDevtools(page());
    const closed = await until(() => page().shownDevtools === false, 5000);
    await sleep(1200);
    const after = await width();
    return [opened && closed && expanded && after === before, { opened, closed, expanded, before, after }];
  });

  await step("zoom, a screenshot, and hiding under a menu", async () => {
    const width = () => invoke("browser_eval", { id: page().id, js: "innerWidth" });
    const before = await width();
    zoomBrowser(page(), 1);
    await sleep(600);
    const zoomed = await width();
    zoomBrowser(page(), 0);
    const png = new Uint8Array(await invoke("browser_snapshot", { id: page().id }));
    const signature = [...png.slice(1, 4)].map((c) => String.fromCharCode(c)).join("");
    document.querySelector('#panel-body [data-nav="more"]').click();
    await sleep(400);
    const hidden = placedPage === null;
    closeMenu();
    await sleep(400);
    const back = placedPage === page().id;
    return [Math.abs(zoomed * 2 - before) <= 2 && signature === "PNG" && png.length > 1000 && hidden && back, { before, zoomed, png: png.length, signature, hidden, back }];
  });

  await step("appearance, cookies, mute, a separate window and the floating preview", async () => {
    const run = (js) => invoke("browser_eval", { id: page().id, js });
    const act = (action) => invoke("browser_do", { id: page().id, action });
    await act({ do: "appearance", mode: "dark" });
    await sleep(300);
    const dark = await run("matchMedia('(prefers-color-scheme: dark)').matches");
    await act({ do: "appearance", mode: "light" });
    await sleep(300);
    const light = !(await run("matchMedia('(prefers-color-scheme: dark)').matches"));
    await run("(() => { document.cookie = 'probe=1; max-age=600'; return document.cookie; })()");
    await act({ do: "clear-cookies" });
    await sleep(500);
    await act({ do: "reload" });
    await until(async () => !page().loading && (await run("document.readyState")) === "complete");
    const cookies = await run("document.cookie");
    await act({ do: "mute", on: true });
    await invoke("browser_window", { url: `${base}/frame` });
    await sleep(1500);
    const windows = (await TAURI.webviewWindow.getAllWebviewWindows()).map((w) => w.label);
    await setView("expanded");
    await sleep(500);
    togglePip(page());
    await sleep(500);
    const pip = document.querySelector("#preview-pip .page")?.getBoundingClientRect();
    const floating = placedPage === page().id && pip?.width > 100 && !!document.querySelector("#panel-body .viewport.floating");
    togglePip(page());
    await sleep(500);
    const returned = placedPage === page().id && !!document.querySelector("#panel .viewport .page");
    return [dark && light && cookies === "" && windows.some((l) => l.startsWith("preview-window-")) && floating && returned, { dark, light, cookies, windows, floating, returned }];
  });

  await step("an annotation picks an element into the prompt with its picture", async () => {
    prompt.value = "";
    const s = page();
    pickElement(s);
    await until(() => s.picking);
    await sleep(500);
    await invoke("browser_eval", { id: s.id, js: `(() => {
      const h1 = document.querySelector("h1").getBoundingClientRect();
      const at = { clientX: h1.left + 5, clientY: h1.top + 5, bubbles: true };
      const layer = document.documentElement.lastElementChild;
      layer.dispatchEvent(new PointerEvent("pointerdown", at));
      layer.dispatchEvent(new PointerEvent("pointerup", at));
      return true;
    })()` });
    await until(() => !s.picking && /\[127\.0\.0\.1:\d+ h1 /.test(prompt.value), 6000);
    await until(() => (threadOf(active).attachments ?? []).some((f) => /annotation-.*\.png$/.test(f.path)), 8000);
    const attached = (threadOf(active).attachments ?? []).map((f) => f.name);
    return [/\[127\.0\.0\.1:\d+ h1 "two"\]/.test(prompt.value) && attached.some((n) => /^annotation-.*\.png$/.test(n)), { prompt: prompt.value, attached }];
  });

  await step("a shift-click on Capture records the page to Downloads", async () => {
    const s = page();
    await capture(s, true);
    await sleep(1500);
    const recordingNow = recording?.id === s.id;
    await stopRecording();
    await sleep(800);
    const note = document.getElementById("panel-note")?.textContent ?? "";
    return [recordingNow && /Recording saved/.test(note), { recordingNow, note }];
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

  await step("the app view's slow calls answer in the window", async () => {
    const tab = active;
    const call = (c) => callOn(tab.machine, { tab: tab.view.id, ...c }).catch((e) => ({ error: String(e) }));
    const [diff, ports, editors, commands, files, branches] = await Promise.all(
      [{ cmd: "diff", scope: "base" }, { cmd: "ports" }, { cmd: "editors" }, { cmd: "commands" }, { cmd: "files" }, { cmd: "branches" }].map(call),
    );
    const seen = { diff: diff.error ?? Object.keys(diff), ports: ports.error ?? ports.length, editors: editors.error ?? editors.length, commands: commands.error ?? commands.length, files: files.error ?? files.length, branches: branches.error ?? branches.refs?.map((r) => r.name) };
    const ok = !diff.error && Array.isArray(ports) && Array.isArray(editors) && Array.isArray(commands) && files.includes?.("README.md") && branches.refs?.length > 0;
    return [ok, seen];
  });

  await fetch(`${base}/done`, { method: "POST" });
})();
