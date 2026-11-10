function accountMachines() {
  const rows = [];
  for (const [id, machine] of machines) {
    const view = id === LOCAL ? machine.hubView : machine.view;
    if (view?.id && view.id !== "local") rows.push(view);
  }
  return rows;
}

const FINAL = new Set(["up_to_date", "failed", "cannot"]);
const PHASE_WAIT = 4 * 5 * 60 * 1000 + 60 * 1000;

function land(id, call) {
  const row = () => accountMachines().find((view) => view.id === id);
  const before = JSON.stringify(row()?.update ?? null);
  let listen;
  const changed = new Promise((resolve) => {
    listen = () => {
      const update = row()?.update;
      if (JSON.stringify(update ?? null) === before) return;
      if (!FINAL.has(update?.state)) return;
      resolve();
    };
    viewListeners.push(listen);
  });
  const finished = call.then(
    (status) => (FINAL.has(status?.state) ? null : new Promise(() => {})),
    (error) => {
      const view = row();
      const reason = error && error.message ? error.message : String(error);
      if (view) view.update = { state: "failed", to: view.update?.to || "", reason };
      for (const notify of viewListeners.slice()) notify();
    },
  );
  const timed = new Promise((resolve) => {
    const timer = setTimeout(() => {
      const view = row();
      if (view && !FINAL.has(view.update?.state)) {
        view.update = { state: "failed", to: view.update?.to || "", reason: "the update timed out" };
        for (const notify of viewListeners.slice()) notify();
      }
      resolve();
    }, updateEveryMachine.phaseWait ?? PHASE_WAIT);
    if (typeof timer.unref === "function") timer.unref();
  });
  return Promise.race([changed, finished, timed]).finally(() => {
    const at = viewListeners.indexOf(listen);
    if (at >= 0) viewListeners.splice(at, 1);
  });
}

async function updateEveryMachine() {
  if (updateEveryMachine.busy) return;
  updateEveryMachine.busy = true;
  try {
    const self = account?.machine_id ?? null;
    const online = accountMachines().filter((view) => view.online && !view.upstash_box);
    const others = online.filter((view) => view.id !== self);
    await Promise.all(others.map((view) => land(view.id, hub.call(view.id, { cmd: "update_self" }))));
    if (self && online.some((view) => view.id === self)) await land(self, hub.call(self, { cmd: "update_self" }));
  } finally {
    updateEveryMachine.busy = false;
  }
}

function rowUpdate(view) {
  if (view?.update) return updateLabel(view.update);
  return view?.version ? `v${view.version}` : "";
}

function updateLabel(update) {
  switch (update.state) {
    case "up_to_date": return `up to date · v${update.version}`;
    case "updating": return update.to ? `updating to v${update.to}` : "updating";
    case "restarting": return `restarting into v${update.to}`;
    case "failed": return `update failed: ${update.reason}`;
    case "cannot": return update.reason;
    default: return "";
  }
}

/** The newest pinned the account can install, without the "v": the hub's version in a browser, the app's own or
 * the one the update check found in the app. "" until it is known, and then no row claims to be up to date. */
let newestVersion = "";
function setNewestVersion(version) {
  const next = String(version ?? "").trim().replace(/^v/, "");
  if (next === newestVersion) return;
  newestVersion = next;
  document.dispatchEvent(new CustomEvent("newest-version"));
}

const versionParts = (version) => (String(version ?? "").match(/\d+/g) ?? []).slice(0, 3).map(Number);
/** -1, 0 or 1 as version `a` is older than, the same as, or newer than `b`, comparing major, minor, patch. */
function versionOrder(a, b) {
  const [x, y] = [versionParts(a), versionParts(b)];
  for (let i = 0; i < 3; i++) {
    const gap = (x[i] ?? 0) - (y[i] ?? 0);
    if (gap) return gap < 0 ? -1 : 1;
  }
  return 0;
}

/** The online machines that run an older pinned than `newest`. A box is replaced by the hub, so it is left out. */
function machinesBehind(newest = newestVersion) {
  return accountMachines().filter((view) => newest && view.online && !view.upstash_box && view.version && versionOrder(view.version, newest) < 0);
}

/** Where a machine stands against `newest`, the line its row prints and what its Update button does:
 * `standing` is offline, updating, failed, cannot, behind, current or unknown (no version to compare). Update
 * is offered where it can help: an offline machine can't take it, and one on the newest version has none. */
function updateStanding(view, newest = newestVersion) {
  const state = view.update?.state;
  const line = (standing, text, button) => ({ standing, text, button: { label: "Update", disabled: false, title: "", primary: false, ...button } });
  if (!view.online) return line("offline", rowUpdate(view), { disabled: true, title: "This machine is offline, so it can't update" });
  if (state === "updating" || state === "restarting") return line("updating", updateLabel(view.update), { label: "Updating…", disabled: true });
  if (state === "failed") return line("failed", updateLabel(view.update), { label: "Retry update" });
  if (state === "cannot") return line("cannot", updateLabel(view.update), { disabled: true, title: view.update.reason });
  if (!newest || !view.version) return line("unknown", rowUpdate(view), {});
  if (versionOrder(view.version, newest) < 0) return line("behind", `v${view.version} · update to v${newest}`, { label: `Update to v${newest}`, primary: true });
  return line("current", `up to date · v${view.version}`, { label: "Up to date", disabled: true, title: `Already on v${view.version}` });
}
