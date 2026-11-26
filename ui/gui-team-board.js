/** The team's work board: every piece of team work is a card that moves from Backlog to Done.
 * Each "Ready" column is a queue and names who picks its cards up: the agent's seat, its worker
 * profile and model, or a person for Ready to deploy. A card shows its round, its one PR, the
 * Approver's score and, when it waits on a person, why, with the act that unblocks it. The owner
 * seats each agent's bot and profile under "Seats". The hub's rules are `work_board.rs`; this
 * page only draws them and sends a member's acts (`add_card`, `card_act`, `set_board_setup`). */

const WB_COLUMNS = [
  ["backlog", "Backlog"],
  ["ready_to_dev", "Ready to dev"],
  ["in_dev", "In dev"],
  ["ready_to_review", "Ready to review"],
  ["in_review", "In review"],
  ["ready_to_deploy", "Ready to deploy"],
  ["done", "Done"],
];
const WB_AGENTS = { builder: "Builder", reviewer: "Reviewer", approver: "Approver" };

/** `draft` and `request` keep what is typed in the add and send-back forms across redraws. */
const wbStore = { team: null, view: null, error: "", busy: new Set(), line: new Map(), adding: false, seats: null, sendBack: null, addLine: "", draft: { title: "", brief: "", project: "" }, request: "" };

const wbName = (agent) => WB_AGENTS[agent] ?? agent;
const wbProfile = (setup, agent) => {
  const name = (setup[agent]?.profile ?? "").trim().toLowerCase();
  return setup.profiles.find((p) => p.name.trim().toLowerCase() === name) ?? null;
};
/** "Implementer · claude-opus-5-5, high". */
function wbProfileLine(setup, agent) {
  const p = wbProfile(setup, agent);
  if (!p) return "no profile";
  const how = [p.model?.model, p.effort].filter(Boolean).join(", ");
  return how ? `${p.name} · ${how}` : p.name;
}
const wbBotName = (workflow) => tcStore.bots.find((b) => b.workflow === workflow)?.name || "a bot";

/** Who picks up a column's cards, as the column's header says it. */
function wbPicksUp(setup, column) {
  const seat = (agent) => {
    const bot = setup[agent]?.bot;
    return { agent, line: wbProfileLine(setup, agent), bot, empty: !bot };
  };
  if (column === "ready_to_dev" || column === "in_dev") return [seat("builder")];
  if (column === "ready_to_review") return [seat("reviewer")];
  if (column === "in_review") return [seat("reviewer"), seat("approver")];
  return [];
}

function wbHeadHtml(setup, column, label, count) {
  const queue = column.startsWith("ready_");
  let who = "";
  if (column === "ready_to_deploy") who = `<div class="wb-who" data-who="person">Picked up by <b>a person</b>, who merges the PR</div>`;
  else if (column === "backlog") who = `<div class="wb-who">A person queues a card</div>`;
  else if (column !== "done") {
    const seats = wbPicksUp(setup, column);
    const verb = queue ? "Picked up by" : "Worked by";
    who = seats
      .map((s, i) => {
        const what = s.agent === "approver" ? `then the <b>Approver</b> scores it (bar ${setup.merge_bar})` : `${i ? "then " : `${verb} `}<b>${esc(wbName(s.agent))}</b>`;
        const bot = s.empty ? `<span class="wb-empty-seat">no bot seated: cards wait here</span>` : `<span class="wb-bot">bot ${esc(wbBotName(s.bot))}</span>`;
        return `<div class="wb-who" data-agent="${s.agent}">${what}<span class="wb-profile" title="Worker profile">${esc(s.line)}</span>${bot}</div>`;
      })
      .join("");
  }
  return `<header class="wb-col-head"><div class="wb-col-name"><span>${esc(label)}</span><span class="wb-count">${count}</span>${queue ? `<span class="wb-kind">queue</span>` : ""}</div>${who}</header>`;
}

function wbActsHtml(card) {
  const busy = wbStore.busy.has(card.id) ? " disabled" : "";
  const act = (name, label, cls = "tc-secondary") => `<button type="button" class="${cls}" data-wb-act="${name}"${busy}>${esc(label)}</button>`;
  const acts = [];
  if (card.column === "backlog") acts.push(act("queue", "Queue", "tc-primary"));
  if (card.person) acts.push(act("resume", card.round >= card.round_limit && card.column === "ready_to_dev" ? "Give more rounds" : "Resume", "tc-primary"));
  if (card.person && card.column === "in_review") acts.push(act("pass", "Pass to deploy"));
  if (card.column === "ready_to_deploy") acts.push(act("merged", "I merged it", "tc-primary"));
  if (["ready_to_review", "in_review", "ready_to_deploy"].includes(card.column)) acts.push(act("send_back", "Send back…", "tc-quiet"));
  return acts.length ? `<div class="tc-actions wb-acts">${acts.join("")}</div>` : "";
}

function wbCardHtml(card, setup) {
  const pr = card.pr ? `<a class="wb-pr" href="${esc(card.pr.url)}" target="_blank" rel="noopener">PR #${card.pr.number}</a>` : "";
  const round = card.round ? `<span class="wb-round" title="Each pick by the Builder is a round">round ${card.round} of ${card.round_limit}</span>` : "";
  const v = card.verdict;
  const score = v ? `<span class="wb-score" data-pass="${v.score >= v.bar}" title="${esc(v.reason)}">${v.score}/${v.bar}</span>` : "";
  const working = card.run && card.turn ? `<div class="wb-working"><span class="tc-dot" data-tone="on"></span>${esc(wbName(card.turn))} is working</div>` : "";
  const waitingFor = !card.run && !card.person && card.turn && card.column === "in_review" ? `<div class="wb-working wb-next">Next: the ${esc(wbName(card.turn))}</div>` : "";
  const person = card.person ? `<div class="wb-person" role="status">${esc(card.person)}</div>` : "";
  const last = card.requests.at(-1);
  const request = last && ["ready_to_dev", "in_dev"].includes(card.column) ? `<div class="wb-request"><span>${esc(last.by.kind === "agent" ? wbName(last.by.agent) : "A person")} asked:</span> ${esc(last.text)}</div>` : "";
  const line = wbStore.line.get(card.id);
  const form = wbStore.sendBack === card.id ? `<form class="tc-typed wb-sendback" data-wb-form="send_back"><input name="request" value="${esc(wbStore.request)}" placeholder="What should the Builder fix?" aria-label="What should the Builder fix?" autocomplete="off"><button type="submit" class="tc-secondary">Send back</button></form>` : "";
  return `<article class="wb-card" data-card="${esc(card.id)}" data-state="${card.person ? "person" : card.run ? "working" : "idle"}">
    <div class="wb-card-title">${esc(card.title)}</div>
    ${card.project ? `<div class="wb-meta">${esc(card.project.split("/").pop())}</div>` : ""}
    ${pr || round || score ? `<div class="wb-meta wb-chips">${pr}${round}${score}</div>` : ""}
    ${working}${waitingFor}${person}${request}
    ${wbActsHtml(card)}${form}${line ? `<div class="tc-note" role="alert">${esc(line)}</div>` : ""}
  </article>`;
}

function wbAddHtml() {
  const d = wbStore.draft;
  const projects = tcStore.projects.map((p) => `<option value="${esc(p.key)}"${d.project === p.key ? " selected" : ""}>${esc(p.name || p.key)}</option>`).join("");
  return `<form class="wb-add" data-wb-form="add">
    <input name="title" value="${esc(d.title)}" placeholder="A new card: what should be built?" aria-label="Card title" autocomplete="off" required>
    <input name="brief" value="${esc(d.brief)}" placeholder="Details for the Builder (optional)" aria-label="Details" autocomplete="off">
    ${projects ? `<select name="project" aria-label="Project">${projects}</select>` : ""}
    <button type="submit" class="tc-primary"${wbStore.adding ? " disabled" : ""}>Add to Backlog</button>
    ${wbStore.addLine ? `<span class="tc-note" role="alert">${esc(wbStore.addLine)}</span>` : ""}
  </form>`;
}

function wbSeatsHtml(setup) {
  if (!tcMe().owner) return "";
  const bots = (agent) =>
    `<option value="">No bot</option>` + tcStore.bots.map((b) => `<option value="${esc(b.workflow)}"${setup[agent].bot === b.workflow ? " selected" : ""}>${esc(b.name || "Untitled bot")}</option>`).join("");
  const profiles = (agent) => setup.profiles.map((p) => `<option${p.name === setup[agent].profile ? " selected" : ""}>${esc(p.name)}</option>`).join("");
  const rows = Object.keys(WB_AGENTS)
    .map((agent) => `<label class="wb-seat"><b>${wbName(agent)}</b><select name="${agent}-profile" aria-label="${wbName(agent)} profile">${profiles(agent)}</select><select name="${agent}-bot" aria-label="${wbName(agent)} bot">${bots(agent)}</select></label>`)
    .join("");
  return `<details class="wb-seats"${wbStore.seats ? " open" : ""}><summary>Seats: who picks up each queue</summary>
    <form data-wb-form="seats">${rows}<label class="wb-seat"><b>Merge bar</b><input name="bar" type="number" min="0" max="100" value="${setup.merge_bar}"></label>
    <button type="submit" class="tc-secondary">Save seats</button>${wbStore.seats?.line ? `<span class="tc-note" role="alert">${esc(wbStore.seats.line)}</span>` : ""}</form></details>`;
}

function wbPageHtml() {
  if (!tcCurrent()) return `<p class="tc-empty">Create or join a team to see its board.</p>`;
  const view = wbStore.view;
  if (!view) return `<p class="tc-empty" role="status">${esc(wbStore.error || "Loading…")}</p>`;
  const columns = WB_COLUMNS.map(([column, label]) => {
    const cards = view.cards.filter((c) => c.column === column);
    return `<section class="wb-col" data-column="${column}" aria-label="${esc(label)}">${wbHeadHtml(view.setup, column, label, cards.length)}<div class="wb-cards">${cards.map((c) => wbCardHtml(c, view.setup)).join("")}</div></section>`;
  }).join("");
  return `<div class="wb-top"><div class="tc-title"><h1>Board</h1><p>Ready columns are queues. Each names who picks its cards up: the agent, its worker profile and model. A person merges.</p></div>${wbAddHtml()}${wbSeatsHtml(view.setup)}</div>
    <div class="wb-columns">${columns}</div>`;
}

async function wbLoad() {
  const team = tcCurrent();
  if (!team) return;
  if (wbStore.team !== team.id) Object.assign(wbStore, { team: team.id, view: null, error: "" });
  try {
    wbStore.view = await tcCall({ op: "board" });
    wbStore.error = "";
  } catch (e) {
    wbStore.error = String(e);
  }
  wbDraw();
}

function wbMerge(card) {
  if (!wbStore.view) return;
  const at = wbStore.view.cards.findIndex((c) => c.id === card.id);
  if (at >= 0) wbStore.view.cards[at] = card;
  else wbStore.view.cards.push(card);
}

/** Redraws the page when it changed, keeping the focused field and its caret. */
function wbDraw() {
  if (shellPage() !== "team-board") return;
  const el = pageView.querySelector(".wb-wrap");
  const html = el && wbPageHtml();
  if (!el || el.dataset.wbHtml === html) return;
  const focused = document.activeElement && el.contains(document.activeElement) && document.activeElement.name ? document.activeElement : null;
  const keep = focused && { name: focused.name, form: focused.form?.dataset.wbForm, at: focused.selectionStart };
  el.dataset.wbHtml = html;
  el.innerHTML = html;
  const back = keep && el.querySelector(`[data-wb-form="${keep.form}"] [name="${keep.name}"]`);
  back?.focus();
  if (back && keep.at != null && back.setSelectionRange) back.setSelectionRange(keep.at, keep.at);
}

async function wbAct(id, act) {
  wbStore.busy.add(id);
  wbStore.line.delete(id);
  wbDraw();
  try {
    wbMerge(await tcCall({ op: "card_act", card: id, act }));
    if (wbStore.sendBack === id) Object.assign(wbStore, { sendBack: null, request: "" });
  } catch (e) {
    wbStore.line.set(id, String(e));
  }
  wbStore.busy.delete(id);
  wbDraw();
}

async function wbSubmit(form) {
  const data = Object.fromEntries(new FormData(form));
  if (form.dataset.wbForm === "add") {
    wbStore.adding = true;
    wbDraw();
    try {
      wbMerge(await tcCall({ op: "add_card", title: data.title, brief: data.brief ?? "", project: data.project ?? "" }));
      wbStore.draft = { title: "", brief: "", project: data.project ?? "" };
      wbStore.addLine = "";
    } catch (e) {
      wbStore.addLine = String(e);
    }
    wbStore.adding = false;
    wbDraw();
  } else if (form.dataset.wbForm === "send_back") {
    wbAct(form.closest("[data-card]").dataset.card, { act: "send_back", request: data.request ?? "" });
  } else if (form.dataset.wbForm === "seats") {
    const setup = structuredClone(wbStore.view.setup);
    for (const agent of Object.keys(WB_AGENTS)) {
      setup[agent].profile = data[`${agent}-profile`];
      setup[agent].bot = data[`${agent}-bot`] || undefined;
    }
    setup.merge_bar = Number(data.bar);
    wbStore.seats = { line: "" };
    try {
      wbStore.view.setup = await tcCall({ op: "set_board_setup", setup });
      wbStore.seats = { line: "Saved." };
    } catch (e) {
      wbStore.seats = { line: String(e) };
    }
    wbDraw();
  }
}

tcOnChange(wbDraw);
function wbShow() {
  setPage(`${headHtml("Board")}<div class="page-body"><div class="wb-wrap" data-tc-page="board"></div></div>`);
  wbDraw();
  wbLoad();
  tcRefresh();
}

pageView.addEventListener("click", (e) => {
  if (shellPage() !== "team-board") return;
  const button = e.target.closest("[data-wb-act]");
  if (!button || button.disabled) return;
  const id = button.closest("[data-card]").dataset.card;
  if (button.dataset.wbAct === "send_back") {
    wbStore.sendBack = wbStore.sendBack === id ? null : id;
    return wbDraw();
  }
  wbAct(id, { act: button.dataset.wbAct });
});
pageView.addEventListener("submit", (e) => {
  if (shellPage() !== "team-board" || !e.target.dataset?.wbForm) return;
  e.preventDefault();
  wbSubmit(e.target);
});
pageView.addEventListener("input", (e) => {
  const form = e.target.form?.dataset.wbForm;
  if (form === "add") wbStore.draft[e.target.name] = e.target.value;
  else if (form === "send_back") wbStore.request = e.target.value;
});
pageView.addEventListener("change", (e) => e.target.form?.dataset.wbForm === "add" && (wbStore.draft[e.target.name] = e.target.value));
pageView.addEventListener("toggle", (e) => e.target.classList?.contains("wb-seats") && (wbStore.seats = e.target.open ? wbStore.seats ?? { line: "" } : null), true);

if (typeof document !== "undefined") {
  document.addEventListener("teams:event", (e) => {
    const frame = e.detail;
    if (frame?.team !== wbStore.team || frame.event?.kind !== "card_changed") return;
    wbMerge(frame.event.card);
    wbDraw();
  });
}

SHELL_PAGES["team-board"] = { show: wbShow };
