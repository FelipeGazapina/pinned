/** The team's left navigation, which takes the personal project list's place in the sidebar while a
 * team is current (`teamsCurrent()`): the team, Calls with the count asking me, Bots, Groups and the
 * team's attached Projects (each with "Take out of the team" for the person who attached it and the
 * owner, asked in place), and the person at the bottom. gui-sidebar.js calls `tcNavDraw()` on every render. With no team
 * the nav is removed and the sidebar is the personal one. Team chat is not in this build, so there
 * is no row for it. */

let tcNavEl = null;
/** Finds the control of the nav to focus after the next draw, so a keyboard user keeps their place. */
let tcNavFocus = null;

/** One attached project: its name, and "Take out of the team" for who may. The question is asked in the row itself. */
function tcProjectHtml(project) {
  const name = esc(project.name || project.key);
  const asked = tcStore.detach?.key === project.key ? tcStore.detach : null;
  if (asked) {
    const sending = asked.state === "sending";
    return `<div class="nav-row tc-row tc-plain tc-project tc-confirm" data-project="${esc(project.key)}" role="group"><span class="tc-ask">Take ${name} out of the team? Bots that work in it go back to draft.</span>${asked.line ? `<span class="tc-ask-error" role="alert">${esc(asked.line)}</span>` : ""}<span class="tc-ask-actions"><button type="button" class="tc-detach-yes" data-tc-detach="confirm"${sending ? " disabled" : ""}>${sending ? "Taking it out…" : "Take it out"}</button><button type="button" class="tc-detach-keep" data-tc-detach="keep"${sending ? " disabled" : ""}>Keep</button></span></div>`;
  }
  const out = tcCanDetach(project) ? `<button type="button" class="tc-detach" data-tc-detach="ask" data-key="${esc(project.key)}">Take out of the team</button>` : "";
  return `<div class="nav-row tc-row tc-plain tc-project" data-project="${esc(project.key)}"><span class="label">${name}</span>${out}</div>`;
}

/** The bot's dot: amber while it has an open call, green while it is on, grey while it is off. */
function tcBotDot(bot) {
  if (tcOpenFor(bot.workflow)) return { tone: "call", label: "has a call open" };
  return bot.on ? { tone: "on", label: "on" } : { tone: "off", label: "off" };
}

const tcNavOpens = (key) => typeof SHELL_PAGES !== "undefined" && key in SHELL_PAGES;

function tcNavHtml(team, page) {
  const open = tcCallsForMe().length;
  const row = (key, inner, { label = "", active = false, attrs = "" } = {}) =>
    tcNavOpens(key)
      ? `<button type="button" class="nav-row tc-row${active ? " active" : ""}" data-tc-nav="${key}"${attrs}${label ? ` aria-label="${esc(label)}"` : ""}>${inner}</button>`
      : `<div class="nav-row tc-row tc-plain"${label ? ` aria-label="${esc(label)}"` : ""}${attrs}>${inner}</div>`;
  const bots = tcStore.bots
    .map((bot) => {
      const dot = tcBotDot(bot);
      const name = bot.name || "Untitled bot";
      return row("team-bot", `<span class="tc-dot" data-tone="${dot.tone}" aria-hidden="true"></span><span class="label">${esc(name)}</span>`, { label: `${name}, ${dot.label}`, active: page === "team-run" && tcStore.runs.find((r) => r.id === tcRunShown)?.workflow === bot.workflow, attrs: ` data-workflow="${esc(bot.workflow)}"` });
    })
    .join("");
  const groups = tcStore.groups.map((g) => row("team-group", `<span class="label">@${esc(g.handle)}</span>`, { attrs: ` data-handle="${esc(g.handle)}"` })).join("");
  const me = tcMe();
  // Drawn only for a reason this app knows; the time names its day when it is not today.
  const held = me.owner && team.line_held?.why === "settings_unread" ? `<div class="tc-none" role="status" data-line-held>Bots are waiting: the team's settings could not be read since ${esc(tcWhen(team.line_held.since))}.</div>` : "";
  const calls = tcNavOpens("team-calls")
    ? `<button type="button" class="nav-row tc-row${page === "team-calls" ? " active" : ""}" data-tc-nav="team-calls"><span class="label">Calls</span><span class="count"${open ? "" : " hidden"} title="${open} waiting on you">${open || ""}</span></button>`
    : "";
  return `<div class="tc-team"><span class="tc-mark tc-team-mark" aria-hidden="true">${esc(tcInitials(team.name))}</span><span class="tc-team-name">${esc(team.name)}</span></div>
    ${held}
    ${calls}
    <div class="tc-section"><div class="tc-head">Bots</div>${bots}${row("team-bot-new", `<span class="label">Create a bot</span>`, { attrs: ' data-create="true"' })}</div>
    <div class="tc-section"><div class="tc-head">Groups</div>${groups}</div>
    ${me.owner && tcNavOpens("team-invites") ? `<div class="tc-section"><div class="tc-head">People</div>${row("team-invites", `<span class="label">Invite people</span>`)}</div>` : ""}
    <div class="tc-section tc-projects"><div class="tc-head">Projects</div>${tcStore.projects.map(tcProjectHtml).join("") || `<div class="tc-none">${tcStore.loaded ? "No projects in the team yet." : "Loading…"}</div>`}</div>
    <div class="tc-me"><span class="tc-mark tc-person" style="--tc-hue:${tcHue(me.account || me.name)}" aria-hidden="true">${esc(tcInitials(me.name))}</span><span class="tc-me-name">${esc(me.name)}</span></div>`;
}

function tcNavDraw() {
  const team = tcCurrent();
  if (!team) {
    tcNavEl?.remove();
    tcNavEl = null;
    delete document.body.dataset.team;
    return;
  }
  if (tcStore.team !== team.id) tcRefresh();
  if (!tcNavEl) {
    tcNavEl = document.createElement("nav");
    tcNavEl.id = "team-nav";
    tcNavEl.setAttribute("aria-label", "Team");
    document.getElementById("side-nav").before(tcNavEl);
  }
  document.body.dataset.team = "on";
  const page = typeof shellPage === "function" ? shellPage() : null;
  const html = tcNavHtml(team, page);
  if (tcNavEl.dataset.tcHtml !== html) {
    tcNavEl.dataset.tcHtml = html;
    tcNavEl.innerHTML = html;
  }
  if (tcNavFocus) {
    const find = tcNavFocus;
    tcNavFocus = null;
    find(tcNavEl)?.focus?.();
  }
}

/** Opens the page a row names: a bot or a group through `openTeamBot(workflow)` / `openTeamGroup(handle)`. */
function tcNavOpen(button) {
  const key = button.dataset.tcNav;
  if (!tcNavOpens(key)) return;
  if (key === "team-bot" && button.dataset.workflow) openTeamBot(button.dataset.workflow);
  else if (key === "team-group" && button.dataset.handle) openTeamGroup(button.dataset.handle);
  else if (key === "team-bot-new") openTeamBotNew();
  else openPage(key);
}

/** What a click on a project's "Take out of the team", "Take it out" or "Keep" does. */
function tcNavDetach(button) {
  const act = button.dataset.tcDetach;
  if (act === "ask") {
    tcNavFocus = (nav) => nav.querySelector('[data-tc-detach="keep"]');
    tcDetachAsk(button.dataset.key);
  } else if (act === "keep") {
    const key = tcStore.detach?.key;
    tcNavFocus = (nav) => [...nav.querySelectorAll("[data-project]")].find((row) => row.dataset.project === key)?.querySelector('[data-tc-detach="ask"]');
    tcDetachAsk(null);
  } else if (act === "confirm") tcDetachConfirm();
}

document.addEventListener("click", (e) => {
  const button = e.target.closest?.("#team-nav [data-tc-nav]");
  if (button) return tcNavOpen(button);
  const detach = e.target.closest?.("#team-nav [data-tc-detach]");
  if (detach && !detach.disabled) tcNavDetach(detach);
});
tcOnChange(() => tcNavDraw());
document.addEventListener("teams:changed", () => tcNavDraw());
document.addEventListener("teams:event", () => tcNavDraw());
