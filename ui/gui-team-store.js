/** What the team screens (nav, calls, run) share: the current team's calls, runs, bots, groups and
 * members as the hub answered them, kept live by the hub's pushes, and the pure rules that read them.
 * `teamsCurrent()` and `teamCall(op)` come from gui-teams.js; without a team nothing here fetches.
 *
 * Everything people or bots typed (names, questions, answers, log lines) stays text here and is
 * escaped by the screens that draw it. Names in this file are prefixed `tc` because every classic
 * script shares one scope. */

const TC_ESCALATE_MIN = 30;
const TC_GIVE_UP_HOURS = 2;

const tcStore = {
  team: null,
  calls: [],
  runs: [],
  bots: [],
  groups: [],
  members: [],
  /** The team's attached projects (`TeamProject`), for the nav's list. */
  projects: [],
  /** A project being taken out of the team: `{ key, state: "asking" | "sending" | "error", line }`, or null. */
  detach: null,
  loaded: false,
  /** What an answer from this page came to, by call id: `{ state: "sending" | "done" | "taken" | "error", line, kind }`. */
  answers: new Map(),
  /** Call ids this page showed as open, so one someone else answered stays as a quiet card. */
  seen: new Set(),
  /** Runs this page stopped, for "Stopped by you". */
  stopped: new Set(),
  /** The last log line of working runs, and each run's log: `run id → [{ at, who, text }] | null` (null: the hub has no log). */
  logs: new Map(),
  /** The hub's words when `run_log` failed for a run. */
  logErrors: new Map(),
};
const tcListeners = new Set();
const tcNotify = () => tcListeners.forEach((fn) => fn());
/** Calls `fn` on every change of the store; returns the way to stop. */
const tcOnChange = (fn) => (tcListeners.add(fn), () => tcListeners.delete(fn));

const tcCurrent = () => {
  const team = globalThis.teamsCurrent();
  return team && team.id ? team : null;
};
const tcCall = (op) => Promise.resolve().then(() => globalThis.teamCall(op));
const tcNow = () => Date.now() / 1000;
const tcLocale = () => (typeof APP_LOCALE === "undefined" ? undefined : APP_LOCALE);
const tcClock = (unix) => new Date(unix * 1000).toLocaleTimeString(tcLocale(), { hour: "numeric", minute: "2-digit" });
const tcMidnight = () => {
  const d = new Date(tcNow() * 1000);
  d.setHours(0, 0, 0, 0);
  return d.getTime() / 1000;
};
/** "just now", "6 min ago", "2 h ago", "3 d ago". */
function tcAgo(unix) {
  const s = Math.max(0, tcNow() - unix);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}
/** A time of day, with the date when it is not today. */
function tcWhen(unix) {
  const day = new Date(unix * 1000).toLocaleDateString(tcLocale(), { month: "short", day: "numeric" });
  return unix >= tcMidnight() ? tcClock(unix) : `${day}, ${tcClock(unix)}`;
}
const tcPlural = (n, one, many) => `${n} ${n === 1 ? one : many}`;

/** Initials for a name: the first letters of its first two words, or its first two letters. */
function tcInitials(name) {
  const words = String(name ?? "").trim().split(/[\s._-]+/).filter(Boolean);
  const letters = words.length > 1 ? words[0][0] + words[1][0] : (words[0] ?? "?").slice(0, 2);
  return letters.toUpperCase();
}
/** A person's name from the email the hub knows them by: `ana.souza@acme.com` is "Ana Souza". */
function tcNameOf(email) {
  const local = String(email ?? "").split("@")[0];
  const words = local.split(/[._+-]+/).filter(Boolean);
  return words.map((w) => w[0].toUpperCase() + w.slice(1)).join(" ") || String(email ?? "");
}
const tcHue = (text) => [...String(text)].reduce((h, c) => (h * 31 + c.codePointAt(0)) % 360, 7);

/** Who the signed-in person is among the team's members: by `teamsCurrent().me`, their own account id.
 * The wire gives that id only for the owner, so a member is found by the e-mail they signed in with. */
function tcMe() {
  const team = tcCurrent();
  const email = (typeof account !== "undefined" && account?.email ? account.email : "").toLowerCase();
  const member = (team?.me ? tcStore.members.find((m) => m.account === team.me) : null) ?? (email ? tcStore.members.find((m) => String(m.email).toLowerCase() === email) : null);
  return { account: member?.account ?? team?.me ?? null, name: member ? tcNameOf(member.email) : email ? tcNameOf(email) : "You", owner: (member?.role ?? team?.role) === "owner" };
}
function tcOwner() {
  const member = tcStore.members.find((m) => m.role === "owner");
  return member ? { account: member.account, name: tcNameOf(member.email) } : null;
}
function tcPerson(accountId) {
  if (!accountId) return "";
  if (accountId === tcMe().account) return "You";
  const member = tcStore.members.find((m) => m.account === accountId);
  return member ? tcNameOf(member.email) : "A former member";
}
const tcBotOf = (workflow) => tcStore.bots.find((b) => b.workflow === workflow) ?? null;
const tcBotName = (workflow) => tcBotOf(workflow)?.name || "Bot";
const tcGroupMembers = (handle) => tcStore.groups.find((g) => g.handle === handle)?.members ?? [];

/** The accounts one `Who` reaches: a person, a group's members, or the owner. A group with nobody
 * left in the team goes to its fallback. */
function tcWhoAccounts(who) {
  if (who.kind === "person") return [who.account];
  if (who.kind === "owner") return [tcOwner()?.account].filter(Boolean);
  const group = tcStore.groups.find((g) => g.handle === who.handle);
  const inTeam = (group?.members ?? []).filter((a) => tcStore.members.some((m) => m.account === a));
  return inTeam.length || !group?.fallback ? inTeam : [group.fallback];
}
/** Every account a call reaches, the owner too once it escalated. */
function tcCallAccounts(call) {
  const all = call.asked.flatMap(tcWhoAccounts);
  if (call.escalated) all.push(...tcWhoAccounts({ kind: "owner" }));
  return [...new Set(all)];
}
/** Whether the call asks the signed-in person: named, in a group it names, or the owner for the owner and escalated calls. */
function tcAsksMe(call) {
  const me = tcMe();
  const named = call.asked.some((w) => (w.kind === "person" ? !!me.account && w.account === me.account : w.kind === "group" ? !!me.account && tcWhoAccounts(w).includes(me.account) : me.owner));
  return named || (call.escalated && me.owner);
}
const tcOpen = (call) => !call.answer;
const tcRunEnded = (runId) => ["done", "stopped", "gave_up", "failed"].includes(tcStore.runs.find((r) => r.id === runId)?.state);

/** The open calls that ask me, newest first. */
const tcCallsForMe = () => tcStore.calls.filter((c) => tcOpen(c) && !tcRunEnded(c.run) && tcAsksMe(c)).sort((a, b) => b.created_at - a.created_at);
const tcOpenFor = (workflow) => tcStore.calls.some((c) => tcOpen(c) && c.workflow === workflow);

/** The other groups of the bot that a call can be passed to: the ones its call rules name that the call did not ask. */
function tcPassTargets(call) {
  const rules = tcBotOf(call.workflow)?.calls ?? {};
  const asked = new Set(call.asked.filter((w) => w.kind === "group").map((w) => w.handle));
  const handles = Object.values(rules).filter((w) => w?.kind === "group").map((w) => w.handle);
  return [...new Set(handles)].filter((h) => !asked.has(h) && (!tcStore.groups.length || tcStore.groups.some((g) => g.handle === h)));
}

/** How a `Who` reads on a chip: `@design` for a group, the person's name, the owner's name. */
function tcWhoLabel(who) {
  if (who.kind === "group") return `@${who.handle}`;
  const name = who.kind === "person" ? tcPerson(who.account) : tcOwner()?.name;
  return name ? `@${name === "You" ? tcMe().name : name}` : "the owner";
}
/** The people a call reached, other than me, by name. */
function tcOthersAsked(call) {
  const me = tcMe().account;
  return tcCallAccounts(call).filter((a) => a !== me).map((a) => tcPerson(a));
}
function tcNameList(names) {
  if (names.length <= 1) return names[0] ?? "";
  if (names.length === 2) return `${names[0]} and ${names[1]}`;
  return `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`;
}
/** "Ana was called too." for the other people asked; "" when it is only me. */
function tcCalledToo(call) {
  const others = tcOthersAsked(call);
  if (!others.length) return "";
  const shown = others.length > 3 ? [...others.slice(0, 2), `${others.length - 2} others`] : others;
  return `${tcNameList(shown)} ${others.length === 1 ? "was" : "were"} called too.`;
}

/** The labels of the hub's never list (`NeverItem::label`): a step the bot must not take alone is asked as
 * "<label>: <step>", the step being the masked command, or a non-Bash tool's name and input as JSON. */
const TC_NEVER = {
  "delete outside the worktree": "Delete outside the worktree",
  "force-push": "Force-push",
  "push to main": "Push to main",
  release: "Release",
  deploy: "Deploy",
  production: "Production",
  secrets: "Secrets",
  payment: "Payment",
  "write outside the worktree": "Write outside the worktree",
  "self-grant": "Change its own permissions",
  "network install": "Network install",
  "machine-wide change": "Machine-wide change",
};
const TC_STEP = new RegExp(`^(${Object.keys(TC_NEVER).join("|")}): ([\\s\\S]+)$`);

/** The step a before-irreversible call asks about, whole and as sent: `{ label, tool, input }`, or null for any other question. */
function tcStepOf(call) {
  if (call.situation !== "before_irreversible") return null;
  const m = TC_STEP.exec(String(call.question ?? ""));
  if (!m) return null;
  const tool = /^([A-Za-z][\w.-]*) (\{[\s\S]*\})$/.exec(m[2]);
  let json = null;
  try {
    json = tool ? JSON.parse(tool[2]) : null;
  } catch {}
  if (json && typeof json === "object") return { label: TC_NEVER[m[1]], tool: tool[1], input: tool[2] };
  return { label: TC_NEVER[m[1]], tool: "Bash", input: m[2] };
}
/** The question as the card draws it: the bot's words as plain text, and for a step the label, then the tool and the whole step in a block that wraps and never clamps. */
function tcQuestionHtml(call, tag, cls) {
  const step = tcStepOf(call);
  if (!step) return `<${tag} class="${cls}">${esc(call.question)}</${tag}>`;
  return `<${tag} class="${cls}">${esc(step.label)}</${tag}><div class="tc-step" data-step><span class="tc-step-tool">${esc(step.tool)}</span><div class="tc-step-text">${esc(step.input)}</div></div>`;
}

/** Whether the call was closed because its run ended: the hub answers it as a stop from the team's own account, with the cause as text. */
const tcClosedByRun = (call) => call.answer?.kind?.kind === "stop" && !!tcStore.team && call.answer.by === tcStore.team;
/** "Closed: the run ended", then the hub's cause as sent. */
const tcClosedText = (call) => ["Closed: the run ended", String(call.answer?.text ?? "").trim()].filter(Boolean).join(" · ");

/** What an answer reads as: `{ who, verb, text }` where `verb` completes "Ana already ..." and
 * `text` is what the person said, for a timeline. Null for an open call. */
function tcAnswerText(call) {
  const a = call.answer;
  if (!a) return null;
  if (tcClosedByRun(call)) return { who: "", verb: "", text: tcClosedText(call), closed: true };
  const who = tcPerson(a.by) || "Someone";
  switch (a.kind?.kind) {
    case "pass_to":
      return { who, verb: `passed it to @${a.kind.handle}`, text: `Passed it to @${a.kind.handle}.` };
    case "take_over":
      return { who, verb: "took over the run", text: "Took over the run." };
    case "stop":
      return { who, verb: "stopped the run", text: "Stopped the run." };
    default: {
      const said = [a.choice, a.text].filter(Boolean).join(": ");
      return { who, verb: said ? `answered “${said}”` : "answered", text: said || "Answered." };
    }
  }
}

/** The answer a button, a pass or a typed line sends. The hub sets `by` and `at`. */
const tcChoiceAnswer = (choice) => ({ kind: { kind: "choice" }, choice });
const tcTextAnswer = (text) => ({ kind: { kind: "choice" }, text });
const tcPassAnswer = (handle) => ({ kind: { kind: "pass_to", handle } });

/** The line on a card once I answered: what I said and what happens next. */
function tcDoneLine(answer) {
  if (answer.kind === "pass_to") {
    const names = tcGroupMembers(answer.handle).filter((a) => tcStore.members.some((m) => m.account === a)).map((a) => tcPerson(a));
    return `Passed to @${answer.handle}. ${names.length ? `${tcNameList(names)} ${names.length === 1 ? "is" : "are"} being called.` : "The people in it are being called."}`;
  }
  return `You answered “${(answer.text || answer.choice || "").replace(/\.$/, "")}”. The bot goes on with it.`;
}

function tcMerge(list, item, key = "id") {
  const at = list.findIndex((x) => x[key] === item[key]);
  return at < 0 ? [item, ...list] : list.map((x, i) => (i === at ? item : x));
}

/** Sends an answer for `call`. The card keeps its line either way: green once it went through, and
 * the hub's own words when it did not (someone else answered first, for one). */
async function tcAnswer(call, answer) {
  const team = tcStore.team;
  tcStore.answers.set(call.id, { state: "sending" });
  tcNotify();
  try {
    const reply = await tcCall({ op: "answer_call", id: call.id, answer });
    if (tcStore.team !== team) return;
    const said = answer.kind.kind === "pass_to" ? { kind: "pass_to", handle: answer.kind.handle } : { kind: "choice", choice: answer.choice, text: answer.text };
    tcStore.answers.set(call.id, { state: "done", line: tcDoneLine(said), by: "me" });
    if (reply && typeof reply === "object" && reply.id === call.id) tcStore.calls = tcMerge(tcStore.calls, reply);
  } catch (e) {
    if (tcStore.team !== team) return;
    const message = String(e?.message ?? e);
    tcStore.answers.set(call.id, { state: /already/i.test(message) ? "taken" : "error", line: /already/i.test(message) ? message : `Could not send: ${message}` });
    tcRefresh();
  }
  tcNotify();
}

/** `take_over`, `give_back` or `stop_run` for a run; the hub answers the run as it is now. */
async function tcRunOp(op, run) {
  const team = tcStore.team;
  try {
    const reply = await tcCall({ op, run });
    if (tcStore.team !== team) return null;
    if (op === "stop_run") tcStore.stopped.add(run);
    if (reply && typeof reply === "object" && reply.id === run) tcStore.runs = tcMerge(tcStore.runs, reply);
    tcNotify();
    return reply;
  } catch (e) {
    tcStore.runError = { run, line: `Could not ${op === "take_over" ? "take over" : op === "give_back" ? "give it back" : "stop the bot"}: ${String(e?.message ?? e)}` };
    tcNotify();
    return null;
  }
}

/** Whether the viewer may take `project` out of the team: the member who attached it, or the owner. */
function tcCanDetach(project) {
  const me = tcMe();
  return me.owner || (!!me.account && project.member === me.account);
}
/** Asks in place before a project goes out; `null` closes the question. */
function tcDetachAsk(key) {
  tcStore.detach = key ? { key, state: "asking", line: "" } : null;
  tcNotify();
}
/** Sends `detach_project` for the project being asked about. The list is read again after, since the bots that worked in it changed. */
async function tcDetachConfirm() {
  const asked = tcStore.detach;
  if (!asked || asked.state === "sending") return;
  const team = tcStore.team;
  tcStore.detach = { ...asked, state: "sending", line: "" };
  tcNotify();
  try {
    await tcCall({ op: "detach_project", key: asked.key });
    if (tcStore.team !== team) return;
    tcStore.projects = tcStore.projects.filter((p) => p.key !== asked.key);
    tcStore.detach = null;
    tcNotify();
    tcRefresh();
  } catch (e) {
    if (tcStore.team !== team) return;
    tcStore.detach = { ...asked, state: "error", line: `Could not take it out: ${String(e?.message ?? e)}` };
    tcNotify();
  }
}

/** One run's log: `[{ at, who, text }]`, or null when `run_log` failed (its words are in `tcStore.logErrors`). */
async function tcLoadLog(run) {
  const team = tcStore.team;
  let log = null;
  let failed = "";
  try {
    const rows = await tcCall({ op: "run_log", run });
    if (Array.isArray(rows)) log = rows.filter((r) => r && typeof r.text === "string").map((r) => ({ at: Number(r.at) || 0, who: typeof r.who === "string" ? r.who : "", text: r.text }));
  } catch (e) {
    failed = String(e?.message ?? e ?? "");
  }
  if (tcStore.team !== team) return null;
  tcStore.logs.set(run, log);
  if (failed) tcStore.logErrors.set(run, failed);
  else tcStore.logErrors.delete(run);
  tcNotify();
  return log;
}
const tcLastLine = (run) => tcStore.logs.get(run)?.at(-1)?.text ?? "";
/** Loads the log of every working run, which the calls page shows the last line of. */
const tcLoadWorkingLogs = () => Promise.all(tcStore.runs.filter((r) => r.state === "working").slice(0, 8).map((r) => tcLoadLog(r.id)));

let tcRefreshing = 0;
/** Reads the team's lists from the hub. A list that fails keeps what it had. */
async function tcRefresh() {
  const team = tcCurrent();
  if (!team) return tcReset(null);
  if (tcStore.team !== team.id) tcReset(team.id);
  const ticket = ++tcRefreshing;
  const lists = { calls: "calls", runs: "runs", bots: "bots", groups: "groups", members: "members", projects: "projects" };
  const got = await Promise.all(Object.values(lists).map((op) => tcCall({ op }).then((v) => (Array.isArray(v) ? v : null), () => null)));
  if (ticket !== tcRefreshing || tcStore.team !== team.id) return;
  Object.keys(lists).forEach((key, i) => got[i] && (tcStore[key] = got[i]));
  tcStore.loaded = tcStore.loaded || got.some(Boolean);
  tcNotify();
  tcLoadWorkingLogs();
}
function tcReset(team) {
  tcStore.team = team;
  Object.assign(tcStore, { calls: [], runs: [], bots: [], groups: [], members: [], projects: [], detach: null, loaded: false, runError: null });
  for (const key of ["answers", "seen", "stopped", "logs", "logErrors"]) tcStore[key].clear();
  tcNotify();
}

/** A push from the hub: `{ type: "team", team, event }`. */
function tcApply(frame) {
  if (!frame || frame.team !== tcStore.team || !frame.event) return;
  const { event } = frame;
  if (event.kind === "call_opened" || event.kind === "call_resolved") {
    tcStore.calls = tcMerge(tcStore.calls, event.call);
    if (tcStore.logs.has(event.call.run)) tcLoadLog(event.call.run);
  } else if (event.kind === "run_changed") {
    tcStore.runs = tcMerge(tcStore.runs, event.run);
    if (event.run.state === "working" || tcStore.logs.has(event.run.id)) tcLoadLog(event.run.id);
  } else return;
  tcNotify();
}

if (typeof document !== "undefined") {
  document.addEventListener("teams:changed", () => tcRefresh());
  document.addEventListener("teams:event", (e) => tcApply(e.detail));
  // What changed while the hub was away is not pushed again, so a hub that comes back is read afresh.
  if (typeof MutationObserver !== "undefined") new MutationObserver(() => document.body.dataset.hub === "online" && tcCurrent() && tcRefresh()).observe(document.body, { attributeFilter: ["data-hub"] });
}

/** The bot's avatar: its initials in a rounded square. */
const tcBotMark = (name, cls = "") => `<span class="tc-mark ${cls}" aria-hidden="true">${esc(tcInitials(name))}</span>`;
/** A person's avatar: initials on a colour taken from their id. */
const tcPersonMark = (accountId, cls = "") => `<span class="tc-mark tc-person ${cls}" style="--tc-hue:${tcHue(accountId || "?")}" aria-hidden="true">${esc(tcInitials(tcPerson(accountId) === "You" ? tcMe().name : tcPerson(accountId)))}</span>`;
const tcChip = (label, handle) => `<span class="tc-chip" data-handle="${esc(handle ?? "")}">${esc(label)}</span>`;

/** The controls of an open call that asks me: one button per choice (the first is primary), a pass
 * to each other group of the bot, and a field for a typed answer when the call has no choices or
 * `typed` asks for one (the person who took the run over answers in their own words). */
function tcActionsHtml(call, { typed = false, disabled = false, extra = "" } = {}) {
  const choices = call.choices.map((c, i) => `<button type="button" class="${i === 0 ? "tc-primary" : "tc-secondary"}" data-tc="choice" data-choice="${i}">${esc(c)}</button>`);
  const passes = tcPassTargets(call).map((h) => `<button type="button" class="tc-quiet" data-tc="pass" data-handle="${esc(h)}">Pass to @${esc(h)}</button>`);
  const form = !call.choices.length || typed ? `<form class="tc-typed" data-tc-form><input type="text" name="answer" placeholder="Type an answer" aria-label="Type an answer" autocomplete="off" enterkeyhint="send"><button type="submit" class="tc-secondary">Send</button></form>` : "";
  const buttons = choices.length || passes.length || extra ? `<div class="tc-actions">${choices.join("")}${passes.join("")}${extra}</div>` : "";
  return `<fieldset class="tc-fields"${disabled ? " disabled" : ""}>${buttons}${form}</fieldset>`;
}

/** The line a card ends on: what I answered, the hub's words when it refused, or what someone else answered. */
function tcResultHtml(call) {
  const mine = tcStore.answers.get(call.id);
  if (mine?.state === "sending") return `<div class="tc-result" data-state="sending" role="status">Sending…</div>`;
  if (mine) return `<div class="tc-result" data-state="${mine.state}" role="status">${esc(mine.line)}</div>`;
  const answered = tcAnswerText(call);
  if (answered?.closed) return `<div class="tc-result" data-state="other" role="status">${esc(answered.text)}</div>`;
  return answered ? `<div class="tc-result" data-state="other" role="status">${esc(`${answered.who} already ${answered.verb}.`)}</div>` : "";
}

/** The typed answer in a field, by call id, so a repaint keeps it. */
const tcTyped = new Map();

/** What a click on a call's controls does. `root` holds the controls; `calls` finds the call by id. */
function tcHandleClick(e, find) {
  const button = e.target.closest("[data-tc]");
  if (!button || button.disabled) return false;
  const call = find(button.closest("[data-call]")?.dataset.call);
  if (!call) return false;
  if (button.dataset.tc === "choice") tcAnswer(call, tcChoiceAnswer(call.choices[Number(button.dataset.choice)]));
  else if (button.dataset.tc === "pass") tcAnswer(call, tcPassAnswer(button.dataset.handle));
  else return false;
  return true;
}
function tcHandleSubmit(e, find) {
  const form = e.target.closest("[data-tc-form]");
  if (!form) return false;
  e.preventDefault();
  const call = find(form.closest("[data-call]")?.dataset.call);
  const text = form.answer.value.trim();
  if (call && text) {
    tcTyped.delete(call.id);
    tcAnswer(call, tcTextAnswer(text));
  }
  return true;
}
function tcHandleInput(e) {
  const form = e.target.closest?.("[data-tc-form]");
  const id = form?.closest("[data-call]")?.dataset.call;
  if (id) tcTyped.set(id, e.target.value);
}
/** Puts `html` in `el` when it changed, keeping typed answers and the focused control. */
function tcPaint(el, html) {
  if (el.dataset.tcHtml === html) return;
  const focus = document.activeElement && el.contains(document.activeElement) ? { id: document.activeElement.closest("[data-call]")?.dataset.call, input: document.activeElement.matches("input"), act: document.activeElement.dataset.tc, choice: document.activeElement.dataset.choice } : null;
  el.dataset.tcHtml = html;
  el.innerHTML = html;
  for (const form of el.querySelectorAll("[data-tc-form]")) {
    const id = form.closest("[data-call]")?.dataset.call;
    if (tcTyped.has(id)) form.answer.value = tcTyped.get(id);
  }
  if (focus?.id) {
    const card = [...el.querySelectorAll("[data-call]")].find((c) => c.dataset.call === focus.id);
    (focus.input ? card?.querySelector("input") : card?.querySelector(`[data-tc="${focus.act}"]${focus.choice !== undefined ? `[data-choice="${focus.choice}"]` : ""}`))?.focus();
  }
}

/** Whether a call's controls show: while it is open and asks me, and after an answer that did not go through. */
function tcCanAnswer(call) {
  const mine = tcStore.answers.get(call.id);
  return tcOpen(call) && !tcRunEnded(call.run) && tcAsksMe(call) && (!mine || mine.state === "sending" || mine.state === "error");
}

const tcPageOpens = (key) => typeof SHELL_PAGES !== "undefined" && key in SHELL_PAGES;
/** The run page opens the run it was last given; `openTeamRun(id)` is how another screen sends one. */
let tcRunShown = null;
function openTeamRun(id) {
  tcRunShown = id;
  if (tcPageOpens("team-run")) openPage("team-run");
}
/** A name that links to the bot's or group's page when that page exists, and is plain text when not. */
function tcLinkHtml(kind, value, label, cls) {
  const key = kind === "run" ? "team-run" : `team-${kind}`;
  return tcPageOpens(key) ? `<button type="button" class="${cls} tc-link" data-tc-open="${kind}" data-value="${esc(value)}">${esc(label)}</button>` : `<span class="${cls}">${esc(label)}</span>`;
}
/** What a link made by `tcLinkHtml` does. */
function tcOpenTarget(button) {
  const { tcOpen: kind, value } = button.dataset;
  if (kind === "run") openTeamRun(value);
  else if (kind === "bot") openTeamBot(value);
  else if (kind === "group") openTeamGroup(value);
}
/** Everyone a call went to, as chips: the groups by `@handle`, people by `@Name`, the owner once it escalated. */
function tcAskedChips(call) {
  const who = call.escalated && !call.asked.some((w) => w.kind === "owner") ? [...call.asked, { kind: "owner" }] : call.asked;
  return who.map((w) => (w.kind === "group" ? (tcPageOpens("team-group") ? tcLinkHtml("group", w.handle, `@${w.handle}`, "tc-chip") : tcChip(`@${w.handle}`, w.handle)) : tcChip(tcWhoLabel(w)))).join("");
}
