export function groupProjects({ threads, listed = {}, grouping = false }) {
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
  for (const [machine, projects] of Object.entries(listed)) {
    for (const project of projects ?? []) {
      if (!project?.path) continue;
      const key = grouping && project.repo ? `repo:${project.repo}` : `${machine}:${project.path}`;
      if (groups.has(key)) continue;
      groups.set(key, { key, name: project.name || project.path, machine, path: project.path, tab: null, tabs: [] });
    }
  }
  return [...groups.values()];
}
