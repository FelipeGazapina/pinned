// #project and @machine in the global orchestrator's composer. The parser, the catalog and the
// popover keys live here so node --test can drive them without the page; gui-composer.js wires
// them to the prompt, and only while the open tab is the account's global orchestrator.

/** The one test for the global orchestrator tab. A view without the field is an ordinary tab. */
function isGlobalOrchestratorTab(tab) {
  return tab?.view?.orchestrator_global === true;
}

/** The slug a project or machine name becomes inside a tag: lower case, accents dropped, words
 * joined by one hyphen. "MacBook Pro" and "macbook-pro" share a slug. */
function tagSlug(name) {
  return String(name ?? "")
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[\s_./]+/g, "-")
    .replace(/[^a-z0-9-]/g, "")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
}

/** `#` or `@` and a slug, at the start of the text or after whitespace, so `felipe@mac` and
 * `a#b` are not tags. The popover, the paint and the routing all read tags with it. */
const TAG_WORD = /(?<!\S)[#@][A-Za-z0-9-]+/g;

/** Whether the text holds any tag, so a tab without one keeps the plain textarea. */
function hasTags(text) {
  return /(?<!\S)[#@][A-Za-z0-9-]/.test(String(text ?? ""));
}

/** Paragraphs split on a blank line, as `{ text, start, end, index }`. Blank paragraphs drop out. */
function composerParagraphs(text) {
  const source = String(text ?? "");
  const out = [];
  const add = (start, end) => {
    const raw = source.slice(start, end);
    if (raw.trim()) out.push({ text: raw, start, end, index: out.length });
  };
  const blank = /\n[ \t]*\n/g;
  let at = 0;
  for (const match of source.matchAll(blank)) {
    add(at, match.index);
    at = match.index + match[0].length;
  }
  add(at, source.length);
  return out;
}

/** The tag word the caret is in or right after, or null. */
function tagTokenAt(text, caret) {
  const source = String(text ?? "");
  const at = Math.max(0, Math.min(caret ?? 0, source.length));
  const slugChar = (i) => /[A-Za-z0-9-]/.test(source.charAt(i));
  let start = at;
  while (start > 0 && slugChar(start - 1)) start--;
  const sign = source.charAt(start - 1);
  if (start === 0 || (sign !== "#" && sign !== "@")) return null;
  if (start > 1 && !/\s/.test(source.charAt(start - 2))) return null;
  let end = at;
  while (end < source.length && slugChar(end)) end++;
  return { sign, query: source.slice(start, at).toLowerCase(), start: start - 1, end };
}

/** A paragraph's project and machine: its first `#` and its first `@`, lower-cased. */
function paragraphTags(text) {
  let project = "";
  let machine = "";
  for (const [tag] of String(text ?? "").matchAll(TAG_WORD)) {
    if (tag[0] === "#" && !project) project = tag.slice(1).toLowerCase();
    if (tag[0] === "@" && !machine) machine = tag.slice(1).toLowerCase();
  }
  return { project, machine };
}

/** Where one paragraph goes. `catalog` is what `tagCatalog` builds. Kinds: `routed`, `ambiguous`
 * (the repo is on 2+ machines and no @ decides), `untagged`, `unknown`. A routed row also names
 * its `project`, its `machine` and that machine's `captain` state. Advisory only. */
function routeParagraph(text, { projects = [], machines = [], name = "Orchestrator" } = {}) {
  const tags = paragraphTags(text);
  const flat = String(text ?? "").replace(/\s+/g, " ").trim();
  const preview = flat.length > 50 ? `${flat.slice(0, 50)}…` : flat;
  if (!tags.project) return { preview, kind: "untagged", text: `no project: ${name} decides` };
  const project = projects.find((p) => p.slug === tags.project);
  if (!project) return { preview, kind: "unknown", text: `unknown project #${tags.project}` };
  const hosts = project.machines;
  if (!tags.machine && hosts.length >= 2) {
    return {
      preview,
      kind: "ambiguous",
      text: `${project.name} is on ${hosts.length} machines`,
      choices: hosts.map((m) => m.slug),
    };
  }
  const host = tags.machine ? hosts.find((m) => m.slug === tags.machine) : hosts[0];
  const named = tags.machine && (host ?? machines.find((m) => m.slug === tags.machine));
  const where = tags.machine ? named?.name ?? tags.machine : hosts[0]?.name ?? "no machine";
  const captain = host?.captain ?? "none";
  const starts = captain === "none" ? " It has no Captain yet, so one starts." : "";
  return { preview, kind: "routed", text: `Goes to Captain ${project.name} on ${where}.${starts}`, project: project.name, machine: where, captain };
}

/** One routing row per paragraph, each read from that paragraph's tags alone. */
function routePreview(text, catalog) {
  return composerParagraphs(text).map((p) => ({ ...routeParagraph(p.text, catalog), index: p.index }));
}

/** The line under a sent message: each tagged paragraph's Captain, as the composer routed it,
 * or null when nothing is tagged or a tag no longer resolves to one Captain. */
function routedLine(text, catalog) {
  const rows = routePreview(text, catalog).filter((row) => row.kind !== "untagged");
  if (!rows.length || rows.some((row) => row.kind !== "routed")) return null;
  const names = [...new Set(rows.map((row) => `Captain ${row.project} on ${row.machine}`))];
  const list = names.length > 1 ? `${names.slice(0, -1).join(", ")} and ${names.at(-1)}` : names[0];
  return `Routed to ${list}`;
}

/** Rendered message HTML with each tag outside code and links painted as `tagLineHtml` paints
 * the composer's: the text between elements is already escaped, so it passes through as is. */
function tagsInHtml(html, known) {
  let skip = 0;
  return String(html ?? "").split(/(<[^>]*>)/).map((part, i) => {
    if (i % 2) {
      const tag = part.match(/^<(\/?)(code|pre|a)\b/i);
      if (tag) skip = Math.max(0, skip + (tag[1] ? -1 : 1));
      return part;
    }
    return skip || !hasTags(part) ? part : tagLineHtml(part, known, (text) => text);
  }).join("");
}

/** Writes `@slug` into paragraph `index` only: after its first `#project`, or at its start. A
 * paragraph that already names a machine stays as it is. */
function tagParagraph(text, index, slug) {
  const source = String(text ?? "");
  const target = composerParagraphs(source)[index];
  if (!target || paragraphTags(target.text).machine) return source;
  const project = [...target.text.matchAll(TAG_WORD)].find(([tag]) => tag[0] === "#");
  if (!project) return `${source.slice(0, target.start)}@${slug} ${source.slice(target.start)}`;
  const at = target.start + project.index + project[0].length;
  return `${source.slice(0, at)} @${slug}${source.slice(at)}`;
}

/** Replaces the partial tag at the caret with the full tag and one space, or null off a tag. */
function insertTagText(text, caret, slug) {
  const token = tagTokenAt(text, caret);
  if (!token) return null;
  const full = `${token.sign}${slug} `;
  const after = text.charAt(token.end) === " " ? token.end + 1 : token.end;
  return { text: text.slice(0, token.start) + full + text.slice(after), caret: token.start + full.length };
}

/** What a key does to the popover: arrows move the highlight, Enter and Tab insert the
 * highlighted option, Esc closes. While it is closed, or with nothing to insert, every key
 * stays the composer's. `typed` says the highlighted tag is already typed out in full; Enter then
 * stays the composer's too, as it does for a typed slash command, so `no #site` sends. */
function tagPopoverKey(key, { open, count, hi, typed = false }) {
  const none = { handled: false, hi, action: "none" };
  if (!open || (key === "Enter" && typed)) return none;
  if (key === "ArrowDown" || key === "ArrowUp") {
    const step = key === "ArrowDown" ? 1 : -1;
    return { handled: true, hi: count ? (hi + step + count) % count : 0, action: "move" };
  }
  if (key === "Enter" || key === "Tab") return count ? { handled: true, hi: Math.min(hi, count - 1), action: "insert" } : none;
  if (key === "Escape") return { handled: true, hi, action: "close" };
  return none;
}

/** The popover's options for a token: projects for `#` (prefix of the slug), machines for `@`
 * (prefix of the slug or the name). */
function tagOptions(token, { projects = [], machines = [] } = {}) {
  if (!token) return [];
  const q = token.query;
  if (token.sign === "#") return projects.filter((p) => p.slug.startsWith(q));
  return machines.filter((m) => m.slug.startsWith(q) || m.name.toLowerCase().startsWith(q));
}

/** A project's Captain across its machines: working when any is mid-turn, idle when one is
 * open, none otherwise. */
function captainState(states) {
  if (states.includes("working")) return "working";
  return states.includes("idle") ? "idle" : "none";
}

/** A project's name in tags: its repo's name when a checkout's folder carries it, as the node
 * names the project, else the group's, which is the folder of whichever thread came first. */
function projectTagName(group, repo) {
  const base = String(repo ?? "").split("/").pop();
  return base && group.checkouts.some((c) => String(c.path).split(/[\\/]/).pop() === base) ? base : group.name;
}

/** The account's projects and machines as tags see them. `groups` is the sidebar's
 * `projectGroups(true)` (grouped by repo whatever the sidebar's Project grouping says), `directory` is
 * `machineDirectory()`, `phase(tab)` is `phaseOf`, and `repoAt(machine, path)` the repo a folder
 * reported. `workers(tab)`, when given, adds each host's running worker count to its entry. A
 * group holding only the global orchestrator's own folder is not a project. */
function tagCatalog({ groups, directory, phase, repoAt, name = "Orchestrator", workers }) {
  const nameOf = (id) => (id === directory.localId ? directory.localName : directory.names[id] ?? "machine");
  const machineOf = (id) => ({ slug: tagSlug(nameOf(id)), name: nameOf(id), online: directory.online[id] === true });
  const projects = [];
  for (const group of groups) {
    if (group.tabs.length && group.tabs.every(isGlobalOrchestratorTab) && group.checkouts.every((c) => c.thread)) continue;
    const hosts = [];
    for (const { machine } of group.checkouts) {
      if (hosts.some((m) => m.id === machine)) continue;
      const captains = group.tabs.filter((tab) => tab.machine === machine && tab.view.role === "orchestrator");
      const busy = captains.some((tab) => !["done", "idle"].includes(phase(tab)));
      const host = { id: machine, ...machineOf(machine), captain: !captains.length ? "none" : busy ? "working" : "idle" };
      if (workers) host.workers = captains.reduce((n, tab) => n + (Number(workers(tab)) || 0), 0);
      hosts.push(host);
    }
    const repo = group.checkouts.map((c) => repoAt(c.machine, c.path)).find(Boolean) ?? "";
    const name = projectTagName(group, repo);
    projects.push({ slug: tagSlug(name), name, repo, machines: hosts });
  }
  return { projects, machines: Object.keys(directory.names).map(machineOf), name };
}

/** One line of the mirror under the textarea: the same letters, each tag in a span that is blue
 * for a known project, green for a known machine, red otherwise. No whitespace is added between
 * spans, since the mirror is pre-wrap and must keep the caret on the letter it covers. */
function tagLineHtml(line, known, esc) {
  return line.split(/((?<!\S)[#@][A-Za-z0-9-]+)/).map((part, i) => {
    if (i % 2 === 0) return esc(part);
    const kind = part[0] === "#" ? "project" : "machine";
    const ok = known[kind].has(part.slice(1).toLowerCase());
    return `<span class="tag ${kind}${ok ? "" : " unknown"}">${esc(part)}</span>`;
  }).join("");
}

/** The popover's rows for a token: a heading, then each option with what it inserts. */
function composerTagPopoverHtml(token, options, hi, esc) {
  const project = token.sign === "#";
  const heading = `<p class="heading">${project ? "Projects" : "Machines"}</p>`;
  if (!options.length) return `${heading}<p class="none">No ${project ? "project" : "machine"} matches '${esc(token.query)}'</p>`;
  return heading + options.map((opt, i) => {
    const offline = project ? opt.machines.length > 0 && opt.machines.every((m) => !m.online) : !opt.online;
    const about = project
      ? [esc(opt.repo || "no remote"), ...opt.machines.map((m) => `<span class="${m.online ? "" : "off"}">${esc(m.name)}</span>`)].join(" · ")
      : opt.online ? "online" : "offline";
    const state = project ? (offline ? "offline" : captainState(opt.machines.map((m) => m.captain))) : `@${opt.slug}`;
    return `<button type="button" class="${i === hi ? "highlight" : ""}"${offline ? ` data-offline="true"` : ""}><span class="sigil ${project ? "project" : "machine"}">${token.sign}</span><span class="text"><span class="name">${esc(opt.name)}</span><span class="about">${about}</span></span><span class="state${project ? ` ${state}` : ""}">${esc(state)}</span></button>`;
  }).join("");
}

/** The routing preview: one row per paragraph, with machine buttons on an ambiguous one. A row
 * names its paragraph only when the draft has more than one. */
function composerRoutingHtml(rows, esc) {
  const arrow = `<svg class="arrow" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12h14"></path><path d="m13 6 6 6-6 6"></path></svg>`;
  return rows.map((row) => {
    const choices = row.kind === "ambiguous" ? `<span class="choices">${row.choices.map((slug) => `<button type="button" data-machine="${esc(slug)}">@${esc(slug)}</button>`).join("")}</span>` : "";
    const where = row.kind === "routed"
      ? `Goes to <b>Captain ${esc(row.project)}</b> on ${esc(row.machine)}.${row.captain === "none" ? " It has no Captain yet, so one starts." : ""}`
      : esc(row.text);
    const preview = rows.length > 1 ? `<span class="preview">${esc(row.preview)}</span>` : "";
    return `<div class="route ${row.kind}" data-index="${row.index}">${arrow}${preview}<span class="where">${where}</span>${choices}</div>`;
  }).join("");
}
