const { invoke, Channel } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const win = window.__TAURI__.window.getCurrentWindow();

const STATUS_TEXT = { idle: "", working: "working…", done: "done", "needs-input": "needs you" };

const term = new Terminal({
  fontFamily: "ui-monospace, Menlo, monospace",
  fontSize: 12,
  cursorBlink: true,
  allowTransparency: true,
  theme: { background: "#00000000" },
});
const fit = new FitAddon.FitAddon();
term.loadAddon(fit);
term.open(document.getElementById("term"));
fit.fit();

let running = false;

async function startClaude() {
  const output = new Channel();
  output.onmessage = (bytes) => term.write(new Uint8Array(bytes));
  const cwd = await invoke("start_claude", { cols: term.cols, rows: term.rows, output });
  document.getElementById("title").textContent = cwd.replace(/^\/Users\/[^/]+/, "~");
  running = true;
  setStatus("idle");
}

term.onData((data) => {
  if (running) return invoke("write_input", { data });
  if (data === "\r") startClaude();
});

listen("claude-exit", () => {
  running = false;
  term.write("\r\n\x1b[2m[claude exited, press Enter to restart]\x1b[0m\r\n");
});

function setStatus(status, message) {
  document.body.dataset.status = status;
  const text = message || STATUS_TEXT[status];
  document.getElementById("status-text").textContent = STATUS_TEXT[status];
  document.getElementById("pill-text").textContent = text || "claude";
}

listen("agent-event", ({ payload }) => {
  setStatus(payload.status, payload.status === "needs-input" ? payload.message : null);
  if (payload.status !== "working" && document.body.dataset.view === "pill") {
    document.body.classList.remove("alert");
    void document.body.offsetWidth;
    document.body.classList.add("alert");
  }
});

async function setView(view) {
  document.body.classList.remove("alert");
  await invoke("set_view", { view });
  document.body.dataset.view = view;
  if (view === "expanded") term.focus();
}
document.getElementById("minimize").onclick = () => setView("pill");
document.getElementById("expand").onclick = () => setView("expanded");
document.getElementById("grip").onmousedown = (e) => {
  e.preventDefault();
  win.startResizeDragging("SouthEast");
};

document.getElementById("folder").onclick = async () => {
  if (await invoke("choose_folder")) {
    term.reset();
    await startClaude();
    term.focus();
  }
};

async function checkUpdate() {
  try {
    const version = await invoke("check_update");
    if (version) document.body.dataset.update = version;
    else delete document.body.dataset.update;
    for (const b of document.querySelectorAll(".update")) {
      b.textContent = b.closest("#pill") ? "↑" : `Update to v${version}`;
    }
  } catch (e) {
    console.warn("update check failed", e);
  }
}
for (const button of document.querySelectorAll(".update")) {
  button.onclick = async () => {
    for (const b of document.querySelectorAll(".update")) {
      b.disabled = true;
      b.textContent = "updating…";
    }
    try {
      await invoke("install_update");
    } catch (e) {
      term.write(`\r\n\x1b[31m[update failed: ${e}]\x1b[0m\r\n`);
      checkUpdate();
    }
  };
}
checkUpdate();
setInterval(checkUpdate, 6 * 60 * 60 * 1000);

new ResizeObserver(() => {
  if (document.body.dataset.view !== "expanded") return;
  fit.fit();
  invoke("resize_pty", { cols: term.cols, rows: term.rows });
}).observe(document.getElementById("term"));

startClaude().then(() => term.focus());
