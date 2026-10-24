const TAURI = window.__TAURI__ ?? null;
document.documentElement.dataset.shell = TAURI ? "app" : "web";
if (!TAURI && "serviceWorker" in navigator) navigator.serviceWorker.register("sw.js").catch(() => {});

const invoke = TAURI ? TAURI.core.invoke : (cmd) => Promise.reject(`${cmd} only works in the app`);
const listen = (event, handler) => TAURI?.event.listen(event, ({ payload }) => handler(payload));

function load(key) {
  try {
    return localStorage.getItem(key);
  } catch {}
}
function save(key, value) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {}
}

/** The calls `Call::may_wait` lets run out of order, which machine_call would run on the app's main thread. */
const WAITING_CALLS = new Set(["run", "pull_request", "pull_requests", "pr_action", "thread", "workspace", "project_history", "diff", "ports", "editors", "open_in", "commands", "files", "branches", "checkout", "checkout_pr", "set_workspace"]);

const localTransport = {
  call: (call) => invoke(WAITING_CALLS.has(call.cmd) ? "machine_call_slow" : "machine_call", { call }),
  attach(tab, sink) {
    const output = new TAURI.core.Channel();
    output.onmessage = (bytes) => sink.write(new Uint8Array(bytes));
    return invoke("attach_tab", { tab, output });
  },
  detach: (tab) => invoke("detach_tab", { tab }),
};
