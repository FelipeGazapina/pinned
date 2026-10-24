const MODELS_MENU = [
  ["default", "Default", "The model your Claude Code settings pick"],
  ["opus", "Opus", "Most capable for complex work"],
  ["sonnet", "Sonnet", "Fast and capable for everyday tasks"],
  ["haiku", "Haiku", "Fastest for quick answers"],
];

function renderComposer(tab) {
  const working = !!tab && phaseOf(tab) === "working";
  composer.dataset.working = working;
  composer.dataset.empty = !prompt.value.trim();
  document.querySelector("#send").innerHTML = icon(working && prompt.value.trim() ? "list-plus" : "arrow-up");
  document.querySelector("#send").title = working ? "Queue message (Enter)" : "Send (Enter)";
  renderApproval(tab);
  document.querySelector("#model-button .name").textContent = tab ? modelName(tab) : "Default";
  const footer = document.getElementById("composer-footer");
  if (!tab) return (footer.innerHTML = "");
  const t = threadOf(tab);
  const pr = prOf(tab);
  const ws = t.workspace;
  footer.innerHTML = `<span class="where" title="${esc(tab.view.cwd)}">${icon(ws?.worktree ? "folder-git" : "folder")}${ws?.worktree ? "Worktree" : "Local checkout"}</span>
    ${tab.machine !== LOCAL ? `<span class="machine">${icon("monitor")}${esc(machineName(tab.machine))}</span>` : ""}
    <span class="spacer"></span>
    ${pr ? `<button type="button" class="pr" data-pr="${esc(pr.url)}" title="Open pull request #${pr.number}">${icon("git-pull-request")}#${pr.number}</button>` : ""}
    ${ws?.branch ? `<span class="branch" title="${esc(ws.branch)}">${icon("git-branch")}<span>${esc(ws.branch)}</span></span>` : ""}`;
  prompt.placeholder = !tab.view.running
    ? "claude isn't running. Press Enter to start it."
    : t.items.length ? "Ask for changes, send follow-ups, or attach images" : "Ask anything, or / for commands";
}

function watchMenu(tab) {
  let queued = false;
  tab.term.onWriteParsed(() => {
    if (queued || !guiOn()) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      const menu = terminalMenu(tab);
      if (JSON.stringify(menu) === JSON.stringify(tab.menu ?? null)) return;
      tab.menu = menu;
      if (tab === active) renderComposer(tab);
      renderSidebar();
    });
  });
}

const ARROW_DOWN = "\x1b[B";
const ARROW_UP = "\x1b[A";

function chooseOption(tab, n, label) {
  const menu = terminalMenu(tab);
  if (menu?.options[n - 1]?.label !== label) {
    tab.menu = menu;
    return renderComposer(tab);
  }
  const from = menu.options.find((o) => o.chosen)?.n ?? 1;
  const keys = (n > from ? ARROW_DOWN : ARROW_UP).repeat(Math.abs(n - from)) + "\r";
  tab.menu = null;
  renderComposer(tab);
  renderSidebar();
  return callOn(tab.machine, { cmd: "write_input", tab: tab.view.id, data: keys });
}

/** claude heads a permission menu with what it wants to run, such as "Bash command", then a
 * description and its own tips, the command in a dashed box, and the question last, such as "Do you
 * want to proceed?". */
function renderApproval(tab) {
  const menu = tab?.menu;
  approval.hidden = !menu;
  if (!menu) return (approval.dataset.key = "");
  const key = JSON.stringify([tab.key, menu]);
  if (approval.dataset.key === key) return;
  approval.dataset.key = key;
  const lines = menu.question.split("\n");
  const permission = lines.length > 1 && /proceed|allow|make this edit|permission/i.test(lines.at(-1));
  approval.dataset.kind = permission ? "approval" : "input";
  const kind = !permission ? "Claude is asking" : /command/i.test(lines[0]) ? "Command approval" : /file|edit/i.test(lines[0]) ? "File change approval" : "Permission approval";
  const what = permission ? lines.at(-1) : menu.question;
  const middle = permission ? lines.slice(1, -1).filter((l) => !/^Tip:/.test(l)) : [];
  const detail = menu.box || middle.join("\n");
  const about = menu.box ? middle.join("\n") : "";
  const options = menu.options.map((o) => `<button type="button" data-option="${o.n}" class="${o.chosen ? "primary" : "outline"}">${esc(o.label)}</button>`);
  approval.innerHTML = `${icon(permission ? "shield" : "circle-alert", "lead")}<div class="body"><p class="kind">${esc(kind)}</p>
      <p class="what">${esc(what)}</p>${about ? `<p class="about">${esc(about)}</p>` : ""}${detail ? `<pre>${esc(detail)}</pre>` : ""}
      <div class="options">${options.join("")}</div></div>
    <div class="actions"><button type="button" data-show-terminal class="outline">Terminal</button></div>`;
}

approval.addEventListener("click", (e) => {
  const option = e.target.closest("[data-option]");
  if (option && active) return chooseOption(active, Number(option.dataset.option), option.textContent);
  if (e.target.closest("[data-show-terminal]")) toggleDrawer(true);
});

function autosize() {
  prompt.style.height = "auto";
  prompt.style.height = `${Math.min(prompt.scrollHeight, 208)}px`;
  if (active) renderComposer(active);
}
prompt.addEventListener("input", autosize);
prompt.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
    e.preventDefault();
    send();
  } else if (e.key === "Escape" && active && phaseOf(active) === "working" && !prompt.value) {
    e.preventDefault();
    stop();
  }
});
document.getElementById("send").onclick = () => send();
document.getElementById("stop").onclick = () => stop();

// Typing into a menu would filter its options or pick one, so the prompt waits for an answer.
function waitsOnMenu(tab) {
  if (!tab.menu) return false;
  approval.animate([{ transform: "translateX(-4px)" }, { transform: "translateX(4px)" }, { transform: "none" }], 180);
  return true;
}

async function send() {
  const tab = active;
  if (!tab) return toggleProjects();
  if (!tab.view.running) return startTab(tab);
  if (waitsOnMenu(tab)) return;
  const text = prompt.value.trim();
  if (!text) return;
  prompt.value = "";
  autosize();
  try {
    await callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text });
  } catch (e) {
    prompt.value = text;
    autosize();
    showAttachNote(String(e), true);
  }
  setTimeout(() => fetchThread(tab), 300);
}

const stop = () => active && callOn(active.machine, { cmd: "write_input", tab: active.view.id, data: "\x1b" });

function insertPaths(paths) {
  const text = paths.map((p) => p.replace(/[^\w@%+=:,./-]/g, "\\$&")).join(" ");
  prompt.value = `${prompt.value}${prompt.value && !prompt.value.endsWith(" ") ? " " : ""}${text} `;
  autosize();
  prompt.focus();
}
document.getElementById("composer-attach").onclick = () => document.getElementById("attach").click();

const composerMenu = document.getElementById("composer-menu");
function openMenu(anchor, entries) {
  composerMenu.replaceChildren(...entries.map(([label, hint, run, cls]) => {
    const b = document.createElement("button");
    b.type = "button";
    b.className = cls ?? "";
    b.innerHTML = `<span>${esc(label)}</span>${hint ? `<span class="hint">${esc(hint)}</span>` : ""}`;
    b.onclick = () => {
      composerMenu.hidden = true;
      run();
    };
    return b;
  }));
  composerMenu.hidden = false;
  composerMenu.style.left = `${anchor.offsetLeft}px`;
}
window.addEventListener("mousedown", (e) => {
  if (!composerMenu.hidden && !e.target.closest("#composer-menu, #model-button, #more-button")) composerMenu.hidden = true;
}, true);

function sendSlash(tab, text) {
  if (!tab || waitsOnMenu(tab)) return false;
  callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text }).then(() => setTimeout(() => fetchThread(tab), 400));
  return true;
}
document.getElementById("model-button").onclick = (e) => {
  const tab = active;
  openMenu(e.currentTarget, MODELS_MENU.map(([id, label, hint]) => [label, hint, () => {
    if (!sendSlash(tab, `/model ${id}`)) return;
    threadOf(tab).picked = label;
    renderComposer(tab);
  }]));
};

function modelName(tab) {
  const t = threadOf(tab);
  return t.picked ?? [...t.items].reverse().find((i) => i.kind === "model")?.name ?? "Default";
}

document.getElementById("more-button").onclick = (e) => {
  const tab = active;
  if (!tab) return;
  openMenu(e.currentTarget, [
    tab.view.bypass
      ? ["Ask for permissions again", "Restarts claude and resumes this conversation", () => setBypass(tab, false)]
      : ["Full access", "Restarts claude with no permission checks and resumes", () => setBypass(tab, true), "danger"],
    ["Compact the conversation", "/compact", () => sendSlash(tab, "/compact")],
    ["Change folder…", "", () => document.getElementById("folder").click()],
    ["Show the terminal", "⌘J", () => toggleDrawer(true)],
    ["Close thread", "⌘W", () => closeTab(tab)],
  ]);
};

function toggleDrawer(open = !ui.drawer) {
  ui.drawer = open;
  document.body.dataset.drawer = open ? "open" : "closed";
  if (open && active) refit(active);
  focusInput();
}
document.getElementById("toggle-terminal").onclick = () => toggleDrawer();
onGuiKey("j", () => toggleDrawer());
