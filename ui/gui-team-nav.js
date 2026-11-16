/** The team's left navigation, which takes the personal project list's place in the sidebar while a
 * team is current (`teamsCurrent()`): the team, Calls with the count asking me, Bots and Groups,
 * and the person at the bottom. gui-sidebar.js calls `tcNavDraw()` on every render. With no team
 * the nav is removed and the sidebar is the personal one. Team chat is not in this build, so there
 * is no row for it. */

let tcNavEl = null;

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
  const calls = tcNavOpens("team-calls")
    ? `<button type="button" class="nav-row tc-row${page === "team-calls" ? " active" : ""}" data-tc-nav="team-calls"><span class="label">Calls</span><span class="count"${open ? "" : " hidden"} title="${open} waiting on you">${open || ""}</span></button>`
    : "";
  return `<div class="tc-team"><span class="tc-mark tc-team-mark" aria-hidden="true">${esc(tcInitials(team.name))}</span><span class="tc-team-name">${esc(team.name)}</span></div>
    ${calls}
    <div class="tc-section"><div class="tc-head">Bots</div>${bots}${row("team-bot-new", `<span class="label">Create a bot</span>`, { attrs: ' data-create="true"' })}</div>
    <div class="tc-section"><div class="tc-head">Groups</div>${groups}</div>
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
}

/** Opens the page a row names. The Bots and Groups pages choose the bot or group themselves when
 * they publish `teamBotOpen(workflow)` / `teamGroupOpen(handle)`; otherwise the row opens the page. */
function tcNavOpen(button) {
  const key = button.dataset.tcNav;
  if (!tcNavOpens(key)) return;
  const opener = key === "team-bot" && button.dataset.workflow ? () => globalThis.teamBotOpen?.(button.dataset.workflow) : key === "team-group" && button.dataset.handle ? () => globalThis.teamGroupOpen?.(button.dataset.handle) : null;
  const published = key === "team-bot" ? globalThis.teamBotOpen : key === "team-group" ? globalThis.teamGroupOpen : null;
  if (opener && published) opener();
  else openPage(key);
}

document.addEventListener("click", (e) => {
  const button = e.target.closest?.("#team-nav [data-tc-nav]");
  if (button) tcNavOpen(button);
});
tcOnChange(() => tcNavDraw());
document.addEventListener("teams:changed", () => tcNavDraw());
document.addEventListener("teams:event", () => tcNavDraw());
