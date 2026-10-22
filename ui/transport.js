/** The app's bridge to its own machine, or null in a plain browser, where the hub serves this page as the web app. */
const TAURI = window.__TAURI__ ?? null;
document.documentElement.dataset.shell = TAURI ? "app" : "web";

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

/**
 * How the UI reaches one machine: `call` sends a `wire::Call` and resolves with its answer,
 * `attach` streams a tab's output to `sink.write(bytes)`, its screen first, until `detach`.
 */
const localTransport = {
  call: (call) => invoke("machine_call", { call }),
  attach(tab, sink) {
    const output = new TAURI.core.Channel();
    output.onmessage = (bytes) => sink.write(new Uint8Array(bytes));
    return invoke("attach_tab", { tab, output });
  },
  detach: (tab) => invoke("detach_tab", { tab }),
};
