Object.assign(ICON_PATHS, {
  "circle-stop": `<circle cx="12" cy="12" r="10"/><rect x="9" y="9" width="6" height="6" rx="1"/>`,
});

/** The agents claude spawned in the thread, as the last page listed them. An older pinned lists none. */
const agentsOf = (tab) => threadOf(tab).agents ?? [];
const runningAgents = (tab) => agentsOf(tab).filter((a) => a.state === "running");
const agentsWorking = (n) => `${n} ${n === 1 ? "agent" : "agents"} working`;

/** Keeps a page's agents on the thread, and answers whether that changes what the cards show. */
function takeAgents(t, agents = []) {
  const key = JSON.stringify(agents.map((a) => [a.id, a.state, a.updated]));
  t.agents = agents;
  if (t.agentsKey === key) return false;
  t.agentsKey = key;
  return true;
}

/** The `Agent` tool use that spawned `agent`: in the thread, or in its parent agent's steps. */
const spawnOf = (t, agent) =>
  (agent.parent ? (t.agents ?? []).find((a) => a.id === agent.parent)?.steps ?? [] : t.items).find((i) => i.kind === "tool" && i.id === agent.tool);
const spawnedAt = (t, agent) => Date.parse(spawnOf(t, agent)?.at ?? "");
/** When the earliest running agent was spawned, in ms, or NaN. */
const delegatedSince = (tab) => Math.min(...runningAgents(tab).map((a) => spawnedAt(threadOf(tab), a)).filter(Number.isFinite));
/** The last thing the thread or one of its agents did, in unix seconds. */
const lastActivity = (tab) => Math.max(tab.view.updated ?? 0, ...agentsOf(tab).map((a) => Date.parse(a.updated) / 1000 || 0));

function agentSteps(agent) {
  const results = new Map(agent.steps.filter((s) => s.kind === "result").map((r) => [r.tool, r]));
  return agent.steps.filter((s) => s.kind !== "result").map((s) => (s.kind === "tool" ? { ...s, result: results.get(s.id) } : s));
}

function stepLine(step, cwd) {
  if (!step) return { icon: "bot", label: "Starting…" };
  if (step.kind === "thinking") return { icon: "brain", label: thoughtLine(step.text) || "Thinking" };
  if (step.kind === "text") return { icon: "message-circle", label: step.text.split("\n").map((l) => l.replace(/^[\s#>*-]+/, "").trim()).find(Boolean) ?? "" };
  return { icon: toolIcon(step), label: toolLabel(step, cwd) };
}

function agentCardHtml(tab, agent, depth) {
  const t = threadOf(tab);
  const cwd = tab.view.cwd;
  const steps = agentSteps(agent);
  const key = `agent:${agent.id}`;
  const open = t.open.has(key);
  const running = agent.state === "running";
  const since = spawnedAt(t, agent);
  const ended = Date.parse(agent.updated);
  const time = !Number.isFinite(since) ? ""
    : running ? `<span class="ticker" data-since="${since}">${duration(Date.now() - since)}</span>`
    : Number.isFinite(ended) ? duration(ended - since) : "";
  const mark = { running: icon("loader-circle", "state"), done: icon("circle-check", "state"), stopped: icon("circle-stop", "state") }[agent.state] ?? "";
  const chip = [agent.kind, agent.model].filter(Boolean).join(" · ");
  const line = stepLine(steps.at(-1), cwd);
  const body = open
    ? `<div class="agent-steps">${steps.map((s) => (s.kind === "text" ? `<div class="narration md">${markdown(s.text)}</div>` : workRowHtml(s, cwd, t.open))).join("")}</div>`
    : `<div class="agent-step">${icon(line.icon, "kind")}<span class="label${running ? " shimmer" : ""}">${esc(line.label)}</span></div>`;
  return `<div class="agent-card" data-agent="${esc(agent.id)}" data-state="${esc(agent.state)}" style="--depth:${depth}">
    <button type="button" class="agent-head" data-toggle="${esc(key)}" aria-expanded="${open}" title="${esc(agent.name)}">${icon("bot", "kind")}<span class="name">${esc(agent.name || "Agent")}</span>
      ${chip ? `<span class="chip">${esc(chip)}</span>` : ""}<span class="time">${time}</span>${mark}</button>${body}</div>`;
}

/** A card per agent the turn spawned, below its work fold so they show while the fold is shut, with
 * the agents those spawned indented under them. */
function agentCardsHtml(tab, turn) {
  const agents = agentsOf(tab);
  if (!agents.length) return "";
  const tools = new Set(turn.steps.filter((s) => s.kind === "tool").map((s) => s.id));
  const tree = (agent, depth) => [agentCardHtml(tab, agent, depth), ...agents.filter((a) => a.parent === agent.id).flatMap((a) => tree(a, depth + 1))];
  const cards = agents.filter((a) => !a.parent && tools.has(a.tool)).flatMap((a) => tree(a, 0));
  return cards.length ? `<div class="agents">${cards.join("")}</div>` : "";
}

/** Under the last turn while claude's turn has ended and its agents work on. */
function agentsFooterHtml(tab) {
  if (phaseOf(tab) !== "delegating") return "";
  const since = delegatedSince(tab);
  return `<div class="agents-working">${icon("loader-circle")}<span class="shimmer">${agentsWorking(runningAgents(tab).length)}</span>
    ${Number.isFinite(since) ? `<span class="ticker" data-since="${since}">${duration(Date.now() - since)}</span>` : ""}</div>`;
}
