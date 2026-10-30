export function groupProjects({ threads, grouping = false }) {
  const groups = new Map();
  for (const thread of threads) {
    const key = grouping && thread.repo ? `repo:${thread.repo}` : `${thread.machine}:${thread.path}`;
    let group = groups.get(key);
    if (!group) {
      group = { key, name: thread.name, machine: thread.machine, path: thread.path, tab: thread.thread ?? null, tabs: [] };
      groups.set(key, group);
    }
    if (thread.thread) group.tabs.push(thread.thread);
  }
  return [...groups.values()];
}
