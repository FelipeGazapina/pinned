const GATEWAY_TIMES_OUT_TWICE_THEN_ANSWERS_SLOWLY = ["504:30000", "504:30000", "delay:30000"];
const GO_THREAD_VARIANTS = [
  { variant: "bypass-fast", bypass: true, first: [] },
  { variant: "bypass-slow", bypass: true, first: GATEWAY_TIMES_OUT_TWICE_THEN_ANSWERS_SLOWLY },
  { variant: "normal-fast", bypass: false, first: [] },
  { variant: "normal-slow", bypass: false, first: GATEWAY_TIMES_OUT_TWICE_THEN_ANSWERS_SLOWLY },
  { variant: "bypass-fast-preview", bypass: true, first: [], preview: true },
];

async function goThread(o) {
  o = { model: "deepseek-v4.1-flash", gapMs: 350, prompt: "check the sweep", deadlineMs: o.first.length ? 240000 : 90000, ...o };
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const until = async (what, test, ms) => {
    for (const end = Date.now() + ms; Date.now() < end; await sleep(100)) if (await test()) return;
    throw new Error(`timed out waiting for ${what}`);
  };
  const REQUIRED = ["gui.thinking", "dom.thinking", "gui.bash", "dom.bash", "tab.menu", "dom.menu", "phase.asking"];
  const result = { variant: o.variant, ok: false, failures: [], seen: {}, statuses: [], menus: [], errors: [] };
  let tab;
  try {
    await fetch(`${o.mock}/mock/script`, { method: "POST", body: JSON.stringify({ first: o.first, preview: o.preview, gapMs: o.gapMs }) });
    setMode("gui");
    tab = await openTab(o.machine, o.folder, o.bypass);
    result.tab = tab.view.id;
    await until("the tab to run", () => tab.view.running, 20000);
    activate(tab);

    document.querySelector("#model-button").click();
    await until("the Go provider in the model picker", () => document.querySelector(`[data-provider="${o.account}"]`), 10000);
    document.querySelector(`[data-provider="${o.account}"]`).click();
    await until("DeepSeek in the model picker", () => document.querySelector(`[data-row="${o.model}"]`), 10000);
    document.querySelector(`[data-row="${o.model}"]`).click();
    await until("the tab to run on the Go account", () => tab.view.account === o.account && tab.view.running, 30000);

    const answered = [];
    let idleSince = 0;
    const answerFirstRunQuestionsUntilIdle = () => {
      const menu = tab.menu;
      if (menu) {
        idleSince = 0;
        const yes = menu.options.find((x) => /^Yes\b/.test(x.head));
        const key = menu.options.map((x) => x.head).join("|");
        if (yes && answered.at(-1) !== key) {
          answered.push(key);
          const button = document.querySelector(`#approval [data-option="${yes.n}"]`) ?? document.querySelector('#approval [data-approval="accept"]');
          if (button) button.click();
          else chooseOption(tab, yes.n, yes.label);
        }
        return false;
      }
      if (!terminalMode(tab)) return (idleSince = 0), false;
      idleSince ||= Date.now();
      return Date.now() - idleSince > 1500;
    };
    await until("claude's prompt on the Go account", answerFirstRunQuestionsUntilIdle, 90000);
    result.startup = answered;

    prompt.value = o.prompt;
    const t0 = Date.now();
    result.t0 = t0;
    await send();

    const at = () => +((Date.now() - t0) / 1000).toFixed(2);
    const mark = (name, ok) => {
      if (ok && !(name in result.seen)) result.seen[name] = at();
    };
    const of = (items) => ({
      thinking: items.some((i) => i.kind === "thinking"),
      bash: items.find((i) => i.kind === "tool" && i.name === "Bash"),
      ask: items.some((i) => i.kind === "tool" && i.name === "AskUserQuestion"),
      done: items.some((i) => i.kind === "text" && /sweep is done/.test(i.text)),
    });
    let machine = [];
    let asking = false;
    let lastAsk = 0;
    const askMachine = () => {
      if (asking || Date.now() - lastAsk < 500) return;
      asking = true;
      lastAsk = Date.now();
      callOn(tab.machine, { cmd: "thread", tab: tab.view.id, from: 0 })
        .then((page) => {
          machine = page.items;
          result.session = page.session ?? result.session;
        })
        .catch((e) => result.errors.push(`${at()}s thread: ${e}`))
        .finally(() => (asking = false));
    };
    const observe = () => {
      askMachine();
      const m = of(machine);
      const g = of(threadOf(tab).items);
      mark("machine.thinking", m.thinking);
      mark("machine.bash", m.bash);
      mark("machine.ask", m.ask);
      mark("gui.thinking", g.thinking);
      mark("gui.bash", g.bash);
      mark("gui.ask", g.ask);
      const thread = document.querySelector("#thread");
      mark("dom.thinking", thread.querySelector("details.tool.thought"));
      mark("dom.bash", g.bash && thread.querySelector(`[data-fold="${CSS.escape(g.bash.id)}"]`));
      const screen = screenLines(tab).join("\n");
      mark("screen.bash", /echo sweep/.test(screen));
      mark("screen.menu", terminalMenu(tab)?.options.some((x) => /North route/.test(x.head)));
      mark("tab.menu", tab.menu?.options.some((x) => /North route/.test(x.head)));
      const card = document.querySelector("#approval");
      mark("dom.menu", card && !card.hidden && /North route/.test(card.textContent));
      mark("phase.asking", phaseOf(tab) === "asking");
      const status = `${statusOf(tab)}${tab.view.message ? ` (${tab.view.message})` : ""} phase=${phaseOf(tab)}`;
      if (result.statuses.at(-1)?.[1] !== status) result.statuses.push([at(), status]);
      const menu = tab.menu;
      if (menu && menuKind(menu) === "approval" && !result.menus.includes(menu.question)) {
        result.menus.push(menu.question);
        mark("permission", true);
        document.querySelector('#approval [data-approval="accept"]')?.click();
      }
      return { g, screen };
    };
    const end = t0 + o.deadlineMs;
    while (Date.now() < end && !REQUIRED.every((name) => name in result.seen)) {
      observe();
      await sleep(100);
    }
    result.failures = REQUIRED.filter((name) => !(name in result.seen));
    result.screen = menuRowsWithBlankRows(tab);
    result.shown = document.querySelector("#thread .turn:last-of-type")?.innerText.split("\n").filter((l) => l.trim()).slice(-8);
    if (typeof goThreadCapture === "function") await goThreadCapture(o.variant);

    if (result.seen["tab.menu"] !== undefined) {
      document.querySelector('#approval [data-option="1"]')?.click();
      for (const stop = Date.now() + 30000; Date.now() < stop && !("gui.done" in result.seen); await sleep(100)) {
        observe();
        mark("gui.done", of(threadOf(tab).items).done);
      }
      if (!("gui.done" in result.seen)) result.failures.push("gui.done");
    }
    result.items = threadOf(tab).items.map((i) => (i.kind === "tool" ? `tool:${i.name}` : i.kind));
    result.requests = (await (await fetch(`${o.mock}/mock/log`)).json()).map((r) => ({
      at: +((r.at - t0) / 1000).toFixed(2),
      answered: r.answeredAt ? +((r.answeredAt - t0) / 1000).toFixed(2) : null,
      main: r.main,
      step: r.step,
      how: r.how,
    }));
    closeTab(tab);
  } catch (e) {
    // WebKit's stack holds only the frames, not the message.
    result.failures.push(`${e}\n${e?.stack ?? ""}`.trim());
    result.view = tab && {
      account: tab.view.account,
      running: tab.view.running,
      status: tab.view.status,
      message: tab.view.message,
      phase: phaseOf(tab),
      menu: tab.menu?.question ?? null,
      answering: !!tab.answering,
      waitingForIdle: threadOf(tab).onIdle?.length ?? 0,
      note: document.querySelector("#composer-note")?.textContent ?? null,
      screen: screenLines(tab).filter((l) => l.trim()).slice(-12),
    };
  }
  result.ok = result.failures.length === 0;
  return result;
}

function menuRowsWithBlankRows(tab) {
  const screen = screenLines(tab);
  return screen.slice(Math.max(0, screen.findLastIndex((l) => /^\s*[─━]{8,}/.test(l)) - 16)).map((l) => l.trimEnd());
}
