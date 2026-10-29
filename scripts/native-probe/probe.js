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

  await step("the window's native appearance follows the app view's Light and Dark, and System or the terminal view hand it back", async () => {
    const seen = async () => ({ theme: await win.theme(), dark: matchMedia("(prefers-color-scheme: dark)").matches });
    const initial = await seen();
    setAppearanceMode("light");
    await sleep(500);
    const light = await seen();
    setAppearanceMode("dark");
    await sleep(500);
    const dark = await seen();
    setMode("terminal");
    await sleep(800);
    const terminal = await seen();
    setMode("gui");
    await sleep(800);
    const again = await seen();
    setAppearanceMode("system");
    await sleep(500);
    const system = await seen();
    const ok = light.theme === "light" && !light.dark && dark.theme === "dark" && dark.dark && terminal.theme === initial.theme && again.theme === "dark" && system.theme === initial.theme && system.dark === initial.dark;
    return [ok, { initial, light, dark, terminal, again, system }];
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

  await step("appearance as radio items, cookies, mute, a separate window that the More menu closes, and the floating preview", async () => {
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
    toggleMute(page());
    await sleep(300);
    const muted = document.querySelector('#panel-tabs .ptab.active .audio')?.title === "Unmute tab";
    toggleMute(page());
    await act({ do: "hard-reload" });
    await act({ do: "clear-cache" });
    const appearance = menuHtml(browserMenu(page()).find((m) => m.label === "Appearance").sub);
    const radios = (appearance.match(/class="i radio/g) ?? []).length === 3 && !appearance.includes('class="i check"');
    await invoke("browser_window", { id: page().id, url: `${base}/frame` });
    await until(() => page().window === true, 5000);
    const windows = (await TAURI.webviewWindow.getAllWebviewWindows()).map((w) => w.label);
    const label = browserMenu(page()).find((m) => /separate preview window/.test(m.label ?? "")).label;
    browserMenu(page()).find((m) => m.label === label).run();
    await until(() => page().window === false, 5000);
    const closed = !(await TAURI.webviewWindow.getAllWebviewWindows()).some((w) => w.label.startsWith("preview-window-"));
    const reopen = browserMenu(page()).find((m) => /separate preview window/.test(m.label ?? "")).label;
    await setView("expanded");
    await sleep(500);
    togglePip(page());
    await sleep(500);
    const pip = document.querySelector("#preview-pip .page")?.getBoundingClientRect();
    const floating = placedPage === page().id && pip?.width > 100 && !!document.querySelector("#panel-body .viewport.floating");
    togglePip(page());
    await sleep(500);
    const returned = placedPage === page().id && !!document.querySelector("#panel .viewport .page");
    const ok = dark && light && radios && cookies === "" && muted && windows.some((l) => l.startsWith("preview-window-")) && label === "Close separate preview window" && closed && reopen === "Open separate preview window" && floating && returned;
    return [ok, { dark, light, radios, cookies, muted, windows, label, closed, reopen, floating, returned }];
  });

  await step("⌘L, ⌘= and the app's own shortcuts work while the page has focus", async () => {
    const s = page();
    await invoke("browser_debug_key", { id: s.id, key: "l" });
    const focused = await until(() => !!document.activeElement?.matches?.(".url-bar input"), 3000);
    await invoke("browser_debug_key", { id: s.id, key: "=" });
    const zoomed = await until(() => s.zoom === 1.1, 3000);
    zoomBrowser(s, 0);
    const sidebar = document.body.dataset.sidebar;
    await invoke("browser_debug_key", { id: s.id, key: "b" });
    const toggled = await until(() => document.body.dataset.sidebar !== sidebar, 3000);
    if (toggled) toggleSidebar();
    return [focused && zoomed && toggled, { focused, zoomed, toggled }];
  });

  await step("an annotation opens T3's editor in the page, and Attach puts its comment, style change and picture in the prompt", async () => {
    prompt.value = "";
    const s = page();
    const run = (js) => invoke("browser_eval", { id: s.id, js });
    const editor = `[...document.documentElement.children].find((e) => e.shadowRoot?.querySelector(".layer")).shadowRoot`;
    pickElement(s);
    await until(() => s.picking);
    await sleep(500);
    const opened = await run(`(() => {
      const root = ${editor};
      const h1 = document.querySelector("h1").getBoundingClientRect();
      const at = { clientX: h1.left + 5, clientY: h1.top + 5, bubbles: true };
      const layer = root.querySelector(".layer");
      layer.dispatchEvent(new PointerEvent("pointerdown", at));
      layer.dispatchEvent(new PointerEvent("pointerup", at));
      return { card: !root.querySelector(".card").hidden, chip: root.querySelector(".chip")?.textContent, placeholder: root.querySelector("textarea").placeholder, tools: [...root.querySelectorAll(".bar button")].map((b) => b.textContent).join(" ") };
    })()`);
    const live = await run(`(() => {
      const root = ${editor};
      root.querySelector("textarea").value = "Make the heading red";
      root.querySelector(".adjust").click();
      const color = root.querySelector('input[type=text][data-prop="color"]');
      color.value = "rgb(255, 0, 0)";
      color.dispatchEvent(new Event("input", { bubbles: true }));
      return getComputedStyle(document.querySelector("h1")).color;
    })()`);
    await run(`(() => { ${editor}.querySelector(".attach").click(); return true; })()`);
    await until(() => !s.picking && prompt.value.includes("</preview_annotation>"), 6000);
    await until(() => (threadOf(active).attachments ?? []).some((f) => /preview-annotation-.*\.png$/.test(f.path)), 8000);
    const attached = (threadOf(active).attachments ?? []).map((f) => f.name);
    await sleep(300);
    const after = await run(`({ color: getComputedStyle(document.querySelector("h1")).color, editor: !!window.__pinnedPicker })`);
    const text = prompt.value;
    const ok = opened.card && opened.chip === "h1" && opened.placeholder === "Describe the change…" && opened.tools === "Select Region Draw Erase" && live === "rgb(255, 0, 0)"
      && text.startsWith("[Preview annotation: Make the heading red]") && /Comment: Make the heading red\n/.test(text) && /- color: rgb\(0, 0, 0\) → rgb\(255, 0, 0\)/.test(text) && /selector: h1\n/.test(text)
      && attached.some((n) => /^preview-annotation-.*\.png$/.test(n)) && after.color === "rgb(0, 0, 0)" && !after.editor;
    prompt.value = "";
    return [ok, { opened, live, prompt: text, attached, after }];
  });

  await step("a shift-click on Capture records the page to Downloads, drawing its key and mouse presses", async () => {
    const s = page();
    save("gui-browser-rec-keys", "true");
    save("gui-browser-rec-mouse", "true");
    await capture(s, true);
    await sleep(800);
    const recordingNow = recording?.id === s.id;
    await invoke("browser_eval", { id: s.id, js: `(() => {
      dispatchEvent(new KeyboardEvent("keydown", { key: "k", metaKey: true }));
      dispatchEvent(new PointerEvent("pointerdown", { clientX: 40, clientY: 40, pointerType: "mouse" }));
      return true;
    })()` });
    await until(() => recording?.input.key && recording?.input.press, 3000);
    const input = JSON.parse(JSON.stringify(recording?.input ?? {}));
    const canvas = Object.assign(document.createElement("canvas"), { width: 400, height: 300 });
    recordingOverlay(canvas.getContext("2d"), recording.input, 1, performance.now());
    const alpha = (x, y) => canvas.getContext("2d").getImageData(x, y, 1, 1).data[3];
    const drawn = { key: alpha(200, 250), ring: alpha(60, 40) };
    await stopRecording();
    await sleep(800);
    save("gui-browser-rec-keys", "");
    save("gui-browser-rec-mouse", "");
    const note = document.getElementById("panel-note")?.textContent ?? "";
    const ok = recordingNow && input.key?.label === "⌘K" && input.press?.x === 40 && drawn.key > 0 && drawn.ring > 0 && /Recording saved/.test(note);
    return [ok, { recordingNow, input, drawn, note }];
  });

  await step("Refresh spins while a native page loads and starts it over when clicked", async () => {
    const s = page();
    go(s, `${base}/slow`);
    await until(() => s.loading && !!document.querySelector('#panel-body [data-nav="reload"].spinning'), 4000);
    const button = document.querySelector('#panel-body [data-nav="reload"]');
    const spinning = { spins: button?.classList.contains("spinning"), label: button?.getAttribute("aria-label") };
    button.click();
    await sleep(300);
    const still = s.loading;
    await until(() => !s.loading && s.title === "probe slow", 8000);
    const done = { loading: s.loading, title: s.title, spins: !!document.querySelector('#panel-body [data-nav="reload"].spinning') };
    return [spinning.spins && spinning.label === "Stop" && still && done.title === "probe slow" && !done.spins, { spinning, still, done }];
  });

  await step("an unreachable address and a downloaded file show T3's screens in place of the page", async () => {
    const s = page();
    const screen = () => document.querySelector("#panel-body .page-failure h1")?.textContent;
    go(s, "http://127.0.0.1:9/");
    await until(() => s.failure, 10000);
    await sleep(300);
    const unreachable = { ...s.failure, screen: screen(), placed: placedPage };
    go(s, `${base}/report.zip`);
    await until(() => s.failure?.kind === "file", 10000);
    await sleep(300);
    const file = { ...s.failure, screen: screen(), placed: placedPage };
    go(s, `${base}/two`);
    await until(() => !s.failure && !s.loading && placedPage === s.id && s.title === "probe two", 8000);
    const back = { failure: s.failure, placed: placedPage, title: s.title };
    const ok = unreachable.kind === "unreachable" && unreachable.description === "ERR_CONNECTION_REFUSED" && unreachable.screen === "This site can’t be reached" && unreachable.placed === null
      && file.kind === "file" && file.name === "report.zip" && /Downloads/.test(file.path ?? "") && file.screen === "This file can’t be shown here" && file.placed === null && back.placed === s.id;
    return [ok, { unreachable, file, back }];
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

  await step("a file chip's menu reveals a file of this machine in Finder", async () => {
    threadView.insertAdjacentHTML("beforeend", fileChip("README.md"));
    const chip = threadView.querySelector('.file-chip[data-file-path="README.md"]');
    chip.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await sleep(200);
    const items = [...document.querySelectorAll("#side-menu button .label")].map((l) => l.textContent);
    closeSideMenu();
    const revealed = await openFileChipIn(chip, "reveal").then(() => true, (e) => String(e));
    chip.remove();
    return [items.includes("Reveal in Finder") && revealed === true, { items, revealed }];
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

  await step("the header title opens the thread's native menu under it, after the double-click wait", async () => {
    const shown = [];
    const real = TAURI.menu.Menu.new;
    TAURI.menu.Menu.new = async (options) => ({ popup: async (at) => shown.push({ items: options.items.length, at: at && { x: at.x, y: at.y }, ms: performance.now() }) });
    try {
      renderThread();
      const title = document.querySelector("#chat-head .crumb button.title");
      const r = title.getBoundingClientRect();
      const t0 = performance.now();
      title.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
      await sleep(250);
      const early = shown.length;
      await until(() => shown.length > 0, 2000);
      const [menu] = shown;
      const ok = early === 0 && !!menu?.at && Math.round(menu.at.x) === Math.round(r.left) && Math.round(menu.at.y) === Math.round(r.bottom + 4) && menu.ms - t0 >= 450 && document.getElementById("side-menu").hidden;
      return [ok, { early, menu: menu && { ...menu, ms: Math.round(menu.ms - t0) }, title: { left: r.left, bottom: r.bottom } }];
    } finally {
      TAURI.menu.Menu.new = real;
    }
  });

  await step("the sidebar's update control sits at the footer's end and checks for a new version", async () => {
    const b = document.getElementById("side-update");
    const footer = b.parentElement.getBoundingClientRect();
    const r = b.getBoundingClientRect();
    const placed = !b.hidden && Math.round(r.width) === 32 && Math.abs(r.right - (footer.right - 8)) <= 1;
    const before = { state: b.dataset.state, tip: b.dataset.tip };
    let checked = before.state !== "idle";
    if (before.state === "idle") {
      b.click();
      const busy = b.dataset.state === "checking" && b.dataset.tip === "Checking for updates…";
      checked = busy && (await until(() => b.dataset.state !== "checking", 20000));
    }
    const after = { state: b.dataset.state, tip: b.dataset.tip };
    const ok = placed && checked && (after.state === "idle" ? after.tip === "Check for updates" : /^Update .+ ready to download$/.test(after.tip));
    return [ok, { placed, width: r.width, right: r.right, footerRight: footer.right, before, after }];
  });


  await step("the desktop palette browses a local folder with Open in Finder, and Keybindings has no browser warning", async () => {
    openPalette({ mode: "add-project" });
    runPaletteItem(palette.shown.find((i) => i.title === "Local folder"));
    await sleep(800);
    const footer = document.querySelector("#palette footer [data-footer-act]")?.textContent ?? null;
    const folders = await callOn(LOCAL, { cmd: "search_threads", query: "zz" }).catch((e) => ({ error: String(e) }));
    closePalette();
    await openSettings("keybindings");
    await sleep(500);
    const note = !!document.querySelector("#settings-page h2 .kb-note");
    const rows = document.querySelectorAll("#keybindings .kb-row").length;
    await toggleSettings();
    return [footer === "Open in Finder" && !note && rows > 40 && Array.isArray(folders), { footer, note, rows, search: folders.error ?? folders.length }];
  });

  await step("Settings' font fields list the Mac's installed fonts, each drawn in its own face", async () => {
    if (!settingsOpen()) await toggleSettings();
    showPage("appearance");
    for (let i = 0; i < 40 && !document.querySelector("#look-sans-font[data-font-pick]"); i++) await sleep(100);
    const trigger = document.querySelector("#look-sans-font[data-font-pick]");
    trigger?.click();
    await sleep(200);
    const names = [...document.querySelectorAll("#font-picker [role=option] .name")].map((el) => el.textContent);
    const menlo = document.querySelector('#font-picker [data-font="Menlo"] .name');
    const seen = { status: installedFonts.status, families: installedFonts.families.length, listed: names.length, menloFace: menlo && getComputedStyle(menlo).fontFamily };
    closeFontPicker();
    await toggleSettings();
    return [!!trigger && installedFonts.families.includes("Menlo") && names.length > 10 && /^Menlo/.test(seen.menloFace ?? ""), seen];
  });

  await step("a diff file's name opens a native menu with Open, Reveal in Finder and Open with", async () => {
    const tab = active;
    // The clone's origin/HEAD is its own branch, so Automatic finds nothing to compare.
    setDiffState(tab, { scope: "base", base: "origin/main" });
    openSurface("diff");
    let title = null;
    for (let i = 0; i < 60 && !(title = panelBody.querySelector(".diff[data-comments] .file > header .path")); i++) await sleep(250);
    for (let i = 0; i < 20 && !editorsOn(tab.machine).length; i++) await sleep(250);
    const { Menu } = TAURI.menu;
    const make = Menu.new;
    let built = null;
    Menu.new = async (opts) => {
      const menu = await make.call(Menu, opts);
      menu.popup = async () => {};
      return (built = menu);
    };
    try {
      title?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 40, clientY: 40 }));
      for (let i = 0; i < 40 && !built; i++) await sleep(100);
    } finally {
      Menu.new = make;
    }
    const items = built ? await built.items() : [];
    const labels = await Promise.all(items.map((i) => i.text()));
    const withList = items[2]?.items ? await Promise.all((await items[2].items()).map((i) => i.text())) : [];
    const ok = labels.join("|") === "Open|Reveal in Finder|Open with" && withList.length > 0 && !withList.includes("Finder");
    return [ok, { file: title?.textContent, labels, withList }];
  });

  await fetch(`${base}/done`, { method: "POST" });
})();
