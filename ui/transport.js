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

/** Calls that spawn an agent CLI, answered off the app's main thread. */
const BLOCKING_CALLS = ["list_accounts", "add_account", "sign_in"];

const localTransport = {
  call: (call) => invoke(BLOCKING_CALLS.includes(call.cmd) ? "machine_call_blocking" : "machine_call", { call }),
  attach(tab, sink) {
    const output = new TAURI.core.Channel();
    output.onmessage = (bytes) => sink.write(new Uint8Array(bytes));
    return invoke("attach_tab", { tab, output });
  },
  detach: (tab) => invoke("detach_tab", { tab }),
};
