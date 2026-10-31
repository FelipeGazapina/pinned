export function sessionOn(checkouts, directory) {
  if (!directory?.localId || !directory?.localName) throw new Error("a directory needs a local id and name");
  if (!checkouts?.length) throw new Error("a session needs a checkout");
  for (const checkout of checkouts) {
    if (!checkout?.machine || !checkout?.path) throw new Error("a checkout needs a machine and a path");
  }
  const nameOf = (id) => (id === directory.localId ? directory.localName : directory.names[id] ?? "machine");
  const winner = checkouts.find((checkout) => checkout.thread === true) ?? checkouts[0];
  const offline = directory.online[winner.machine] !== true;
  const seen = new Set([winner.machine]);
  const also = [];
  for (const checkout of checkouts) {
    if (seen.has(checkout.machine)) continue;
    seen.add(checkout.machine);
    also.push(nameOf(checkout.machine));
  }
  const label = `${nameOf(winner.machine)}${offline ? " (offline)" : ""}${also.length ? ` +${also.length}` : ""}`;
  return { machine: winner.machine, path: winner.path, label, offline, also };
}

export function groupProjects({ threads, listed = {}, grouping = false }) {
  const groups = new Map();
  for (const thread of threads) {
    const key = grouping && thread.repo ? `repo:${thread.repo}` : `${thread.machine}:${thread.path}`;
    let group = groups.get(key);
    if (!group) {
      group = { key, name: thread.name, tab: thread.thread ?? null, tabs: [], checkouts: [] };
      groups.set(key, group);
    }
    if (thread.thread) group.tabs.push(thread.thread);
    if (!group.checkouts.some((checkout) => checkout.machine === thread.machine && checkout.path === thread.path)) {
      group.checkouts.push({ machine: thread.machine, path: thread.path, thread: true });
    }
  }
  for (const [machine, projects] of Object.entries(listed)) {
    for (const project of projects ?? []) {
      if (!project?.path) continue;
      const key = grouping && project.repo ? `repo:${project.repo}` : `${machine}:${project.path}`;
      let group = groups.get(key);
      if (!group) {
        group = { key, name: project.name || project.path, tab: null, tabs: [], checkouts: [] };
        groups.set(key, group);
      }
      if (group.checkouts.some((checkout) => checkout.machine === machine && checkout.path === project.path)) continue;
      group.checkouts.push({ machine, path: project.path, thread: false });
    }
  }
  return [...groups.values()];
}
