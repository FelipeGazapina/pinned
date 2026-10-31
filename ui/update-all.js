function accountMachines() {
  const rows = [];
  for (const [id, machine] of machines) {
    const view = id === LOCAL ? machine.hubView : machine.view;
    if (view?.id && view.id !== "local") rows.push(view);
  }
  return rows;
}

function land(id, call) {
  const before = JSON.stringify(accountMachines().find((view) => view.id === id)?.update ?? null);
  let listen;
  const changed = new Promise((resolve) => {
    listen = () => {
      const next = JSON.stringify(accountMachines().find((view) => view.id === id)?.update ?? null);
      if (next === before) return;
      resolve();
    };
    viewListeners.push(listen);
  });
  return Promise.race([changed, call.then(() => null, () => null)]).finally(() => {
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
