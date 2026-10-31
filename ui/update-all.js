function accountMachines() {
  const rows = [];
  for (const [id, machine] of machines) {
    const view = id === LOCAL ? machine.hubView : machine.view;
    if (view?.id && view.id !== "local") rows.push(view);
  }
  return rows;
}

const FINAL = new Set(["up_to_date", "failed", "cannot", "restarting"]);
// Four downloads of five minutes, then a minute for the new binary to be accepted.
const PHASE_WAIT = 21 * 60 * 1000;

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
    const timer = setTimeout(resolve, updateEveryMachine.phaseWait ?? PHASE_WAIT);
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
