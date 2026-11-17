/** pinned Teams, the way in: the "Where you work" block of the Account and pages menu, the Create a
 * team teamWizard and the Join a team page. A team call is the hub's `team` call (crates/core/src/teams.rs);
 * this file also owns the contract the other Teams pages read: `teamsCurrent()`, `teamsList()`,
 * `teamCall(op)` (function declarations, so real `globalThis` properties) and the `teams:changed` event.
 *
 * What people typed (team names, project names) reaches the DOM through `esc()` or `textContent`.
 * A key typed in a form is read from its field, sent, and the field emptied: it is never kept in
 * state, never drawn again and never logged. */

const TEAMS_KEY = "teams-current";
const TEAM_PAGES = ["team-create", "team-join", "team-invites"];
const TEAM_INVITES_MAX = 20;
const TEAM_HUES = ["#f0a35e", "#5fd4b8", "#c7a6ff", "#6ea8ff", "#f08aa8", "#e3cf6a"];

/** Next to every Claude token field: whose account the token is from. */
const CLAUDE_SOURCE = "The token comes from the Claude account your terminal is signed in to. Sign in to the team's own Claude account first.";

const TEAMS_TEXT = {
  where: "Where you work",
  justMe: "Just me",
  here: "you are here",
  roles: { owner: "Owner", member: "Member" },
  create: "Create a team",
  join: "Join a team",
  accountName: "Account and pages",
  noHub: "The hub did not answer.",

  createTitle: "Create a team",
  createAbout: "Four steps, about 2 minutes. You can go back and change any of them.",
  stepsLabel: "Steps to create the team",
  sideNote: "Your personal account stays as it is. A team is one more place, and you switch between them in the same account menu.",
  stepOf: (n) => `Step ${n} of 4`,
  steps: [
    { name: "Name", hint: "What the team is called" },
    { name: "Projects", hint: "Which of your projects go in" },
    { name: "The team's AI account", hint: "What signs the bots in" },
    { name: "People", hint: "Who works with you" },
    { name: "Done", hint: "Open the team and make the first bot" },
  ],
  backToAccount: "Back to my account",
  back: "Back",
  cancel: "Cancel",
  working: "Working…",
  next: ["Continue", "Continue", "Continue", "Finish"],
  nextHints: ["Next: choose the projects", "Next: the team's AI account", "Next: invite people", "Last step"],
  connectLater: "Connect later",

  nameTitle: "What is the team called?",
  nameLabel: "Team name",
  namePlaceholder: "E.g. North Studio",
  nameOwner: "You are the owner. You can rename it later.",
  nameHas: {
    title: "What a team has",
    lines: [
      "Bots that do the repeated work on their own, like running the tests or reviewing a pull request.",
      "Groups of people the bots call when they need a yes or an answer.",
    ],
  },
  nameEmpty: "Give the team a name.",

  projectsTitle: (team) => `Which of your projects go to ${team}?`,
  projectsAbout: "In the projects that go in, the team's bots can work. The others stay yours.",
  projectsNone: "No projects on your machines yet.",
  projectIn: "Goes to the team",
  projectOut: "Only yours",
  projectNoRepo: "no remote: the team finds a project by its repository",
  attachFailed: (name, e) => `Could not add ${name}: ${e}`,

  aiTitle: "The team has its own AI account",
  aiAbout: "The team's bots never use your subscription or a teammate's. The cost is the team's, and no bot stops when someone leaves.",
  creds: [
    { id: "upstash", name: "Upstash key", about: "Opens the boxes the bots work in" },
    { id: "claude", name: "Claude token for boxes", about: "Signs the bots in to Claude inside the box" },
    { id: "github", name: "GitHub token", about: "Clones the team's repositories" },
  ],
  connected: "Connected",
  connectNow: "Connect now",
  ownerOnly: "Only the owner connects these.",
  keyAccess: "The bot can do whatever the team's keys allow, so connect keys with the least access the bot needs.",
  keptNote: "You paste the key in a form here in pinned. It is kept with the team and is never shown again.",
  noRush: "No rush: you can connect later. Until then you can set up bots and groups, but no bot runs.",
  form: {
    upstash: { title: "Connect the Upstash key", fields: [{ id: "key", label: "Upstash Box key", placeholder: "box_…" }, { id: "claude_token", label: "Claude token", placeholder: "sk-ant-oat01-…", note: CLAUDE_SOURCE }], about: "Create the Box API key in the Box tab of the Upstash console. The Claude token comes from claude setup-token." },
    claude: { title: "Connect the Claude token for boxes", fields: [{ id: "name", label: "Name", placeholder: "team", plain: true, value: "team" }, { id: "token", label: "Claude token", placeholder: "sk-ant-oat01-…", note: CLAUDE_SOURCE }], about: "Run claude setup-token in a terminal and paste what it prints. Connect the Upstash key first." },
    github: { title: "Connect the GitHub token", fields: [{ id: "token", label: "GitHub token", placeholder: "ghp_…" }], about: "A token that can read the repositories the bots clone." },
  },
  save: "Connect",
  sending: "Connecting…",
  closeForm: "Cancel",
  fieldMissing: "Fill in every field.",

  peopleTitle: "Who works with you?",
  inviteMake: "Make an invite code",
  inviteMaking: "Making…",
  inviteSend: "Send this code to the person. It works once.",
  inviteCopy: "Copy",
  inviteCopied: "Copied",
  inviteAbout: "Each person signs in with their own pinned account, pastes the code and picks which of their projects belong to the team. You pass each code on to the person yourself. The bots call people through groups. You can skip this step.",
  inviteShown: "Shown only now.",

  invitesTitle: "Invite people",
  invitesAbout: "Make an invite code and pass it on to the person yourself. A code works once. It stays open until the person uses it or you revoke it.",
  invitesOwnerOnly: "Only the team's owner makes invite codes.",
  invitesOpen: "Open invites",
  invitesNone: "No open invites.",
  invitesLoading: "Loading the open invites…",
  invitesMade: (when) => `Made ${when}`,
  invitesBy: (by) => `by ${by}`,
  invitesCount: (n) => `${n} of ${TEAM_INVITES_MAX} open`,
  inviteRevoke: "Revoke",
  inviteRevoking: "Revoking…",
  inviteRevoked: (hint) => `Revoked ${hint}…`,

  readyTitle: (team) => `${team} is ready`,
  readyNoBot: "The team has no bots yet. Start with one, or open the team and look around. To go back to your account, use the same account menu.",
  readyProjects: (n) => (n === 0 ? "No projects added." : n === 1 ? "1 project added." : `${n} projects added.`),
  readyAi: (have, all) => (have === all ? " The team's AI account is connected." : have === 0 ? " The team's AI account is not connected yet, so no bot runs until it is." : ` ${have} of ${all} keys of the team's AI account are connected, so no bot runs until the rest are.`),
  readyInvites: (n) => (n === 0 ? " No invite codes made." : n === 1 ? " 1 invite code made. You pass it on yourself." : ` ${n} invite codes made. You pass them on yourself.`),
  firstBot: "Create the first bot",
  openTeam: "Open the team",

  joinTitle: "Join a team",
  joinAbout: "Someone on the team makes you an invite code. It works once.",
  codeLabel: "Paste the invite code",
  codePlaceholder: "ABCD-EFGH",
  joinGo: "Join",
  teamJoining: "Joining…",
  joinedTitle: (team) => `You joined ${team}`,
  joinedAbout: "You keep working on your own computer, your own way. Pick which of your projects the team's bots may work in.",
  joinWhich: "Which of your projects belong to the team?",
  joinMatch: (n) => (n === 1 ? "1 of yours matches a project the team already has." : `${n} of yours match projects the team already has.`),
  joinNoMatch: "The team has no project that matches yours yet. You can still add some.",
  joinProjectsFailed: (e) => `Could not read the team's projects: ${e}`,
  teamHas: "The team already has it",
  teamSees: "The team sees",
  onlyYou: "Only you",
  seeTitle: "What the team will see",
  seeLines: [
    "The names and repositories of the projects you add, and the runs of the team's bots in them.",
    "Never your conversations, your other projects or your passwords. You can take a project out any time.",
  ],
  joinWith: (n) => (n === 0 ? "Join the team" : n === 1 ? "Join with 1 project" : `Join with ${n} projects`),
  joinWithout: "Continue without any",

  connectTitle: "Connect AI account",
  connectWho: "Team",
  connectAbout: "The keys the team's bots run with. Only the owner connects them.",
  connectSide: "Only this team's keys change. The team's name, projects and people stay as they are.",
  connectDone: "Done",
  connectDoneHint: "Back to where you were",
};

const teamsEsc = (v) => (typeof esc === "function" ? esc(v) : String(v ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`));
const teamsHue = (name) => TEAM_HUES[[...String(name)].reduce((h, c) => (h * 31 + c.codePointAt(0)) >>> 0, 7) % TEAM_HUES.length];
const teamsInitials = (name) => {
  const words = String(name ?? "").split(/[\s@._-]+/).filter(Boolean);
  return ((words.length > 1 ? words[0][0] + words[1][0] : (words[0] ?? "").slice(0, 2)) || "?").toUpperCase();
};
const teamsAvatar = (name, size = "", hue = teamsHue(name)) => `<span class="tm-avatar ${size}" aria-hidden="true" style="background:${hue}">${teamsEsc(teamsInitials(name))}</span>`;
const teamsErrorText = (e) => String(e?.message ?? e ?? TEAMS_TEXT.noHub);
/** A project's repository as the hub keys it, github.com/owner/repo, from whatever spelling a machine reported. */
function teamsRepoKey(repo) {
  const url = String(repo ?? "").trim().replace(/\/+$/, "").replace(/\.git$/, "");
  if (!url) return "";
  const rest = url.includes("://") ? url.slice(url.indexOf("://") + 3) : url.replace(":", "/");
  return (rest.includes("@") ? rest.slice(rest.lastIndexOf("@") + 1) : rest).toLowerCase();
}
/** The person's own projects that a team can take: one entry per repository, `key` being what attach_project sends. */
function teamsOwnProjects() {
  const catalog = typeof globalProjectsCatalog === "function" ? globalProjectsCatalog() : null;
  const seen = new Set();
  const out = [];
  for (const p of catalog?.projects ?? []) {
    const key = teamsRepoKey(p.repo);
    if (key && seen.has(key)) continue;
    if (key) seen.add(key);
    out.push({ name: p.name, key, repo: p.repo ?? "" });
  }
  return out;
}

// ---- The contract: the current team, the list, and the call -------------------------------------

let teamsKnown = [];
let teamsLoaded = false;
let teamsNow = null;
try {
  const kept = JSON.parse(load(TEAMS_KEY) ?? "null");
  if (kept && typeof kept.id === "string" && kept.id) teamsNow = { id: kept.id, name: String(kept.name ?? ""), role: kept.role === "owner" ? "owner" : "member", credential: kept.credential ?? {}, me: typeof kept.me === "string" && kept.me ? kept.me : null };
} catch {}
// A team remembered from the last run shows its home once, at the first list from the hub: until the hub answers,
// boot still shows the personal thread, and only then can a page open over it.
let teamsStartHome = teamsNow !== null;

/** The team this viewer works in, `{id, name, role, credential, me}`, or null for the personal account.
 * `me` is the viewer's own account id. The wire's `Team` names only the owner's (`owner`), so `me` is set
 * for the owner and is null for a member, whose id the Members list gives by e-mail. */
function teamsCurrent() {
  return teamsNow;
}

/** A call about a team, run by the hub on the team's account. `List`, `Create` and `Join` take no team. */
function teamCall(op, team = teamsNow?.id) {
  const free = op.op === "list" || op.op === "create" || op.op === "join";
  if (!free && !team) return Promise.reject("no team is chosen");
  return hub.hub({ cmd: "team", ...(free ? {} : { team }), op });
}

const teamView = (t) => {
  const role = t.role === "owner" ? "owner" : "member";
  // The hub sends `line_held` to the owner only; a member's view never carries it.
  const held = role === "owner" && t.line_held ? { line_held: t.line_held } : {};
  return { id: t.id, name: t.name, role, credential: t.credential ?? {}, me: typeof t.me === "string" && t.me ? t.me : role === "owner" && typeof t.owner === "string" && t.owner ? t.owner : null, ...held };
};

function teamsRemember() {
  save(TEAMS_KEY, teamsNow ? JSON.stringify(teamsNow) : null);
}

/** A team page is open: a `team-*` page the shell knows (the Calls, bot, group, run, invites and create pages). */
const teamPageShown = () => {
  const page = document.body.dataset.page ?? "";
  return page.startsWith("team-") && page in SHELL_PAGES;
};

/** Makes `team` (a Team of the list, or null for the personal account) the current one. A change of team sets the
 * main area: a team opens its home, and the personal account closes a team page so the thread shows. A page that is
 * not a team's is left as it is. `home: false` is for a caller that opens its own page right after. */
function teamsSelect(team, { home = true } = {}) {
  const before = teamsNow?.id ?? null;
  teamsNow = team ? teamView(team) : null;
  teamsRemember();
  drawTeamsButton();
  drawTeamsWhere();
  const now = teamsNow?.id ?? null;
  if (now === before) return;
  document.dispatchEvent(new CustomEvent("teams:changed", { detail: teamsNow }));
  if (!home) return;
  if (now) teamsOpenHome();
  else if (teamPageShown()) closePage();
}

/** The teams the person belongs to. Also keeps the current team's name, role and keys up to date,
 * and goes back to the personal account when the person is no longer in it. */
async function teamsList() {
  const list = await teamCall({ op: "list" });
  teamsKnown = Array.isArray(list) ? list : [];
  teamsLoaded = true;
  if (teamsNow) {
    const fresh = teamsKnown.find((t) => t.id === teamsNow.id);
    if (!fresh) teamsSelect(null);
    else {
      teamsNow = teamView(fresh);
      teamsRemember();
    }
  }
  drawTeamsButton();
  drawTeamsWhere();
  // No team event tells of a held line or its end: the nav shows what this read found.
  if (typeof tcNavDraw === "function") tcNavDraw();
  teamsPollArm();
  if (teamsStartHome) {
    teamsStartHome = false;
    if (teamsNow) teamsOpenHome();
  }
  return teamsKnown;
}

/** How often an owner's list is read again while a team of theirs holds its line, and otherwise. */
const TEAMS_HELD_POLL_MS = 30_000;
const TEAMS_OWNER_POLL_MS = 120_000;
let teamsPollTimer = null;

/** Arms the one timer that reads the list again, for an owner, while the hub is online and the
 * page is visible; clears it otherwise. */
function teamsPollArm() {
  if (teamsPollTimer !== null) clearTimeout(teamsPollTimer);
  teamsPollTimer = null;
  const owned = teamsKnown.filter((t) => t.role === "owner");
  if (!owned.length || document.body?.dataset.hub !== "online" || document.visibilityState === "hidden") return;
  const wait = owned.some((t) => t.line_held) ? TEAMS_HELD_POLL_MS : TEAMS_OWNER_POLL_MS;
  teamsPollTimer = setTimeout(() => {
    teamsPollTimer = null;
    teamsList().catch(() => teamsPollArm());
  }, wait);
}
document.addEventListener?.("visibilitychange", () => teamsPollArm());

// ---- Where you work: the menu block and the menu button -----------------------------------------

const teamsMenu = document.getElementById("account-menu");
const teamsMenuButton = document.getElementById("account-pages");
const teamsAvatarSlot = teamsMenuButton?.querySelector(".avatar");
const teamsAvatarIcon = teamsAvatarSlot?.innerHTML ?? "";

function teamsPerson() {
  const email = typeof account !== "undefined" && account?.email ? String(account.email) : "";
  return email ? email.split("@")[0] : "You";
}

function teamsWhereHtml() {
  const t = TEAMS_TEXT;
  const row = (id, name, avatar, hint) =>
    `<button type="button" role="menuitem" class="tm-row" data-team-pick="${teamsEsc(id)}"${(teamsNow?.id ?? "") === id ? ' aria-current="true"' : ""}>${avatar}<span class="label">${teamsEsc(name)}</span><span class="hint">${teamsEsc(hint)}</span></button>`;
  const rows = teamsKnown.map((team) => row(team.id, team.name, teamsAvatar(team.name), teamsNow?.id === team.id ? t.here : t.roles[team.role] ?? ""));
  return `<div class="tm-where" role="group" aria-label="${t.where}">
    <p class="tm-where-head">${t.where}</p>
    ${row("", t.justMe, teamsAvatar(teamsPerson(), "", TEAM_HUES[0]), teamsNow ? "" : t.here)}
    ${rows.join("")}
    <button type="button" role="menuitem" class="tm-create" data-team-page="team-create"><span class="tm-plus" aria-hidden="true">+</span><span class="label">${t.create}</span></button>
    <button type="button" role="menuitem" class="tm-join" data-team-page="team-join"><span class="tm-plus" aria-hidden="true">→</span><span class="label">${t.join}</span></button>
  </div><div class="tm-sep" role="separator"></div>`;
}

function drawTeamsWhere() {
  const slot = document.getElementById("teams-where");
  if (slot) slot.innerHTML = teamsWhereHtml();
}

/** The menu button names the team the person works in; the personal account keeps its own label. */
function drawTeamsButton() {
  const name = teamsMenuButton?.querySelector(".name");
  if (name) name.textContent = teamsNow ? teamsNow.name : TEAMS_TEXT.accountName;
  if (teamsAvatarSlot) {
    if (teamsNow) {
      teamsAvatarSlot.innerHTML = `<span class="tm-mono">${teamsEsc(teamsInitials(teamsNow.name))}</span>`;
      teamsAvatarSlot.style.background = teamsHue(teamsNow.name);
      teamsAvatarSlot.dataset.team = "true";
    } else {
      teamsAvatarSlot.innerHTML = teamsAvatarIcon;
      teamsAvatarSlot.style.background = "";
      delete teamsAvatarSlot.dataset.team;
    }
  }
  teamsMenuButton?.classList.toggle("in-team", !!teamsNow);
}

function openTeamPage(name) {
  if (name in SHELL_PAGES) openPage(name);
}

teamsMenu?.addEventListener("click", (e) => {
  const pick = e.target.closest("[data-team-pick]");
  if (pick) {
    const id = pick.dataset.teamPick;
    teamsSelect(id ? teamsKnown.find((t) => t.id === id) ?? null : null);
    return;
  }
  const page = e.target.closest("[data-team-page]");
  if (page) openTeamPage(page.dataset.teamPage);
});
// The menu opens through gui-sidebar.js; the block reads the teams again each time it does.
if (teamsMenu) {
  new MutationObserver(() => {
    if (teamsMenu.hidden) return;
    drawTeamsWhere();
    teamsList().catch(() => {});
  }).observe(teamsMenu, { attributeFilter: ["hidden"] });
}
// The hub answers once the app is online: the saved team's name and keys are read then.
if (typeof MutationObserver === "function" && document.body) {
  new MutationObserver(() => (document.body.dataset.hub === "online" ? teamsList().catch(() => {}) : teamsPollArm())).observe(document.body, { attributeFilter: ["data-hub"] });
}

// ---- Shared page pieces -------------------------------------------------------------------------

const tmButton = (attrs, label, kind = "secondary", disabled = false) => `<button type="button" class="tm-btn ${kind}" ${attrs}${disabled ? " disabled" : ""}>${teamsEsc(label)}</button>`;
const tmError = (text) => (text ? `<p class="tm-error" role="alert">${teamsEsc(text)}</p>` : "");
const tmCheckRow = (attrs, on, name, about, state) =>
  `<button type="button" class="tm-check" role="switch" aria-checked="${on}" ${attrs}><span class="tm-box" aria-hidden="true"></span><span class="tm-texts"><span class="tm-name">${teamsEsc(name)}</span>${about ? `<span class="tm-about">${teamsEsc(about)}</span>` : ""}</span><span class="tm-state">${teamsEsc(state)}</span></button>`;

/** The team's home is its Calls page. */
function teamsOpenHome() {
  openPage("team-calls");
}

// ---- Create a team ------------------------------------------------------------------------------

const teamWizard = {};
function resetWizard(own = teamsOwnProjects()) {
  Object.assign(teamWizard, {
    step: 1,
    name: "",
    team: null,
    busy: false,
    error: "",
    own,
    on: {},
    attached: new Set(),
    form: null,
    formBusy: false,
    formError: "",
    invites: [],
    inviteBusy: false,
    inviteError: "",
    copied: "",
    connect: false,
    origin: "",
  });
}
// The projects are read when the page opens: the catalog is not ready while this script loads.
resetWizard([]);

const teamWizardTeamName = () => (teamWizard.team?.name ?? teamWizard.name).trim() || "the team";
const teamWizardCreds = () => teamWizard.team?.credential ?? {};
const teamWizardOwner = () => (teamWizard.team?.role ?? "owner") === "owner";
const connectedCount = () => TEAMS_TEXT.creds.filter((c) => teamWizardCreds()[c.id]).length;

function teamWizardStepSub(n) {
  const t = TEAMS_TEXT;
  if (n >= teamWizard.step) return t.steps[n - 1].hint;
  if (n === 1) return teamWizardTeamName();
  if (n === 2) return t.readyProjects(teamWizard.attached.size).replace(/\.$/, "");
  if (n === 3) return `${connectedCount()} of ${t.creds.length} connected`;
  if (n === 4) return teamWizard.invites.length ? `${teamWizard.invites.length} ${teamWizard.invites.length === 1 ? "code" : "codes"} made` : "No codes made";
  return "";
}

function teamWizardStepsHtml() {
  return TEAMS_TEXT.steps
    .map((s, i) => {
      const n = i + 1;
      const state = n === teamWizard.step ? "current" : n < teamWizard.step ? "done" : "todo";
      // A step ahead stays shut until Continue gets there; one behind can be revisited, which repeats no create.
      const shut = n > teamWizard.step;
      return `<li><button type="button" class="tm-step ${state}" data-tm="go" data-step="${n}"${n === teamWizard.step ? ' aria-current="step"' : ""}${shut || teamWizard.busy ? " disabled" : ""}><span class="tm-num" aria-hidden="true">${state === "done" ? "✓" : n}</span><span class="tm-texts"><span class="tm-name">${teamsEsc(s.name)}</span><span class="tm-about">${teamsEsc(teamWizardStepSub(n))}</span></span></button></li>`;
    })
    .join("");
}

function teamWizardCredHtml() {
  const t = TEAMS_TEXT;
  const owner = teamWizardOwner();
  const rows = t.creds
    .map((c) => {
      const on = !!teamWizardCreds()[c.id];
      const state = on ? `<span class="tm-ok">${t.connected}</span>` : owner ? tmButton(`data-tm="connect" data-cred="${c.id}"`, t.connectNow) : `<span class="tm-about">Not connected</span>`;
      return `<div class="tm-cred" data-cred="${c.id}"><span class="tm-texts"><span class="tm-name">${teamsEsc(c.name)}</span><span class="tm-about">${teamsEsc(c.about)}</span></span>${state}</div>${teamWizard.form === c.id ? teamWizardFormHtml(c.id) : ""}`;
    })
    .join("");
  return `<div class="tm-creds">${rows}</div>${owner ? "" : `<p class="tm-note">${t.ownerOnly}</p>`}`;
}

function teamWizardFormHtml(cred) {
  const f = TEAMS_TEXT.form[cred];
  const fields = f.fields
    .map((x) => `<label class="tm-field"><span>${teamsEsc(x.label)}</span><input data-field="${x.id}" type="${x.plain ? "text" : "password"}" autocomplete="off" spellcheck="false" placeholder="${teamsEsc(x.placeholder)}"${x.value ? ` value="${teamsEsc(x.value)}"` : ""}>${x.plain ? "" : `<span class="tm-field-note">${teamsEsc(TEAMS_TEXT.keyAccess)}</span>`}${x.note ? `<span class="tm-field-note">${teamsEsc(x.note)}</span>` : ""}</label>`)
    .join("");
  return `<form class="tm-form" data-tm="form" data-cred="${cred}" autocomplete="off">
    <h3>${f.title}</h3>${fields}<p class="tm-note">${teamsEsc(f.about)}</p>${tmError(teamWizard.formError)}
    <div class="tm-actions">${teamWizard.formBusy ? tmButton("", TEAMS_TEXT.sending, "primary", true) : `<button type="submit" class="tm-btn primary">${TEAMS_TEXT.save}</button>${tmButton('data-tm="close-form"', TEAMS_TEXT.closeForm, "quiet")}`}</div>
  </form>`;
}

function teamWizardInvitesHtml() {
  const t = TEAMS_TEXT;
  const codes = teamWizard.invites
    .map((inv, i) => `<div class="tm-code"><code>${teamsEsc(inv.code)}</code>${tmButton(`data-tm="copy" data-i="${i}"`, teamWizard.copied === inv.code ? t.inviteCopied : t.inviteCopy)}</div>`)
    .join("");
  const note = teamWizard.invites.length ? `<p class="tm-note">${t.inviteSend} ${t.inviteShown}</p>` : "";
  return `${codes}${note}${tmError(teamWizard.inviteError)}<div class="tm-actions">${tmButton('data-tm="invite"', teamWizard.inviteBusy ? t.inviteMaking : t.inviteMake, teamWizard.invites.length ? "secondary" : "primary", teamWizard.inviteBusy)}</div>`;
}

function teamWizardBodyHtml() {
  const t = TEAMS_TEXT;
  const team = teamsEsc(teamWizardTeamName());
  const step = teamWizard.step;
  const kicker = step <= 4 && !teamWizard.connect ? `<p class="tm-kicker">${t.stepOf(step)}</p>` : "";
  if (step === 1) {
    return `${kicker}<h2>${t.nameTitle}</h2>
      <label class="tm-field big"><span>${t.nameLabel}</span><input id="tm-team-name" type="text" value="${teamsEsc(teamWizard.name)}" placeholder="${t.namePlaceholder}" autocomplete="off" spellcheck="false"></label>
      <p class="tm-note">${t.nameOwner}</p>
      <div class="tm-card"><b>${t.nameHas.title}</b>${t.nameHas.lines.map((l) => `<span>${l}</span>`).join("")}</div>`;
  }
  if (step === 2) {
    const rows = teamWizard.own.map((p) => {
      const key = p.key || p.name;
      return tmCheckRow(`data-tm="toggle" data-key="${teamsEsc(key)}"${p.key ? "" : " disabled"}`, !!teamWizard.on[key] && !!p.key, p.name, p.key ? "" : t.projectNoRepo, teamWizard.on[key] && p.key ? t.projectIn : t.projectOut);
    });
    return `${kicker}<h2>${t.projectsTitle(team)}</h2><p class="tm-lede">${t.projectsAbout}</p>
      <div class="tm-list">${rows.join("") || `<p class="tm-note">${t.projectsNone}</p>`}</div>`;
  }
  if (step === 3) {
    return `${kicker}<h2>${t.aiTitle}</h2><p class="tm-lede">${t.aiAbout}</p>${teamWizardCredHtml()}<p class="tm-note">${t.keptNote}</p><p class="tm-note">${t.noRush}</p>`;
  }
  if (step === 4) {
    return `${kicker}<h2>${t.peopleTitle}</h2>${teamWizardInvitesHtml()}<p class="tm-note">${t.inviteAbout}</p>`;
  }
  const people = teamWizard.invites.slice(0, 2).map(() => `<span class="tm-avatar big ghost" aria-hidden="true">+</span>`).join("");
  const summary = t.readyProjects(teamWizard.attached.size) + t.readyAi(connectedCount(), t.creds.length) + t.readyInvites(teamWizard.invites.length);
  return `<div class="tm-faces">${teamsAvatar(teamsPerson(), "big", TEAM_HUES[0])}${people}</div>
    <h2>${t.readyTitle(team)}</h2><p class="tm-lede big">${teamsEsc(summary)}</p><p class="tm-note">${t.readyNoBot}</p>
    <div class="tm-actions">${tmButton('data-tm="first-bot"', t.firstBot, "primary")}${tmButton('data-tm="open-team"', t.openTeam)}</div>`;
}

function teamWizardNavHtml() {
  const t = TEAMS_TEXT;
  const step = teamWizard.step;
  if (teamWizard.connect) return `<div class="tm-actions">${tmButton('data-tm="connect-done"', t.connectDone, "primary")}<span class="tm-hint">${t.connectDoneHint}</span></div>`;
  if (step > 4) return "";
  const label = step === 3 && teamWizardOwner() && connectedCount() < t.creds.length ? t.connectLater : t.next[step - 1];
  return `${tmError(teamWizard.error)}<div class="tm-actions">${tmButton('data-tm="next"', teamWizard.busy ? t.working : label, "primary", teamWizard.busy)}<span class="tm-hint">${t.nextHints[step - 1]}</span></div>`;
}

function teamWizardTopHtml() {
  const t = TEAMS_TEXT;
  const step = teamWizard.step;
  if (teamWizard.connect) return `<div class="tm-top">${tmButton('data-tm="leave"', `‹ ${t.back}`)}</div>`;
  const left = step === 1 ? tmButton('data-tm="leave"', `‹ ${t.backToAccount}`) : tmButton('data-tm="prev"', `‹ ${t.back}`, "secondary", teamWizard.busy);
  const right = step <= 4 ? tmButton('data-tm="leave"', t.cancel, "quiet") : "";
  return `<div class="tm-top">${left}<span class="tm-grow"></span>${right}</div>`;
}

function teamWizardAsideHtml() {
  const t = TEAMS_TEXT;
  if (teamWizard.connect) {
    const team = teamsEsc(teamWizardTeamName());
    return `<aside class="tm-aside" aria-label="${t.connectTitle}">
        <div class="tm-who">${teamsAvatar(teamWizardTeamName(), "big")}<span class="tm-texts"><span class="tm-name">${team}</span><span class="tm-about">${t.connectWho}</span></span></div>
        <div class="tm-intro"><h1>${t.connectTitle}</h1><p>${t.connectAbout}</p></div>
        <p class="tm-side-note">${t.connectSide}</p>
      </aside>`;
  }
  const person = teamsPerson();
  return `<aside class="tm-aside" aria-label="${t.stepsLabel}">
        <div class="tm-who">${teamsAvatar(person, "big", TEAM_HUES[0])}<span class="tm-texts"><span class="tm-name">${teamsEsc(person)}</span><span class="tm-about">personal account</span></span></div>
        <div class="tm-intro"><h1>${t.createTitle}</h1><p>${t.createAbout}</p></div>
        <ol class="tm-steps">${teamWizardStepsHtml()}</ol>
        <p class="tm-side-note">${t.sideNote}</p>
      </aside>`;
}

function renderCreate() {
  const t = TEAMS_TEXT;
  const keep = document.getElementById("tm-team-name");
  const focused = keep && document.activeElement === keep;
  setPage(
    `${headHtml(teamWizard.connect ? t.connectTitle : t.createTitle)}<div class="page-body"><div class="tm-wiz">
      ${teamWizardAsideHtml()}
      <section class="tm-main">${teamWizardTopHtml()}<div class="tm-col">${teamWizardBodyHtml()}${teamWizardNavHtml()}</div></section>
    </div></div>`,
  );
  const input = document.getElementById("tm-team-name");
  if (input && focused) input.focus();
}

/** Continue on step 1: creates the team once, renames it when the name changed after, and goes on. */
async function teamWizardName() {
  const name = teamWizard.name.trim();
  if (!name) throw TEAMS_TEXT.nameEmpty;
  if (!teamWizard.team) {
    teamWizard.team = teamView(await teamCall({ op: "create", name }));
    teamsList().catch(() => {});
  } else if (teamWizard.team.name !== name) {
    await teamCall({ op: "rename", name }, teamWizard.team.id);
    teamWizard.team = { ...teamWizard.team, name };
    teamsList().catch(() => {});
  }
}

/** Continue on step 2: attaches each project turned on that the team lacks, detaches one turned off after. */
async function teamWizardProjects() {
  const id = teamWizard.team.id;
  for (const p of teamWizard.own) {
    if (!p.key) continue;
    const on = !!teamWizard.on[p.key];
    const had = teamWizard.attached.has(p.key);
    try {
      if (on && !had) {
        await teamCall({ op: "attach_project", key: p.key }, id);
        teamWizard.attached.add(p.key);
      } else if (!on && had) {
        await teamCall({ op: "detach_project", key: p.key }, id);
        teamWizard.attached.delete(p.key);
      }
    } catch (e) {
      throw TEAMS_TEXT.attachFailed(p.name, teamsErrorText(e));
    }
  }
}

async function teamWizardNext() {
  if (teamWizard.busy || teamWizard.step > 4) return;
  teamWizard.busy = true;
  teamWizard.error = "";
  renderCreate();
  try {
    if (teamWizard.step === 1) await teamWizardName();
    else if (teamWizard.step === 2) await teamWizardProjects();
    teamWizard.step += 1;
    teamWizard.form = null;
  } catch (e) {
    teamWizard.error = teamsErrorText(e);
  }
  teamWizard.busy = false;
  renderCreate();
}

/** The personal call inside a team call, for the key form `cred`, with what the person typed. */
function credentialCall(cred, v) {
  if (cred === "upstash") return { cmd: "connect_upstash", key: v.key, claude_token: v.claude_token };
  if (cred === "claude") return { cmd: "set_box_claude_token", name: v.name, token: v.token };
  return { cmd: "connect_github", token: v.token };
}

async function teamWizardConnect(cred, values) {
  if (teamWizard.formBusy) return;
  const fields = TEAMS_TEXT.form[cred].fields;
  if (fields.some((f) => !String(values[f.id] ?? "").trim())) {
    teamWizard.formError = TEAMS_TEXT.fieldMissing;
    return renderCreate();
  }
  teamWizard.formBusy = true;
  teamWizard.formError = "";
  renderCreate();
  try {
    await teamCall({ op: "hub", call: credentialCall(cred, Object.fromEntries(fields.map((f) => [f.id, String(values[f.id]).trim()]))) }, teamWizard.team.id);
    teamWizard.form = null;
    const fresh = await teamsList().then((l) => l.find((x) => x.id === teamWizard.team.id), () => null);
    teamWizard.team = fresh ? teamView(fresh) : { ...teamWizard.team, credential: { ...teamWizardCreds(), [cred]: true } };
  } catch (e) {
    teamWizard.formError = teamsErrorText(e);
  }
  teamWizard.formBusy = false;
  renderCreate();
}

async function teamWizardInvite() {
  if (teamWizard.inviteBusy) return;
  teamWizard.inviteBusy = true;
  teamWizard.inviteError = "";
  renderCreate();
  try {
    const made = await teamCall({ op: "invite" }, teamWizard.team.id);
    teamWizard.invites.push({ code: String(made.code), hint: String(made.hint ?? "") });
  } catch (e) {
    teamWizard.inviteError = teamsErrorText(e);
  }
  teamWizard.inviteBusy = false;
  renderCreate();
}

function teamWizardCopy(i) {
  const code = teamWizard.invites[i]?.code;
  if (!code) return;
  const done = () => {
    teamWizard.copied = code;
    renderCreate();
  };
  try {
    navigator.clipboard.writeText(code).then(done, () => {});
  } catch {}
}

/** Makes the new team the one the person works in, then `then` runs. */
function teamWizardFinish(then) {
  const team = teamWizard.team;
  if (team) teamsSelect(team, { home: false });
  then();
}

/** Set by Connect AI account just before the create page opens, read once by that page. */
let teamConnectAsk = null;

/** Connect AI account: opens step 3 for the current team, owner only, and remembers the page it was asked from. */
function openTeamConnect() {
  if (teamsNow?.role !== "owner") return;
  teamConnectAsk = { origin: (typeof shellPage === "function" && shellPage()) || "" };
  openPage("team-create");
}

function showTeamCreate() {
  const ask = teamConnectAsk;
  teamConnectAsk = null;
  resetWizard();
  if (ask && teamsNow?.role === "owner") Object.assign(teamWizard, { step: 3, team: { ...teamsNow }, connect: true, origin: ask.origin });
  renderCreate();
}

/** Leaves Connect AI account for the page it was asked from. Nothing is created, renamed or invited. */
function leaveConnect() {
  const origin = teamWizard.origin;
  teamWizard.connect = false;
  if (origin) openPage(origin);
  else closePage();
}

// ---- Join a team --------------------------------------------------------------------------------

const teamJoining = {};
function resetJoining() {
  Object.assign(teamJoining, { phase: "code", code: "", busy: false, error: "", team: null, own: [], theirs: [], theirsError: "", on: {}, attached: new Set(), note: "" });
}
resetJoining();

function teamJoiningRows() {
  const have = new Set(teamJoining.theirs.map((p) => String(p.key).toLowerCase()));
  const own = teamJoining.own.map((p) => ({ ...p, match: !!p.key && have.has(p.key) }));
  return [...own.filter((p) => p.match), ...own.filter((p) => !p.match)];
}

function teamJoiningPickHtml() {
  const t = TEAMS_TEXT;
  const rows = teamJoiningRows();
  const matches = rows.filter((p) => p.match).length;
  const count = rows.filter((p) => p.key && teamJoining.on[p.key]).length;
  const intro = teamJoining.theirsError ? t.joinProjectsFailed(teamJoining.theirsError) : matches ? t.joinMatch(matches) : t.joinNoMatch;
  const list = rows
    .map((p) => {
      const on = !!p.key && !!teamJoining.on[p.key];
      return tmCheckRow(`data-tm="toggle" data-key="${teamsEsc(p.key || p.name)}"${p.key ? "" : " disabled"}`, on, p.name, p.match ? t.teamHas : p.key ? p.repo || p.key : t.projectNoRepo, on ? t.teamSees : t.onlyYou);
    })
    .join("");
  return `<div class="tm-hero"><div class="tm-faces">${teamsAvatar(teamJoining.team.name, "big")}</div><h1>${t.joinedTitle(teamsEsc(teamJoining.team.name))}</h1><p class="tm-lede big">${t.joinedAbout}</p></div>
    <section class="tm-section"><h2>${t.joinWhich}</h2><p class="tm-note">${teamsEsc(intro)}</p><div class="tm-list">${list}</div></section>
    <section class="tm-card"><h2>${t.seeTitle}</h2>${t.seeLines.map((l) => `<p>${l}</p>`).join("")}</section>
    ${tmError(teamJoining.error)}
    <div class="tm-actions">${tmButton('data-tm="join-with"', teamJoining.busy ? TEAMS_TEXT.working : t.joinWith(count), "primary", teamJoining.busy)}${tmButton('data-tm="join-without"', t.joinWithout, "quiet", teamJoining.busy)}</div>`;
}

function teamJoiningCodeHtml() {
  const t = TEAMS_TEXT;
  return `<div class="tm-hero"><h1>${t.joinTitle}</h1><p class="tm-lede big">${t.joinAbout}</p></div>
    <form class="tm-form flat" data-tm="join-form" autocomplete="off">
      <label class="tm-field big"><span>${t.codeLabel}</span><input id="tm-join-code" type="text" value="${teamsEsc(teamJoining.code)}" placeholder="${t.codePlaceholder}" autocomplete="off" spellcheck="false"></label>
      ${tmError(teamJoining.error)}
      <div class="tm-actions"><button type="submit" class="tm-btn primary"${teamJoining.busy ? " disabled" : ""}>${teamJoining.busy ? t.teamJoining : t.joinGo}</button>${tmButton('data-tm="leave"', t.cancel, "quiet")}</div>
    </form>`;
}

function renderJoin() {
  const focused = document.activeElement?.id === "tm-join-code";
  setPage(`${headHtml(TEAMS_TEXT.joinTitle)}<div class="page-body"><div class="tm-join-page">${teamJoining.phase === "code" ? teamJoiningCodeHtml() : teamJoiningPickHtml()}</div></div>`);
  if (focused) document.getElementById("tm-join-code")?.focus();
}

async function joinTeam() {
  const code = teamJoining.code.trim();
  if (teamJoining.busy) return;
  if (!code) {
    teamJoining.error = "Paste the invite code first.";
    return renderJoin();
  }
  teamJoining.busy = true;
  teamJoining.error = "";
  renderJoin();
  try {
    teamJoining.team = teamView(await teamCall({ op: "join", code }));
    teamJoining.own = teamsOwnProjects();
    teamJoining.theirs = [];
    teamJoining.theirsError = "";
    try {
      const theirs = await teamCall({ op: "projects" }, teamJoining.team.id);
      teamJoining.theirs = Array.isArray(theirs) ? theirs : [];
    } catch (e) {
      teamJoining.theirsError = teamsErrorText(e);
    }
    const have = new Set(teamJoining.theirs.map((p) => String(p.key).toLowerCase()));
    for (const p of teamJoining.own) if (p.key && have.has(p.key)) teamJoining.on[p.key] = true;
    teamJoining.phase = "pick";
    teamsList().catch(() => {});
  } catch (e) {
    teamJoining.error = teamsErrorText(e);
  }
  teamJoining.busy = false;
  renderJoin();
}

async function joinAttach(withProjects) {
  if (teamJoining.busy) return;
  teamJoining.busy = true;
  teamJoining.error = "";
  renderJoin();
  let failed = false;
  if (withProjects) {
    for (const p of teamJoining.own) {
      if (!p.key || !teamJoining.on[p.key] || teamJoining.attached.has(p.key)) continue;
      try {
        await teamCall({ op: "attach_project", key: p.key }, teamJoining.team.id);
        teamJoining.attached.add(p.key);
      } catch (e) {
        teamJoining.error = TEAMS_TEXT.attachFailed(p.name, teamsErrorText(e));
        failed = true;
        break;
      }
    }
  }
  teamJoining.busy = false;
  if (failed) return renderJoin();
  teamsSelect(teamJoining.team, { home: false });
  teamsOpenHome();
}

function showTeamJoin() {
  resetJoining();
  renderJoin();
}

// ---- Invites (owner only) -----------------------------------------------------------------------

/** `{op:"invites"}` lists the open invites, newest first: `{hint, created, made_by}`, never a code. */
const teamInvites = { team: "", list: [], loading: false, loadError: "", made: null, copied: false, busy: "", error: "", note: "" };
function resetInvites() {
  Object.assign(teamInvites, { team: teamsNow?.id ?? "", list: [], loading: false, loadError: "", made: null, copied: false, busy: "", error: "", note: "" });
}

function teamsWhen(unix) {
  const at = new Date(Number(unix) * 1000);
  if (!unix || Number.isNaN(at.getTime())) return "";
  return at.toLocaleString(typeof APP_LOCALE === "string" ? APP_LOCALE : undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

function teamInvitesHtml() {
  const t = TEAMS_TEXT;
  const head = `<div class="tm-hero"><h1>${t.invitesTitle}</h1><p class="tm-lede big">${t.invitesAbout}</p></div>`;
  if (teamsNow?.role !== "owner") return `${head}<p class="tm-note">${t.invitesOwnerOnly}</p>`;
  const made = teamInvites.made
    ? `<div class="tm-code" data-invite-made><code>${teamsEsc(teamInvites.made.code)}</code>${tmButton('data-tm="invite-copy"', teamInvites.copied ? t.inviteCopied : t.inviteCopy)}</div><p class="tm-note">${t.inviteSend} ${t.inviteShown}</p>`
    : "";
  const rows = teamInvites.list
    .map((inv) => {
      const when = teamsWhen(inv.created);
      const by = String(inv.made_by ?? "");
      const revoking = teamInvites.busy === `revoke:${inv.hint}`;
      const about = [when ? t.invitesMade(when) : "", by ? t.invitesBy(by) : ""].filter(Boolean).join(" ");
      return `<div class="tm-code tm-invite" data-hint="${teamsEsc(inv.hint)}"><span class="tm-texts"><code>${teamsEsc(inv.hint)}…</code>${about ? `<span class="tm-about">${teamsEsc(about)}</span>` : ""}</span>${tmButton(`data-tm="invite-revoke" data-hint="${teamsEsc(inv.hint)}"`, revoking ? t.inviteRevoking : t.inviteRevoke, "secondary", !!teamInvites.busy)}</div>`;
    })
    .join("");
  const open = teamInvites.list.length ? `<div class="tm-list">${rows}</div>` : teamInvites.loading ? `<p class="tm-note">${t.invitesLoading}</p>` : teamInvites.loadError ? "" : `<p class="tm-note">${t.invitesNone}</p>`;
  return `${head}
    <section class="tm-section">${made}${tmError(teamInvites.error)}<div class="tm-actions">${tmButton('data-tm="invite-make"', teamInvites.busy === "make" ? t.inviteMaking : t.inviteMake, "primary", !!teamInvites.busy)}</div></section>
    <section class="tm-section"><h2>${t.invitesOpen}</h2>${open}${tmError(teamInvites.loadError)}${teamInvites.note ? `<p class="tm-ok" role="status">${teamsEsc(teamInvites.note)}</p>` : ""}</section>`;
}

function renderInvites() {
  if (document.body.dataset.page !== "team-invites") return;
  setPage(`${headHtml(TEAMS_TEXT.invitesTitle)}<div class="page-body"><div class="tm-join-page">${teamInvitesHtml()}</div></div>`);
}

/** Reads the open invites again; an answer for a team the person has since left is dropped. */
async function loadTeamInvites() {
  const team = teamsNow?.id;
  if (!team || teamsNow.role !== "owner") return;
  teamInvites.loading = true;
  teamInvites.loadError = "";
  renderInvites();
  try {
    const list = await teamCall({ op: "invites" }, team);
    if (teamInvites.team !== team) return;
    teamInvites.list = (Array.isArray(list) ? list : []).filter((x) => x && typeof x.hint === "string" && x.hint);
  } catch (e) {
    if (teamInvites.team !== team) return;
    teamInvites.loadError = teamsErrorText(e);
  }
  teamInvites.loading = false;
  renderInvites();
}

async function makeInvite() {
  if (teamInvites.busy || teamsNow?.role !== "owner") return;
  const team = teamsNow.id;
  Object.assign(teamInvites, { busy: "make", error: "", note: "" });
  renderInvites();
  try {
    const made = await teamCall({ op: "invite" }, team);
    teamInvites.made = { code: String(made.code), hint: String(made.hint ?? "") };
    teamInvites.copied = false;
  } catch (e) {
    // The hub's own words, as sent: at 20 open invites it says so.
    teamInvites.error = teamsErrorText(e);
  }
  teamInvites.busy = "";
  await loadTeamInvites();
}

async function revokeInvite(hint) {
  const team = teamsNow?.id;
  if (teamInvites.busy || !team || teamsNow.role !== "owner") return;
  hint = String(hint ?? "").trim();
  if (!hint) return;
  Object.assign(teamInvites, { busy: `revoke:${hint}`, error: "", note: "" });
  renderInvites();
  try {
    await teamCall({ op: "revoke_invite", hint }, team);
    teamInvites.note = TEAMS_TEXT.inviteRevoked(hint);
    if (teamInvites.made?.hint === hint) teamInvites.made = null;
  } catch (e) {
    teamInvites.error = teamsErrorText(e);
  }
  teamInvites.busy = "";
  await loadTeamInvites();
}

function copyInvite() {
  const code = teamInvites.made?.code;
  if (!code) return;
  try {
    navigator.clipboard.writeText(code).then(() => {
      teamInvites.copied = true;
      renderInvites();
    }, () => {});
  } catch {}
}

function showTeamInvites() {
  resetInvites();
  renderInvites();
  loadTeamInvites();
}

// ---- The pages' events --------------------------------------------------------------------------

const teamsPageOpen = () => TEAM_PAGES.includes(document.body.dataset.page);
pageView.addEventListener("click", (e) => {
  const el = teamsPageOpen() && e.target.closest("[data-tm]");
  if (!el || el.disabled) return;
  const page = document.body.dataset.page;
  const act = el.dataset.tm;
  if (act === "leave") return page === "team-create" && teamWizard.connect ? leaveConnect() : closePage();
  if (act === "form" || act === "join-form") return;
  if (page === "team-invites") {
    if (act === "invite-make") makeInvite();
    else if (act === "invite-copy") copyInvite();
    else if (act === "invite-revoke") revokeInvite(el.dataset.hint);
    return;
  }
  if (page === "team-join") {
    if (act === "toggle") {
      const key = el.dataset.key;
      teamJoining.on[key] = !teamJoining.on[key];
      renderJoin();
    } else if (act === "join-with") joinAttach(true);
    else if (act === "join-without") joinAttach(false);
    return;
  }
  if (act === "go") {
    const n = Number(el.dataset.step);
    if (n < teamWizard.step && !teamWizard.busy) {
      teamWizard.step = n;
      teamWizard.error = "";
      renderCreate();
    }
  } else if (act === "prev") {
    teamWizard.step = Math.max(1, teamWizard.step - 1);
    teamWizard.error = "";
    renderCreate();
  } else if (act === "next") teamWizardNext();
  else if (act === "connect-done") leaveConnect();
  else if (act === "toggle") {
    teamWizard.on[el.dataset.key] = !teamWizard.on[el.dataset.key];
    renderCreate();
  } else if (act === "connect") {
    teamWizard.form = el.dataset.cred;
    teamWizard.formError = "";
    renderCreate();
    pageView.querySelector(".tm-form input")?.focus();
  } else if (act === "close-form") {
    teamWizard.form = null;
    teamWizard.formError = "";
    renderCreate();
  } else if (act === "invite") teamWizardInvite();
  else if (act === "copy") teamWizardCopy(Number(el.dataset.i));
  else if (act === "first-bot") teamWizardFinish(openTeamBotNew);
  else if (act === "open-team") teamWizardFinish(teamsOpenHome);
});
pageView.addEventListener("input", (e) => {
  if (!teamsPageOpen()) return;
  if (e.target.id === "tm-team-name") teamWizard.name = e.target.value;
  else if (e.target.id === "tm-join-code") teamJoining.code = e.target.value;
});
pageView.addEventListener("submit", (e) => {
  const form = teamsPageOpen() && e.target.closest("[data-tm]");
  if (!form) return;
  e.preventDefault();
  if (form.dataset.tm === "join-form") return joinTeam();
  if (form.dataset.tm !== "form") return;
  const values = {};
  for (const input of form.querySelectorAll("input[data-field]")) {
    values[input.dataset.field] = input.value;
    input.value = "";
  }
  teamWizardConnect(form.dataset.cred, values);
});
pageView.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && e.target.id === "tm-team-name" && teamsPageOpen()) {
    e.preventDefault();
    teamWizardNext();
  }
});

SHELL_PAGES["team-create"] = { show: showTeamCreate };
SHELL_PAGES["team-join"] = { show: showTeamJoin };
SHELL_PAGES["team-invites"] = { show: showTeamInvites };

drawTeamsButton();
drawTeamsWhere();
