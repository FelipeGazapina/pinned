/** pinned Teams, bots: the page of one bot (`team-bot`), the Create a bot wizard (`team-bot-new`) and the
 * page of one group (`team-group`). A team call is the hub's `team` call (crates/core/src/teams.rs); the
 * contract with the other Teams files is `teamCall(op)` and `teamsCurrent()`, both read at the time of
 * the call so this file loads in any order and runs without them (the pages then ask for a team).
 *
 * Opening a page: `openTeamBot(workflow)`, `openTeamGroup(handle)` and `openTeamBotNew()`.
 *
 * A bot is a workflow of the team's store plus a `Bot` row. It is born a draft (off) and runs on its
 * own only after the owner, or someone its call rules name, turns it on. Text people or bots typed
 * (names, steps, handles, about lines) reaches the DOM through `esc()` only. */

const TB_TEXT = {
  noTeam: "Choose a team in the account menu to see its bots and groups.",
  noTeamTitle: "No team chosen",
  loading: "Loading…",
  loadFailed: (e) => `Could not load: ${e}`,
  retry: "Try again",
  noHub: "The hub did not answer.",
  unavailable: "Teams is not available in this build.",
  you: "you",
  youCap: "You",
  owner: "Owner",
  backToTeam: "Back to the team",

  bot: {
    pageTitle: "Bot",
    notFound: "This team has no such bot.",
    of: (project, owner) => `${project ? `Bot of ${project}. ` : ""}${owner ? `Owner: ${owner}.` : ""}`.trim(),
    draft: "Draft",
    onTag: "On",
    once: "Runs once, then is discarded",
    runNow: "Run now",
    starting: "Starting…",
    follow: "Running, follow it",
    runNowDraft: "Turn the bot on first: a draft does not run.",
    turnOff: "Turn the bot off",
    turnOn: "Off, turn it on",
    switching: "Working…",
    cannotTurnOn: (who) => `Ask ${who} to turn it on.`,
    needKeys: "The team needs its Upstash key and Claude token before a bot can run. The owner connects them in the team's settings.",
    does: "What it does",
    doesManual: "What it does, only when someone runs it",
    noSteps: "This bot has no steps written in its workflow.",
    calls: "Who it calls",
    moments: {
      before_irreversible: "Before something that can't be undone",
      in_doubt: "When in doubt",
      on_limit: "At a limit",
      on_failure: "On failure",
      limit_or_failure: "At a limit or on failure",
    },
    limits: (usd, minutes) => `up to US$ ${usd} or ${minutes} minutes per run`,
    callsNote: "Outside those moments it works alone, and anyone on the team can follow or take over.",
    runs: "Latest runs",
    noRuns: "No runs yet.",
  },

  run: {
    working: "Working now",
    waiting: "Waiting for a person",
    waitingFor: (who) => `Waiting for ${who}`,
    takenOver: (who) => (who ? `${who} took it over` : "A person took it over"),
    done: "Done.",
    stopped: "Stopped.",
    gaveUp: "Nobody answered in time, so it stopped without doing what it asked.",
    failed: "Failed.",
  },

  group: {
    pageTitle: "Group",
    notFound: "This team has no such group.",
    join: "Join the group",
    leave: "Leave the group",
    people: (n) => `${n} ${n === 1 ? "person answers" : "people answer"}`,
    noPeople: "Nobody answers yet",
    fallback: "answers if nobody else does",
    inApp: "in the app",
    inAppSlack: "in the app and in Slack",
    add: "Add someone",
    adding: "Who joins the group?",
    nobodyToAdd: "Everyone on the team is already in the group.",
    remove: (name) => `Take ${name} out of the group`,
    how: "How a call works",
    step1: (handle, slack) => `The bot stops and writes @${handle}. Everyone in the group is told, in the app${slack ? " and in Slack" : ""}.`,
    step2: "The first person to answer settles it. The others see who answered and what they said.",
    step3: (minutes, owner) => `If nobody answers in ${minutes} minutes, the bot calls ${owner} directly.`,
    step4: (hours) => `With no answer in ${hours} ${hours === 1 ? "hour" : "hours"}, the bot ends without doing what it asked and leaves the notice here.`,
    theOwner: "the owner",
    bots: "Bots that call this group",
    noBots: "No bot calls this group yet.",
    week: "Calls this week",
    noCalls: "No calls this week.",
    waiting: (bot) => `${bot} is waiting`,
    escalated: (bot, owner) => `${bot} asked, nobody answered, and it went on to ${owner}.`,
    answered: (who, mins, what) => `${who} answered${mins === null ? "" : ` in ${mins} ${mins === 1 ? "minute" : "minutes"}`} and ${what}.`,
    chose: (choice) => (choice ? `chose “${choice}”` : "settled it"),
    passed: (handle) => `passed it to @${handle}`,
    tookOver: "took the run over",
    stoppedRun: "stopped the run",
  },

  wiz: {
    pageTitle: "Create a bot",
    stepOf: (n) => `Step ${n} of 4`,
    back: "Back",
    next: ["Continue", "This is right", "Continue", "Create the bot"],
    creating: "Creating…",
    loadFailed: (e) => `Could not read the team's people, groups and projects: ${e}`,
    s1Title: "What should the bot do?",
    s1About: "Write it the way you would explain it to a new colleague. Or start from one of these.",
    ideas: [
      { id: "review", title: "Review every new PR", text: "Reads each open PR nobody reviewed, checks that it has tests and points out what looks wrong.", name: "PR reviewer" },
      { id: "publish", title: "Publish to the test environment", text: "Runs the tests and publishes, stopping before it goes live.", name: "Test publisher" },
      { id: "weekly", title: "Weekly summary", text: "Gathers what the team did in each project and writes a summary.", name: "Weekly summary" },
    ],
    own: "Or describe it in your own words",
    ownPlaceholder: "E.g. whenever a PR is open, check that it has tests",
    s2Title: "Here is the plan. Is it right?",
    s2About: "These are the steps the bot is given, in this order. Nothing was worked out from your words: change any step, add or remove one.",
    s2AboutOwn: "Your description is the only step for now. Write the steps the way you want them done, one by one.",
    nameLabel: "Name",
    stepLabel: (n) => `Step ${n}`,
    stepPlaceholder: "What the bot does in this step",
    removeStep: (n) => `Remove step ${n}`,
    addStep: "Add a step",
    noSteps: "Add at least one step.",
    when: "When it runs",
    manual: "Only when someone runs it",
    notYet: "Not available yet",
    whens: {
      review: ["When a PR opens", "Every day at 9am"],
      publish: ["Every weekday at 3pm", "When a PR is merged"],
      weekly: ["Every Friday at 4pm", "Every day at 6pm"],
      own: ["On a schedule", "When something happens on GitHub"],
    },
    once: "Run once, then discard",
    onceAbout: "When its first run ends, the bot and its workflow are deleted.",
    s3Title: "Who stays nearby?",
    s3About: "The bot works alone. Choose who it calls when it needs a person: someone or a group.",
    s3Moments: [
      { key: "before", label: "Before something that can't be undone" },
      { key: "doubt", label: "When in doubt" },
      { key: "limit", label: "At a limit or on failure" },
    ],
    s4Title: "How far can it go on its own?",
    project: "Project it touches",
    noProjects: "The team has no projects yet: attach one first. Until then the bot has nothing to work on.",
    spend: "Spend per run",
    spendValue: (usd) => `up to US$ ${usd}`,
    time: "Time per run",
    timeValue: (minutes) => `up to ${minutes} minutes`,
    never: "It never deploys to production, releases or pushes to main without calling a person.",
    draftNote: (who) => `The bot starts as a draft. It runs on its own only after ${who} turns it on.`,
    doneDraft: (name) => `${name} is ready, as a draft`,
    doneOn: (name) => `${name} is on`,
    doneDraftText: (why) => `It is not running yet. ${why}`,
    doneOnText: (who) => `It runs when someone runs it, from its page. When it needs a person, it calls ${who}.`,
    mayTurnOn: (why) => `${why}, so you can turn it on now.`,
    youOwner: "You are the team's owner",
    youIn: (handle) => `You are in @${handle}`,
    youNamed: "A rule names you",
    ask: (who) => `Ask ${who} to turn it on.`,
    turnOn: "Turn the bot on",
    seePage: "See the bot's page",
    saveFailed: (e) => `Could not create the bot: ${e}`,
    turnFailed: (e) => `Could not turn the bot on: ${e}`,
  },
};

/** The hub's `TeamPolicy::default()`, which is not on the wire: the numbers a call and a new bot start from. */
const TB_POLICY = { escalateSecs: 1800, giveUpSecs: 7200, usd: 5, minutes: 30 };
/** The trigger of a bot made here. It never fires on its own: a team with no Linear key scans nothing and nobody
 * puts this label on an issue, so the bot runs only through Run now. */
const TB_MANUAL_LABEL = "pinned-run-now-only";
const TB_DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const TB_MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const TB_PAGES = ["team-bot", "team-bot-new", "team-group"];

/** The steps each starting idea fills in. Each step that calls a person says so ("call a person"). */
const TB_IDEA_STEPS = {
  review: [
    "Find the open pull requests of the project that nobody has reviewed yet.",
    "Read the changes and check that there are tests for them.",
    "Write the review on the pull request itself.",
    "Call a person if in doubt about what the change is for.",
  ],
  publish: [
    "Take what entered the project since the last publish.",
    "Run all the tests.",
    "Stop and call a person before publishing.",
    "Publish to the test environment and tell whoever made the changes the result.",
  ],
  weekly: [
    "Read what each person finished this week, by project.",
    "Write a short summary of each project.",
    "Give the summary as the result of the run.",
  ],
};

// ---- Pure helpers -------------------------------------------------------------------------------

const tbEsc = (v) => (typeof esc === "function" ? esc(v) : String(v ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`));
const tbErrorText = (e) => String(e?.message ?? e ?? TB_TEXT.noHub);
const tbCall = (op) => (typeof teamCall === "function" ? teamCall(op) : Promise.reject(TB_TEXT.unavailable));
const tbTeam = () => (typeof teamsCurrent === "function" ? teamsCurrent() : null);
const tbDocument = () => (typeof document !== "undefined" ? document : null);
const tbShown = () => (typeof shellPage === "function" ? shellPage() : null);

/** `ana.prado@acme.dev` is shown as "Ana Prado". */
function tbPerson(email) {
  const local = String(email ?? "").split("@")[0];
  return local.split(/[._-]+/).filter(Boolean).map((w) => w[0].toUpperCase() + w.slice(1)).join(" ") || "Someone";
}
const tbInitials = (name) => {
  const words = String(name ?? "").split(/[\s@._-]+/).filter(Boolean);
  return ((words.length > 1 ? words[0][0] + words[1][0] : (words[0] ?? "").slice(0, 2)) || "?").toUpperCase();
};
const TB_HUES = ["#f0a35e", "#5fd4b8", "#c7a6ff", "#6ea8ff", "#f08aa8", "#e3cf6a"];
const tbHue = (name) => TB_HUES[[...String(name)].reduce((h, c) => (h * 31 + c.codePointAt(0)) >>> 0, 7) % TB_HUES.length];
/** A project's repository as the hub keys it, github.com/owner/repo. */
function tbRepoKey(repo) {
  const url = String(repo ?? "").trim().replace(/\/+$/, "").replace(/\.git$/, "");
  if (!url) return "";
  const rest = url.includes("://") ? url.slice(url.indexOf("://") + 3) : url.replace(":", "/");
  return (rest.includes("@") ? rest.slice(rest.lastIndexOf("@") + 1) : rest).toLowerCase();
}

/** Who the viewer is among the team's members, by the e-mail they signed in with. */
function tbMe(members) {
  const email = typeof account !== "undefined" && account?.email ? String(account.email).toLowerCase() : "";
  return (members ?? []).find((m) => email && String(m.email ?? "").toLowerCase() === email) ?? null;
}
const tbMemberName = (members, id) => {
  const m = (members ?? []).find((x) => x.account === id);
  return m ? tbPerson(m.email) : "A former member";
};
const tbOwnerOf = (members) => (members ?? []).find((m) => m.role === "owner") ?? null;

const tbSameWho = (a, b) => a?.kind === b?.kind && (a?.account ?? a?.handle ?? "") === (b?.account ?? b?.handle ?? "");
const tbRules = (bot) => {
  const c = bot?.calls ?? {};
  const owner = { kind: "owner" };
  return { before_irreversible: c.before_irreversible ?? owner, in_doubt: c.in_doubt ?? owner, on_limit: c.on_limit ?? owner, on_failure: c.on_failure ?? owner };
};
const TB_SITUATIONS = ["before_irreversible", "in_doubt", "on_limit", "on_failure"];
const tbGroupHandles = (calls) => [...new Set(TB_SITUATIONS.map((k) => calls[k]).filter((w) => w?.kind === "group").map((w) => w.handle))];

/** The wording of a Who inside a sentence: a person's name, the owner's name, or the group's handle. */
function tbWhoText(who, members) {
  if (who?.kind === "group") return `@${who.handle}`;
  if (who?.kind === "person") return tbMemberName(members, who.account);
  const owner = tbOwnerOf(members);
  return owner ? `${tbPerson(owner.email)} (owner)` : "the owner";
}
/** A Who as a chip: a group is a link to its page. */
function tbWhoChip(who, members) {
  const text = tbEsc(tbWhoText(who, members));
  return who?.kind === "group" ? `<button type="button" class="tb-chip link" data-tb="open-group" data-handle="${tbEsc(who.handle)}">${text}</button>` : `<span class="tb-chip">${text}</span>`;
}

/** Whether the viewer may turn the bot on: the owner, a person a rule names, or a member of a group a rule names. */
function tbMayTurnOn(bot, { role, members, groups }) {
  if (role === "owner") return true;
  const me = tbMe(members);
  if (!me) return false;
  const calls = tbRules(bot);
  return TB_SITUATIONS.some((k) => {
    const who = calls[k];
    if (who.kind === "person") return who.account === me.account;
    if (who.kind === "group") return (groups ?? []).find((g) => g.handle === who.handle)?.members?.includes(me.account) ?? false;
    return false;
  });
}
/** Who to ask when the viewer may not: the groups the rules name, else the people they name, else the owner. */
function tbAskText(calls, members) {
  const groups = tbGroupHandles(calls);
  if (groups.length) return `someone in ${groups.map((h) => `@${h}`).join(" or ")}`;
  const people = [...new Set(TB_SITUATIONS.map((k) => calls[k]).filter((w) => w.kind === "person").map((w) => tbMemberName(members, w.account)))];
  return people.length ? `${people.join(" or ")} or the owner` : "the owner";
}

/** The steps of a workflow's prompt: one per line, the "1." numbers taken off. */
function tbStepsOf(prompt) {
  return String(prompt ?? "").split(/\r?\n/).map((line) => line.replace(/^\s*(?:\d+[.)]|[-*])\s*/, "").trim()).filter(Boolean);
}
/** The text of a workflow's session steps: the session nodes' prompts in the graph, else its instructions. */
function tbWorkflowPrompt(workflow) {
  const nodes = (workflow?.graph?.nodes ?? []).filter((n) => n.type === "session" && String(n.prompt ?? "").trim());
  return nodes.length ? nodes.map((n) => n.prompt).join("\n") : String(workflow?.instructions ?? "");
}
const tbWorkflowRepo = (workflow) => (workflow?.graph?.nodes ?? []).find((n) => n.type === "session" && n.repo)?.repo || workflow?.repo || "";
/** Whether a step is one where the bot calls a person, and which moment's rule says who. */
function tbCallStep(text) {
  if (!/\b(?:call|calls|ask|asks|page|pages|wait|waits|stop|stops)\b[^.]{0,40}?(?:\b(?:person|people|someone|group|owner)\b|@\w+)/i.test(text)) return null;
  if (/\b(?:doubt|unsure|not sure)\b/i.test(text)) return "in_doubt";
  if (/\b(?:limit|fail|failure|fails|failed)\b/i.test(text)) return "on_limit";
  return "before_irreversible";
}

/** The numbered steps a bot's workflow is saved with. */
const tbPromptOf = (steps) => steps.map((s, i) => `${i + 1}. ${s.trim()}`).join("\n");

/** The connector the hub stores for a bot made here: one Session step, and a trigger that fires only through Run now. */
function tbConnector({ id = "", name, steps, repo }) {
  return {
    id,
    name,
    enabled: false,
    trigger: { kind: "linear-label", label: TB_MANUAL_LABEL },
    folder: "",
    repo,
    model: "opus",
    effort: "high",
    permissionMode: "auto",
    worktree: false,
    proof: false,
    instructions: "",
    graph: {
      nodes: [
        { id: "trigger-1", name: "Trigger", position: [0, 0], type: "trigger", watch: { kind: "linear-label", label: TB_MANUAL_LABEL } },
        { id: "session-1", name: "Run session", position: [230, 0], type: "session", prompt: tbPromptOf(steps), folder: "", repo, model: "", effort: "", permission: "", account: "", worktree: false, instructions: "" },
      ],
      edges: [{ from: "trigger-1", port: "out", to: "session-1" }],
    },
  };
}
const tbSaveConnectorOp = (connector) => ({ op: "hub", call: { cmd: "save_connector", connector } });

/** The `Bot` a wizard saves: draft, owner left to the hub. */
const tbBotOf = ({ workflow, name, who, once }) => ({
  workflow,
  name,
  owner: null,
  calls: { before_irreversible: who.before, in_doubt: who.doubt, on_limit: who.limit, on_failure: who.limit },
  limits: { usd: TB_POLICY.usd, minutes: TB_POLICY.minutes },
  on: false,
  on_by: null,
  once,
});

/** "today", "yesterday", the weekday for the last week, then the date. */
function tbWhen(unix, now = Date.now()) {
  const day = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const at = new Date(unix * 1000);
  const ago = Math.round((day(new Date(now)) - day(at)) / 86400000);
  if (ago <= 0) return "today";
  if (ago === 1) return "yesterday";
  if (ago < 7) return TB_DAYS[at.getDay()];
  return `${TB_MONTHS[at.getMonth()]} ${at.getDate()}`;
}
const tbThisWeek = (unix, now = Date.now()) => unix * 1000 > now - 7 * 86400000;

/** What a run came to, in words, and whether it needs a person. `open` is the call of the run still waiting, if any. */
function tbRunLine(run, { members, open } = {}) {
  const t = TB_TEXT.run;
  const withOutcome = (head) => [head, String(run.outcome ?? "").trim()].filter(Boolean).join(" ");
  switch (run.state) {
    case "working":
      return { text: t.working, tone: "live" };
    case "waiting": {
      const asked = (open?.asked ?? []).map((w) => tbWhoText(w, members));
      return { text: asked.length ? t.waitingFor(asked.join(" or ")) : t.waiting, tone: "wait" };
    }
    case "taken_over":
      return { text: t.takenOver(run.controller ? tbMemberName(members, run.controller) : ""), tone: "live" };
    case "done":
      return { text: String(run.outcome ?? "").trim() || t.done, tone: "" };
    case "stopped":
      return { text: withOutcome(t.stopped), tone: "" };
    case "gave_up":
      return { text: t.gaveUp, tone: "bad" };
    case "failed":
      return { text: withOutcome(t.failed), tone: "bad" };
    default:
      return { text: String(run.outcome ?? "").trim() || String(run.state ?? ""), tone: "" };
  }
}
const tbRunning = (run) => run?.state === "working" || run?.state === "taken_over" || run?.state === "waiting";

/** What a call came to for the group page: when it was asked, the line, and its tone. */
function tbCallLine(call, { members, bots, me }) {
  const t = TB_TEXT.group;
  const bot = (bots ?? []).find((b) => b.workflow === call.workflow)?.name || "A bot";
  const a = call.answer;
  if (!a) {
    if (call.escalated) return { text: t.escalated(bot, tbWhoText({ kind: "owner" }, members)), tone: "wait" };
    return { text: t.waiting(bot), tone: "wait" };
  }
  const who = me && a.by === me.account ? TB_TEXT.youCap : tbMemberName(members, a.by);
  const kind = a.kind?.kind;
  const what = kind === "pass_to" ? t.passed(a.kind.handle) : kind === "take_over" ? t.tookOver : kind === "stop" ? t.stoppedRun : t.chose(a.choice);
  const mins = a.at && call.created_at ? Math.max(0, Math.round((a.at - call.created_at) / 60)) : null;
  return { text: t.answered(who, mins, what), tone: "" };
}
const tbCallsGroup = (call, handle) => (call.asked ?? []).some((w) => w.kind === "group" && w.handle === handle);

// ---- Page plumbing ------------------------------------------------------------------------------

const tbButton = (attrs, label, kind = "secondary", disabled = false) => `<button type="button" class="tb-btn ${kind}" ${attrs}${disabled ? " disabled" : ""}>${tbEsc(label)}</button>`;
const tbError = (text) => (text ? `<p class="tb-error" role="alert">${tbEsc(text)}</p>` : "");
const tbAvatar = (name, size = "") => `<span class="tb-avatar ${size}" aria-hidden="true" style="background:${tbHue(name)}">${tbEsc(tbInitials(name))}</span>`;
const tbMono = (name, size = "") => `<span class="tb-mono ${size}" aria-hidden="true">${tbEsc(tbInitials(name))}</span>`;

function tbDraw(title, body, wide = true) {
  if (typeof setPage !== "function") return;
  setPage(`${headHtml(tbEsc(title))}<div class="page-body"><div class="tb-page${wide ? "" : " narrow"}">${body}</div></div>`);
}
const tbAsk = (title, text, extra = "") => tbDraw(title, `<div class="tb-empty"><h2>${tbEsc(title)}</h2><p>${tbEsc(text)}</p>${extra}</div>`);

/** The team's home is its Calls page. */
function tbOpenHome() {
  openPage("team-calls");
}

const tbBotUi = { id: "", state: "idle", error: "", data: null, busy: "", note: "", started: null };
const tbGroupUi = { handle: "", state: "idle", error: "", data: null, busy: "", note: "", adding: false };

function openTeamBot(workflow) {
  tbBotUi.id = String(workflow ?? "");
  if (typeof openPage === "function") openPage("team-bot");
}
function openTeamGroup(handle) {
  tbGroupUi.handle = String(handle ?? "").replace(/^@/, "");
  if (typeof openPage === "function") openPage("team-group");
}
function openTeamBotNew() {
  if (typeof openPage === "function") openPage("team-bot-new");
}
const tbRunLink = (run, label, cls = "") => `<button type="button" class="${cls}" data-tb="open-run" data-run="${tbEsc(run)}">${label}</button>`;

// ---- The bot page -------------------------------------------------------------------------------

async function tbLoadBot(quiet = false) {
  const ui = tbBotUi;
  const team = tbTeam();
  if (!team) return drawBotPage();
  if (!quiet) {
    ui.state = "loading";
    drawBotPage();
  }
  const id = ui.id;
  try {
    const [bots, members, groups, projects, runs, calls, settings] = await Promise.all([
      tbCall({ op: "bots" }),
      tbCall({ op: "members" }),
      tbCall({ op: "groups" }),
      tbCall({ op: "projects" }),
      tbCall({ op: "runs" }),
      tbCall({ op: "calls" }).catch(() => []),
      tbCall({ op: "hub", call: { cmd: "get_settings" } }),
    ]);
    if (ui.id !== id) return;
    const list = (v) => (Array.isArray(v) ? v : []);
    ui.data = {
      bot: list(bots).find((b) => b.workflow === id) ?? null,
      members: list(members),
      groups: list(groups),
      projects: list(projects),
      runs: list(runs).filter((r) => r.workflow === id).sort((a, b) => b.started_at - a.started_at),
      calls: list(calls),
      workflow: list(settings?.connectors).find((c) => c.id === id) ?? null,
    };
    ui.state = "ready";
    if (!quiet) ui.error = "";
  } catch (e) {
    if (ui.id !== id) return;
    ui.state = "error";
    ui.error = tbErrorText(e);
  }
  drawBotPage();
}

function tbBotHtml(ui, team) {
  const t = TB_TEXT.bot;
  if (ui.state === "error") return `${tbError(TB_TEXT.loadFailed(ui.error))}<div class="tb-actions">${tbButton('data-tb="reload"', TB_TEXT.retry)}</div>`;
  if (!ui.data) return `<p class="tb-hint" role="status">${TB_TEXT.loading}</p>`;
  const { bot, members, groups, projects, runs, calls, workflow } = ui.data;
  if (!bot) return `<div class="tb-empty"><h2>${tbEsc(t.notFound)}</h2></div>`;
  const rules = tbRules(bot);
  const owner = bot.owner ? tbMemberName(members, bot.owner) : tbOwnerOf(members) ? tbPerson(tbOwnerOf(members).email) : "";
  const key = tbRepoKey(tbWorkflowRepo(workflow));
  const project = projects.find((p) => tbRepoKey(p.repo) === key)?.name || (key ? key.split("/").pop() : "");
  const name = bot.name || workflow?.name || "Bot";

  const latest = runs[0];
  // A run just started may not be in the list yet.
  const running = tbRunning(latest) ? latest : ui.started && !runs.some((r) => r.id === ui.started.id) ? ui.started : null;
  const may = tbMayTurnOn(bot, { role: team.role, members, groups });
  const needKeys = team.credential && !(team.credential.upstash && team.credential.claude);

  const runBtn = running
    ? tbRunLink(running.id, tbEsc(t.follow), "tb-btn follow")
    : ui.busy === "run"
      ? tbButton("disabled", t.starting, "secondary", true)
      : `<button type="button" class="tb-btn primary" data-tb="run-now"${bot.on ? "" : ` disabled title="${tbEsc(t.runNowDraft)}"`}>${tbEsc(t.runNow)}</button>`;
  const toggle = bot.on
    ? tbButton(`data-tb="toggle" aria-pressed="true"`, ui.busy === "toggle" ? t.switching : t.turnOff, "secondary", ui.busy === "toggle")
    : tbButton(`data-tb="toggle" aria-pressed="false"`, ui.busy === "toggle" ? t.switching : t.turnOn, "secondary", ui.busy === "toggle" || !may);

  const steps = tbStepsOf(tbWorkflowPrompt(workflow));
  const manual = workflow?.trigger?.kind === "linear-label" && workflow.trigger.label === TB_MANUAL_LABEL;
  const stepRows = steps.map((s, i) => {
    const moment = tbCallStep(s);
    const num = `<span class="tb-n">${i + 1}</span>`;
    if (!moment) return `<li class="tb-step">${num}<span class="tb-step-text">${tbEsc(s)}</span></li>`;
    return `<li class="tb-step call">${num}<span class="tb-step-text">${tbEsc(s)} ${tbWhoChip(rules[moment], members)}</span></li>`;
  });

  const moments = [
    ["before_irreversible", t.moments.before_irreversible, rules.before_irreversible],
    ["in_doubt", t.moments.in_doubt, rules.in_doubt],
  ];
  if (tbSameWho(rules.on_limit, rules.on_failure)) moments.push(["on_limit", `${t.moments.limit_or_failure} (${t.limits(bot.limits?.usd ?? TB_POLICY.usd, bot.limits?.minutes ?? TB_POLICY.minutes)})`, rules.on_limit]);
  else {
    moments.push(["on_limit", `${t.moments.on_limit} (${t.limits(bot.limits?.usd ?? TB_POLICY.usd, bot.limits?.minutes ?? TB_POLICY.minutes)})`, rules.on_limit]);
    moments.push(["on_failure", t.moments.on_failure, rules.on_failure]);
  }
  const moment = ([k, label, who]) => `<div class="tb-row" data-moment="${k}"><span>${tbEsc(label)}</span>${tbWhoChip(who, members)}</div>`;

  const openCalls = calls.filter((c) => !c.answer);
  const runRows = runs.slice(0, 8).map((r) => {
    const line = tbRunLine(r, { members, open: openCalls.find((c) => c.run === r.id) });
    const row = `<span class="tb-when">${tbEsc(tbWhen(r.started_at))}</span><span class="tb-line" data-tone="${line.tone}">${tbEsc(line.text)}</span>`;
    return `<button type="button" class="tb-listrow" data-tb="open-run" data-run="${tbEsc(r.id)}">${row}</button>`;
  });

  const notice = bot.on ? "" : may ? "" : `<p class="tb-note">${tbEsc(t.cannotTurnOn(tbAskText(rules, members)))}</p>`;
  return `<div class="tb-head">
      <div class="tb-who">${tbMono(name, "big")}<div class="tb-titles"><h1>${tbEsc(name)}${bot.on ? "" : ` <span class="tb-tag draft">${tbEsc(t.draft)}</span>`}</h1>
        <div class="tb-sub">${tbEsc(t.of(project, owner))}${bot.once ? ` <span class="tb-tag">${tbEsc(t.once)}</span>` : ""}</div></div></div>
      <div class="tb-actions">${runBtn}${toggle}</div>
    </div>
    ${tbError(ui.error && ui.state === "ready" ? ui.error : "")}${notice}${needKeys && !bot.on ? `<p class="tb-note">${tbEsc(t.needKeys)}</p>` : ""}
    <section class="tb-section"><h2>${tbEsc(manual ? t.doesManual : t.does)}</h2>${stepRows.length ? `<ol class="tb-steps">${stepRows.join("")}</ol>` : `<p class="tb-hint">${tbEsc(t.noSteps)}</p>`}</section>
    <section class="tb-section"><h2>${tbEsc(t.calls)}</h2><div class="tb-card">${moments.map(moment).join("")}</div><p class="tb-foot">${tbEsc(t.callsNote)}</p></section>
    <section class="tb-section"><h2>${tbEsc(t.runs)}</h2>${runRows.length ? `<div class="tb-list">${runRows.join("")}</div>` : `<p class="tb-hint">${tbEsc(t.noRuns)}</p>`}</section>`;
}

function drawBotPage() {
  if (tbShown() !== "team-bot") return;
  const team = tbTeam();
  if (!team) return tbAsk(TB_TEXT.noTeamTitle, TB_TEXT.noTeam);
  tbDraw(TB_TEXT.bot.pageTitle, tbBotHtml(tbBotUi, team));
}

const TB_BOT_ACTS = {
  reload: () => tbLoadBot(),
  "open-group": (b) => openTeamGroup(b.dataset.handle),
  "open-run": (b) => openTeamRun(b.dataset.run),
  async "run-now"() {
    const ui = tbBotUi;
    ui.busy = "run";
    ui.error = "";
    drawBotPage();
    try {
      ui.started = await tbCall({ op: "run_now", workflow: ui.id });
    } catch (e) {
      ui.error = tbErrorText(e);
    }
    ui.busy = "";
    await tbLoadBot(true);
  },
  async toggle() {
    const ui = tbBotUi;
    const on = !ui.data?.bot?.on;
    ui.busy = "toggle";
    ui.error = "";
    drawBotPage();
    try {
      await tbCall({ op: "turn_bot_on", workflow: ui.id, on });
      if (!on) ui.started = null;
    } catch (e) {
      ui.error = tbErrorText(e);
    }
    ui.busy = "";
    await tbLoadBot(true);
  },
};

function showBot() {
  Object.assign(tbBotUi, { state: "idle", error: "", data: null, busy: "", started: null });
  if (!tbTeam()) return drawBotPage();
  tbLoadBot();
}

// ---- The group page -----------------------------------------------------------------------------

async function tbLoadGroup(quiet = false) {
  const ui = tbGroupUi;
  if (!tbTeam()) return drawGroupPage();
  if (!quiet) {
    ui.state = "loading";
    drawGroupPage();
  }
  const handle = ui.handle;
  try {
    const [groups, members, bots, calls] = await Promise.all([tbCall({ op: "groups" }), tbCall({ op: "members" }), tbCall({ op: "bots" }), tbCall({ op: "calls" })]);
    if (ui.handle !== handle) return;
    const list = (v) => (Array.isArray(v) ? v : []);
    ui.data = { group: list(groups).find((g) => g.handle === handle) ?? null, members: list(members), bots: list(bots), calls: list(calls) };
    ui.state = "ready";
    if (!quiet) ui.error = "";
  } catch (e) {
    if (ui.handle !== handle) return;
    ui.state = "error";
    ui.error = tbErrorText(e);
  }
  drawGroupPage();
}

function tbGroupHtml(ui, team) {
  const t = TB_TEXT.group;
  if (ui.state === "error") return `${tbError(TB_TEXT.loadFailed(ui.error))}<div class="tb-actions">${tbButton('data-tb="reload"', TB_TEXT.retry)}</div>`;
  if (!ui.data) return `<p class="tb-hint" role="status">${TB_TEXT.loading}</p>`;
  const { group, members, bots, calls } = ui.data;
  if (!group) return `<div class="tb-empty"><h2>${tbEsc(t.notFound)}</h2></div>`;
  const me = tbMe(members);
  const isOwner = team.role === "owner";
  const slack = !!team.credential?.slack;
  const owner = tbOwnerOf(members);
  const inGroup = (id) => group.members.includes(id);
  const member = !!me && inGroup(me.account);
  const answering = members.filter((m) => inGroup(m.account));
  const fallbackId = group.fallback ?? owner?.account ?? null;
  const fallbackMember = fallbackId ? members.find((m) => m.account === fallbackId) : null;
  const shown = fallbackMember && !inGroup(fallbackMember.account) ? [...answering, fallbackMember] : answering;

  const person = (m) => {
    const name = tbPerson(m.email);
    const mine = me?.account === m.account;
    const sub = m.account === fallbackId ? t.fallback : mine ? (slack ? t.inAppSlack : t.inApp) : "";
    const removable = isOwner && inGroup(m.account) ? `<button type="button" class="tb-x" data-tb="remove-member" data-account="${tbEsc(m.account)}" aria-label="${tbEsc(t.remove(name))}" title="${tbEsc(t.remove(name))}">×</button>` : "";
    return `<div class="tb-face" data-account="${tbEsc(m.account)}">${tbAvatar(name, "big")}<span class="tb-face-text"><span class="tb-face-name">${tbEsc(mine ? TB_TEXT.youCap : name)}</span>${sub ? `<span class="tb-face-sub">${tbEsc(sub)}</span>` : ""}</span>${removable}</div>`;
  };
  const addable = members.filter((m) => !inGroup(m.account));
  const picker = ui.adding
    ? `<div class="tb-pick" role="group" aria-label="${tbEsc(t.adding)}"><span class="tb-hint">${tbEsc(t.adding)}</span>${addable.length ? addable.map((m) => `<button type="button" class="tb-opt" data-tb="add-member" data-account="${tbEsc(m.account)}">${tbEsc(tbPerson(m.email))}</button>`).join("") : `<span class="tb-hint">${tbEsc(t.nobodyToAdd)}</span>`}</div>`
    : "";
  const add = isOwner ? `<button type="button" class="tb-face add" data-tb="show-add" aria-expanded="${ui.adding}">${tbEsc(t.add)}</button>` : "";

  const theOwner = owner ? tbPerson(owner.email) : t.theOwner;
  const how = [
    t.step1(group.handle, slack),
    t.step2,
    t.step3(Math.round(TB_POLICY.escalateSecs / 60), theOwner),
    t.step4(Math.round(TB_POLICY.giveUpSecs / 3600)),
  ];
  const momentWords = { before_irreversible: "before something that can't be undone", in_doubt: "when in doubt", on_limit: "at a limit", on_failure: "on failure" };
  const calling = bots
    .map((b) => ({ bot: b, moments: TB_SITUATIONS.filter((k) => tbRules(b)[k].kind === "group" && tbRules(b)[k].handle === group.handle) }))
    .filter((x) => x.moments.length);
  const botRows = calling.map(({ bot, moments }) => {
    const name = bot.name || "Bot";
    return `<button type="button" class="tb-listrow bot" data-tb="open-bot" data-workflow="${tbEsc(bot.workflow)}">${tbMono(name)}<span class="tb-line">${tbEsc(`${name}, ${moments.map((k) => momentWords[k]).join(" and ")}`)}</span></button>`;
  });
  const week = calls.filter((c) => tbCallsGroup(c, group.handle) && tbThisWeek(c.created_at)).sort((a, b) => b.created_at - a.created_at);
  const callRows = week.map((c) => {
    const line = tbCallLine(c, { members, bots, me });
    const row = `<span class="tb-when">${tbEsc(tbWhen(c.created_at))}</span><span class="tb-line" data-tone="${line.tone}">${tbEsc(line.text)}</span>`;
    return !c.answer && c.run ? `<button type="button" class="tb-listrow" data-tb="open-run" data-run="${tbEsc(c.run)}">${row}</button>` : `<div class="tb-listrow">${row}</div>`;
  });

  return `<div class="tb-head">
      <div class="tb-titles"><h1>@${tbEsc(group.handle)}</h1>${group.about ? `<p class="tb-about">${tbEsc(group.about)}</p>` : ""}</div>
      <div class="tb-actions">${tbButton(`data-tb="membership" aria-pressed="${member}"`, member ? t.leave : t.join, "secondary", ui.busy === "membership" || !me)}</div>
    </div>
    ${tbError(ui.error && ui.state === "ready" ? ui.error : "")}
    <section class="tb-section"><h2>${tbEsc(answering.length ? t.people(answering.length) : t.noPeople)}</h2>
      <div class="tb-faces">${shown.map(person).join("")}${add}</div>${picker}</section>
    <section class="tb-section"><h2>${tbEsc(t.how)}</h2><ol class="tb-steps">${how.map((s, i) => `<li class="tb-step"><span class="tb-n">${i + 1}</span><span class="tb-step-text">${tbEsc(s)}</span></li>`).join("")}</ol></section>
    <section class="tb-section"><h2>${tbEsc(t.bots)}</h2>${botRows.length ? `<div class="tb-list">${botRows.join("")}</div>` : `<p class="tb-hint">${tbEsc(t.noBots)}</p>`}</section>
    <section class="tb-section"><h2>${tbEsc(t.week)}</h2>${callRows.length ? `<div class="tb-list">${callRows.join("")}</div>` : `<p class="tb-hint">${tbEsc(t.noCalls)}</p>`}</section>`;
}

function drawGroupPage() {
  if (tbShown() !== "team-group") return;
  const team = tbTeam();
  if (!team) return tbAsk(TB_TEXT.noTeamTitle, TB_TEXT.noTeam);
  tbDraw(`@${tbGroupUi.handle}`, tbGroupHtml(tbGroupUi, team));
}

/** Saves the group with `members` as its people (the owner's way to set them). */
async function tbSetMembers(members) {
  const ui = tbGroupUi;
  const group = ui.data?.group;
  if (!group) return;
  ui.busy = "members";
  ui.error = "";
  try {
    await tbCall({ op: "save_group", group: { ...group, members } });
  } catch (e) {
    ui.error = tbErrorText(e);
  }
  ui.busy = "";
  await tbLoadGroup(true);
}

const TB_GROUP_ACTS = {
  reload: () => tbLoadGroup(),
  "open-bot": (b) => openTeamBot(b.dataset.workflow),
  "open-run": (b) => openTeamRun(b.dataset.run),
  "show-add"() {
    tbGroupUi.adding = !tbGroupUi.adding;
    drawGroupPage();
  },
  "add-member"(b) {
    const group = tbGroupUi.data?.group;
    if (group && !group.members.includes(b.dataset.account)) return tbSetMembers([...group.members, b.dataset.account]);
  },
  "remove-member"(b) {
    const group = tbGroupUi.data?.group;
    if (group) return tbSetMembers(group.members.filter((m) => m !== b.dataset.account));
  },
  async membership() {
    const ui = tbGroupUi;
    const me = tbMe(ui.data?.members);
    const group = ui.data?.group;
    if (!me || !group) return;
    const member = group.members.includes(me.account);
    ui.busy = "membership";
    ui.error = "";
    drawGroupPage();
    try {
      await tbCall({ op: member ? "leave_group" : "join_group", handle: group.handle });
    } catch (e) {
      ui.error = tbErrorText(e);
    }
    ui.busy = "";
    await tbLoadGroup(true);
  },
};

function showGroup() {
  Object.assign(tbGroupUi, { state: "idle", error: "", data: null, busy: "", adding: false });
  if (!tbTeam()) return drawGroupPage();
  tbLoadGroup();
}

// ---- Create a bot -------------------------------------------------------------------------------

const tbWiz = {};
function tbWizReset() {
  Object.assign(tbWiz, {
    step: 1,
    idea: null,
    desc: "",
    name: "",
    steps: [],
    once: false,
    who: { before: { kind: "owner" }, doubt: { kind: "owner" }, limit: { kind: "owner" } },
    project: "",
    data: null,
    loadError: "",
    busy: false,
    error: "",
    workflow: "",
    saved: null,
    on: false,
    focus: null,
  });
}
tbWizReset();

const tbIdea = () => TB_TEXT.wiz.ideas.find((i) => i.id === tbWiz.idea) ?? null;
/** A name for a bot made from the person's own words: the first few of them. */
function tbNameFrom(text) {
  const words = String(text).trim().split(/\s+/).filter(Boolean);
  let name = "";
  for (const w of words) {
    if ((name + " " + w).trim().length > 32) break;
    name = (name + " " + w).trim();
  }
  return name || words[0]?.slice(0, 32) || "New bot";
}
const tbWizReady = () => {
  const s = tbWiz;
  if (s.step === 1) return !!s.idea || !!s.desc.trim();
  if (s.step === 2) return s.steps.some((x) => x.trim()) && !!s.name.trim();
  if (s.step === 4) return !s.data?.projects.length || !!s.project;
  return true;
};

/** The people and groups that may be picked for a moment: the owner, each group, each other member. */
function tbOptions(data) {
  const owner = tbOwnerOf(data.members);
  return [
    { who: { kind: "owner" }, label: owner ? `Owner (${tbPerson(owner.email)})` : "Owner" },
    ...data.groups.map((g) => ({ who: { kind: "group", handle: g.handle }, label: `@${g.handle}` })),
    ...data.members.filter((m) => m.role !== "owner").map((m) => ({ who: { kind: "person", account: m.account }, label: tbPerson(m.email) })),
  ];
}

function tbWizTurnOn(calls, data, role) {
  const w = TB_TEXT.wiz;
  const rules = { ...calls };
  const may = tbMayTurnOn({ calls: rules }, { role, members: data.members, groups: data.groups });
  const me = tbMe(data.members);
  const group = tbGroupHandles(rules).find((h) => data.groups.find((g) => g.handle === h)?.members?.includes(me?.account));
  const why = role === "owner" ? w.youOwner : group ? w.youIn(group) : w.youNamed;
  return { may, why, ask: tbAskText(rules, data.members) };
}

function tbWizHtml(team) {
  const w = TB_TEXT.wiz;
  const s = tbWiz;
  const back = `<button type="button" class="tb-back" data-tb="back-team">${tbEsc(TB_TEXT.backToTeam)}</button>`;
  if (s.loadError) return `${back}${tbError(w.loadFailed(s.loadError))}<div class="tb-actions">${tbButton('data-tb="reload"', TB_TEXT.retry)}</div>`;
  if (!s.data) return `${back}<p class="tb-hint" role="status">${TB_TEXT.loading}</p>`;
  const kicker = (n) => `<p class="tb-kicker">${tbEsc(w.stepOf(n))}</p>`;
  const calls = { before_irreversible: s.who.before, in_doubt: s.who.doubt, on_limit: s.who.limit, on_failure: s.who.limit };
  let body = "";
  if (s.step === 1) {
    body = `${kicker(1)}<h1>${tbEsc(w.s1Title)}</h1><p class="tb-lede">${tbEsc(w.s1About)}</p>
      <div class="tb-choices">${w.ideas.map((i) => `<button type="button" class="tb-choice" data-tb="idea" data-idea="${i.id}" aria-pressed="${s.idea === i.id}"><span class="tb-choice-title">${tbEsc(i.title)}</span><span class="tb-choice-text">${tbEsc(i.text)}</span></button>`).join("")}</div>
      <div class="tb-field"><label for="tb-desc">${tbEsc(w.own)}</label><input id="tb-desc" type="text" data-tb-input="desc" value="${tbEsc(s.desc)}" placeholder="${tbEsc(w.ownPlaceholder)}" autocomplete="off"></div>`;
  } else if (s.step === 2) {
    const own = !s.idea;
    const whens = w.whens[s.idea ?? "own"];
    body = `${kicker(2)}<h1>${tbEsc(w.s2Title)}</h1><p class="tb-lede">${tbEsc(own ? w.s2AboutOwn : w.s2About)}</p>
      <div class="tb-field"><label for="tb-name">${tbEsc(w.nameLabel)}</label><input id="tb-name" type="text" data-tb-input="name" value="${tbEsc(s.name)}" autocomplete="off"></div>
      <ol class="tb-steps editable">${s.steps.map((text, i) => `<li class="tb-step edit"><span class="tb-n">${i + 1}</span><input type="text" class="tb-step-input" data-tb-input="step" data-i="${i}" value="${tbEsc(text)}" aria-label="${tbEsc(w.stepLabel(i + 1))}" placeholder="${tbEsc(w.stepPlaceholder)}" autocomplete="off"><button type="button" class="tb-x" data-tb="remove-step" data-i="${i}" aria-label="${tbEsc(w.removeStep(i + 1))}" title="${tbEsc(w.removeStep(i + 1))}">×</button></li>`).join("")}</ol>
      ${s.steps.some((x) => x.trim()) ? "" : `<p class="tb-hint">${tbEsc(w.noSteps)}</p>`}
      <div class="tb-actions">${tbButton('data-tb="add-step"', w.addStep, "quiet")}</div>
      <div class="tb-field"><div class="tb-label">${tbEsc(w.when)}</div><div class="tb-pills">
        <button type="button" class="tb-pill" aria-pressed="true">${tbEsc(w.manual)}</button>
        ${whens.map((x) => `<button type="button" class="tb-pill" disabled aria-pressed="false">${tbEsc(x)} <span class="tb-soon">${tbEsc(w.notYet)}</span></button>`).join("")}</div></div>
      <button type="button" class="tb-check" role="checkbox" aria-checked="${s.once}" data-tb="once"><span class="tb-box" aria-hidden="true"></span><span class="tb-texts"><span class="tb-name">${tbEsc(w.once)}</span><span class="tb-about">${tbEsc(w.onceAbout)}</span></span></button>`;
  } else if (s.step === 3) {
    const options = tbOptions(s.data);
    body = `${kicker(3)}<h1>${tbEsc(w.s3Title)}</h1><p class="tb-lede">${tbEsc(w.s3About)}</p>
      <div class="tb-moments">${w.s3Moments.map((m) => `<div class="tb-moment" data-moment="${m.key}"><div class="tb-moment-label">${tbEsc(m.label)}</div><div class="tb-pills">${options.map((o) => `<button type="button" class="tb-opt" data-tb="who" data-moment="${m.key}" data-kind="${o.who.kind}" data-id="${tbEsc(o.who.account ?? o.who.handle ?? "")}" aria-pressed="${tbSameWho(s.who[m.key], o.who)}">${tbEsc(o.label)}</button>`).join("")}</div></div>`).join("")}</div>`;
  } else if (s.step === 4) {
    const turn = tbWizTurnOn(calls, s.data, team.role);
    const groups = tbGroupHandles(calls);
    const who = groups.length ? `someone in ${groups.map((h) => `@${h}`).join(" or ")}` : turn.ask;
    body = `${kicker(4)}<h1>${tbEsc(w.s4Title)}</h1>
      <div class="tb-card"><div class="tb-row stack"><span>${tbEsc(w.project)}</span>${
        s.data.projects.length
          ? `<span class="tb-pills">${s.data.projects.map((p) => `<button type="button" class="tb-opt" data-tb="project" data-key="${tbEsc(p.key)}" aria-pressed="${s.project === p.key}">${tbEsc(p.name)}</button>`).join("")}</span>`
          : `<span class="tb-value">${tbEsc(w.noProjects)}</span>`
      }</div><div class="tb-row"><span>${tbEsc(w.spend)}</span><span class="tb-value">${tbEsc(w.spendValue(TB_POLICY.usd))}</span></div><div class="tb-row"><span>${tbEsc(w.time)}</span><span class="tb-value">${tbEsc(w.timeValue(TB_POLICY.minutes))}</span></div></div>
      <p class="tb-lede">${tbEsc(w.never)}</p><p class="tb-lede">${tbEsc(w.draftNote(`${who}${groups.length ? " (or the owner)" : ""}`))}</p>`;
  } else {
    const name = s.saved?.name ?? s.name;
    const turn = tbWizTurnOn(s.saved?.calls ?? calls, s.data, team.role);
    const title = s.on ? w.doneOn(name) : w.doneDraft(name);
    const text = s.on ? w.doneOnText(tbWhoText(s.who.before, s.data.members)) : w.doneDraftText(turn.may ? w.mayTurnOn(turn.why) : w.ask(turn.ask));
    body = `${tbMono(name, "huge")}<h1>${tbEsc(title)}</h1><p class="tb-lede big">${tbEsc(text)}</p>${tbError(s.error)}
      <div class="tb-actions">${!s.on && turn.may ? tbButton('data-tb="turn-on"', s.busy ? TB_TEXT.bot.switching : w.turnOn, "primary", s.busy) : ""}<button type="button" class="tb-link" data-tb="see-bot">${tbEsc(w.seePage)}</button></div>`;
  }
  const nav = s.step <= 4
    ? `${tbError(s.error)}<div class="tb-actions">${tbButton('data-tb="next"', s.busy ? w.creating : w.next[s.step - 1], "primary", !tbWizReady() || s.busy)}${s.step > 1 ? tbButton('data-tb="prev"', w.back, "quiet", s.busy) : ""}</div>`
    : "";
  return `${back}<div class="tb-wiz">${body}${nav}</div>`;
}

function tbWizDraw() {
  if (tbShown() !== "team-bot-new") return;
  const team = tbTeam();
  if (!team) return tbAsk(TB_TEXT.noTeamTitle, TB_TEXT.noTeam);
  tbDraw(TB_TEXT.wiz.pageTitle, tbWizHtml(team), false);
  const target = tbWiz.focus;
  tbWiz.focus = null;
  if (target) pageView?.querySelector?.(target)?.focus?.();
}

async function tbWizLoad() {
  tbWiz.loadError = "";
  tbWiz.data = null;
  tbWizDraw();
  try {
    const [members, groups, projects] = await Promise.all([tbCall({ op: "members" }), tbCall({ op: "groups" }), tbCall({ op: "projects" })]);
    const list = (v) => (Array.isArray(v) ? v : []);
    tbWiz.data = { members: list(members), groups: list(groups), projects: list(projects) };
    if (!tbWiz.project && tbWiz.data.projects.length === 1) tbWiz.project = tbWiz.data.projects[0].key;
  } catch (e) {
    tbWiz.loadError = tbErrorText(e);
  }
  tbWizDraw();
}

/** Saves the workflow in the team's store, then the bot that goes with it. The workflow's id is kept, so a retry updates it. */
async function tbWizCreate() {
  const s = tbWiz;
  const steps = s.steps.map((x) => x.trim()).filter(Boolean);
  const project = s.data.projects.find((p) => p.key === s.project);
  s.busy = true;
  s.error = "";
  tbWizDraw();
  try {
    if (!s.workflow) {
      const saved = await tbCall(tbSaveConnectorOp(tbConnector({ name: s.name.trim(), steps, repo: project?.repo ?? "" })));
      s.workflow = String(saved?.id ?? "");
      if (!s.workflow) throw "the hub did not name the workflow";
    } else {
      await tbCall(tbSaveConnectorOp(tbConnector({ id: s.workflow, name: s.name.trim(), steps, repo: project?.repo ?? "" })));
    }
    const bot = tbBotOf({ workflow: s.workflow, name: s.name.trim(), who: s.who, once: s.once });
    await tbCall({ op: "save_bot", bot });
    s.saved = bot;
    s.step = 5;
    s.on = false;
  } catch (e) {
    s.error = TB_TEXT.wiz.saveFailed(tbErrorText(e));
  }
  s.busy = false;
  tbWizDraw();
}

const TB_WIZ_ACTS = {
  reload: () => tbWizLoad(),
  "back-team": () => tbOpenHome(),
  idea(b) {
    const idea = TB_TEXT.wiz.ideas.find((i) => i.id === b.dataset.idea);
    if (!idea) return;
    Object.assign(tbWiz, { idea: idea.id, desc: "", name: idea.name, steps: [...TB_IDEA_STEPS[idea.id]] });
    tbWizDraw();
  },
  prev() {
    tbWiz.step = Math.max(1, tbWiz.step - 1);
    tbWiz.error = "";
    tbWizDraw();
  },
  next() {
    const s = tbWiz;
    if (!tbWizReady()) return;
    if (s.step === 1 && !s.idea) {
      // Nothing turns a description into steps in this build: the text itself is the one step.
      if (s.steps.length !== 1 || s.steps[0] !== s.desc.trim()) Object.assign(s, { steps: [s.desc.trim()], name: tbNameFrom(s.desc) });
    }
    if (s.step === 4) return tbWizCreate();
    s.step += 1;
    s.error = "";
    tbWizDraw();
  },
  "add-step"() {
    tbWiz.steps.push("");
    tbWiz.focus = `[data-tb-input="step"][data-i="${tbWiz.steps.length - 1}"]`;
    tbWizDraw();
  },
  "remove-step"(b) {
    tbWiz.steps.splice(Number(b.dataset.i), 1);
    tbWizDraw();
  },
  once() {
    tbWiz.once = !tbWiz.once;
    tbWizDraw();
  },
  who(b) {
    const who = b.dataset.kind === "group" ? { kind: "group", handle: b.dataset.id } : b.dataset.kind === "person" ? { kind: "person", account: b.dataset.id } : { kind: "owner" };
    tbWiz.who[b.dataset.moment] = who;
    tbWizDraw();
  },
  project(b) {
    tbWiz.project = b.dataset.key;
    tbWizDraw();
  },
  async "turn-on"() {
    const s = tbWiz;
    s.busy = true;
    s.error = "";
    tbWizDraw();
    try {
      await tbCall({ op: "turn_bot_on", workflow: s.workflow, on: true });
      s.on = true;
    } catch (e) {
      s.error = TB_TEXT.wiz.turnFailed(tbErrorText(e));
    }
    s.busy = false;
    tbWizDraw();
  },
  "see-bot": () => openTeamBot(tbWiz.workflow),
};

function showWizard() {
  tbWizReset();
  if (!tbTeam()) return tbWizDraw();
  tbWizLoad();
}

// ---- Wiring -------------------------------------------------------------------------------------

const TB_ACTS = { "team-bot": TB_BOT_ACTS, "team-group": TB_GROUP_ACTS, "team-bot-new": TB_WIZ_ACTS };

if (typeof pageView !== "undefined" && pageView?.addEventListener) {
  pageView.addEventListener("click", (e) => {
    const page = tbShown();
    const acts = TB_ACTS[page];
    const b = acts && e.target.closest("[data-tb]");
    if (b && !b.disabled) acts[b.dataset.tb]?.(b);
  });
  // Typing keeps the page as it is: the text goes to the state and nothing is drawn again.
  pageView.addEventListener("input", (e) => {
    if (tbShown() !== "team-bot-new") return;
    const field = e.target?.dataset?.tbInput;
    if (!field) return;
    const value = String(e.target.value ?? "");
    if (field === "desc") {
      tbWiz.desc = value;
      if (tbWiz.idea) {
        tbWiz.idea = null;
        for (const el of pageView.querySelectorAll?.('[data-tb="idea"]') ?? []) el.setAttribute("aria-pressed", "false");
      }
      tbWizSyncNext();
    } else if (field === "name") {
      tbWiz.name = value;
      tbWizSyncNext();
    } else if (field === "step") {
      tbWiz.steps[Number(e.target.dataset.i)] = value;
      tbWizSyncNext();
    }
  });
}
/** Keeps the Continue button's enabled state in step with the typing. */
function tbWizSyncNext() {
  const next = pageView?.querySelector?.('[data-tb="next"]');
  if (next) next.disabled = !tbWizReady() || tbWiz.busy;
}

const tbDoc = tbDocument();
if (tbDoc?.addEventListener) {
  // A team chosen in the account menu: the pages ask again, or ask for a team.
  tbDoc.addEventListener("teams:changed", () => {
    const page = tbShown();
    if (page === "team-bot") showBot();
    else if (page === "team-group") showGroup();
    else if (page === "team-bot-new") showWizard();
  });
  // The hub pushes team changes (a run moved, a call opened or was answered): the bot and group pages read again.
  tbDoc.addEventListener("teams:event", () => {
    const page = tbShown();
    if (page === "team-bot" && tbBotUi.state === "ready" && !tbBotUi.busy) tbLoadBot(true);
    else if (page === "team-group" && tbGroupUi.state === "ready" && !tbGroupUi.busy) tbLoadGroup(true);
  });
}

if (typeof SHELL_PAGES !== "undefined") {
  SHELL_PAGES["team-bot"] = { show: showBot };
  SHELL_PAGES["team-bot-new"] = { show: showWizard };
  SHELL_PAGES["team-group"] = { show: showGroup };
}
