/** T3 Code's prompt links: a URL on the web app, /new?prompt=…, that opens a new thread with the
 * prompt in its composer, in the project of `repo` when the link names one. */
const PROMPT_LINK_MAX = 8000;
// Zero-width and bidi controls, tag and variation-selector blocks, and other characters that
// render as nothing: a model reads them and the person checking the draft can't see them.
const HIDDEN_CHARACTERS = /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F\u00AD\u034F\u061C\u115F\u1160\u17B4\u17B5\u180B-\u180F\u200B\u200C\u200E\u200F\u202A-\u202E\u2060-\u206F\u3164\uFE00-\uFE0D\uFEFF\uFFA0\uFFF0-\uFFFB\u{1BCA0}-\u{1BCA3}\u{1D173}-\u{1D17A}\u{E0000}-\u{E0FFF}]/gu;
const isEmojiPart = (cp) => cp !== undefined && ((cp >= 0x1f000 && cp <= 0x1faff) || (cp >= 0x2190 && cp <= 0x2bff) || cp === 0xfe0f);

function sanitizeLinkedPrompt(raw) {
  let hidden = 0;
  const visible = Array.from(raw.replace(/\r\n?|[\u2028\u2029]/g, "\n").replace(HIDDEN_CHARACTERS, () => (hidden++, "")));
  const text = visible.filter((c, i) => {
    if (c !== "\u200D") return true;
    const kept = isEmojiPart(visible[i - 1]?.codePointAt(0)) && isEmojiPart(visible[i + 1]?.codePointAt(0));
    if (!kept) hidden++;
    return kept;
  }).join("");
  return { text: text.trim(), hidden };
}

function linkRepository(value) {
  const m = /^https?:\/\/(?:www\.)?github\.com\/([^/?#]+)\/([^/?#]+)\/?$/i.exec(value.trim()) ?? /^([^/\s]+)\/([^/\s]+)$/.exec(value.trim());
  if (!m) return null;
  const name = m[2].replace(/\.git$/i, "");
  return /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})$/.test(m[1]) && /^[A-Za-z0-9._-]{1,100}$/.test(name) && !/^\.+$/.test(name) ? { owner: m[1], name } : null;
}
function linkPullRequest(value) {
  const m = /^https?:\/\/(?:www\.)?github\.com\/([^/?#]+)\/([^/?#]+)\/pull\/(\d+)(?:[/?#].*)?$/i.exec(value.trim()) ?? /^([^/\s#]+)\/([^/\s#]+)(?:\/|#)(\d+)$/.exec(value.trim());
  const repo = m && linkRepository(`${m[1]}/${m[2]}`);
  const number = m && Number(m[3]);
  return repo && Number.isSafeInteger(number) && number > 0 ? { ...repo, number } : null;
}

/** `{ ok, value | error }`, as T3 Code's parseNewThreadLink. */
function parseNewThreadLink(search) {
  const params = new URLSearchParams(search);
  const raw = params.get("prompt");
  let prompt = null;
  if (raw !== null) {
    if (raw.length > PROMPT_LINK_MAX) return { ok: false, error: `This prompt is longer than ${PROMPT_LINK_MAX.toLocaleString("en-US")} characters, so Pinned did not open it.` };
    prompt = sanitizeLinkedPrompt(raw);
    if (!prompt.text && prompt.hidden) return { ok: false, error: "This prompt had only hidden characters, so it may be malicious. Pinned did not open it." };
    if (!prompt.text) prompt = null;
  }
  if (params.has("pr")) {
    const pullRequest = linkPullRequest(params.get("pr"));
    return pullRequest ? { ok: true, value: { kind: "pull-request", pullRequest, prompt } } : { ok: false, error: "This link names a pull request Pinned cannot read." };
  }
  if (!prompt) return { ok: false, error: "This link has no prompt." };
  const repo = params.get("repo");
  const repository = repo === null ? null : linkRepository(repo);
  if (repo !== null && !repository) return { ok: false, error: "This link names a repository Pinned cannot read." };
  const optional = (key) => params.get(key)?.trim() || null;
  return { ok: true, value: { kind: "new", prompt, repository, provider: optional("provider")?.toLowerCase() ?? null, model: optional("model") } };
}

/** The web app this link opens on: this page's, or the signed-in hub's in the desktop app. */
const promptLinkOrigin = () => (TAURI ? account?.hub ?? null : location.origin);
function promptLinkFor(prompt, context = {}) {
  const origin = promptLinkOrigin();
  const text = prompt.trim();
  if (!origin || !text || text.length > PROMPT_LINK_MAX) return null;
  const url = new URL("/new", origin);
  url.searchParams.set("prompt", text);
  if (context.repository) url.searchParams.set("repo", context.repository);
  if (context.provider) url.searchParams.set("provider", context.provider);
  if (context.provider && context.model) url.searchParams.set("model", context.model);
  return url.toString();
}
const githubOf = (repo) => /^github\.com\/([^/]+\/[^/]+)$/i.exec(repo ?? "")?.[1] ?? null;

/** The composer's model a link names, by its family as the menu shows it or by its slug. */
const linkedModel = (name) => CLAUDE_MODELS.find((m) => (!m.legacy && m.menu.toLowerCase() === name.toLowerCase()) || m.slug === name) ?? null;

function promptLinkItems(tab) {
  const draft = tab && drafts[threadKey(tab)];
  const plain = draft && promptLinkFor(draft);
  if (!plain) return [];
  const repository = githubOf(paletteProjects().find((p) => p.machine === tab.machine && p.path === projectOfTab(tab))?.repo);
  const family = modelName(tab).split(" ")[0].toLowerCase();
  const withContext = repository && promptLinkFor(draft, { repository, provider: "claude", model: linkedModel(family) ? family : null });
  return [
    { icon: "link", title: "Copy prompt link", terms: ["copy prompt link", "share"], run: () => copyAndTell(plain, "Prompt link") },
    withContext && { icon: "link", title: "Copy prompt link with repo and model", terms: ["copy prompt link with repo and model", "share repository"], run: () => copyAndTell(withContext, "Prompt link") },
  ].filter(Boolean);
}

/** The link this page opened with, kept across a sign-in, and what became of it. */
const linkLanding = { link: null, state: null };
try {
  if (!TAURI && location.pathname === "/new") {
    sessionStorage.setItem("pinned-new-link", location.search);
    history.replaceState(null, "", "/");
  }
  const pending = sessionStorage.getItem("pinned-new-link");
  if (pending !== null) linkLanding.link = parseNewThreadLink(pending);
} catch {}

const OPENED = { title: "This link is open", buttons: [["again", "Open it again", true]] };
function renderLinkPage() {
  const { state } = linkLanding;
  const buttons = (state.buttons ?? []).map(([act, label, outline]) => `<button type="button" class="${outline ? "outline" : "primary"}" data-link="${act}">${esc(label)}</button>`).join("");
  setPage(`${headHtml("New thread")}<div class="page-body"><div class="page-wrap link-page"><div class="pr-empty">
    <h2>${esc(state.title)}</h2>${state.message ? `<p>${esc(state.message)}</p>` : ""}${buttons ? `<span class="buttons">${buttons}</span>` : ""}</div></div></div>`);
}
SHELL_PAGES.link = { show: renderLinkPage };
function showLinkState(state) {
  linkLanding.state = state;
  if (shellPage() === "link") renderLinkPage();
  else openPage("link");
}

/** Puts the linked prompt in the thread's composer, asking first when it holds another draft. */
function placeLinkedPrompt(tab, prompt, notes = []) {
  const place = () => {
    saveDraft(tab, prompt.text);
    if (tab === active) switchDraft(tab);
    tell(esc(["Prompt added from a link. Read it before you send.", ...notes].join(" ")), 6000);
  };
  if (prompt.hidden) showComposerNote(`This prompt had hidden characters, it may be malicious. Pinned removed ${prompt.hidden} hidden character${prompt.hidden === 1 ? "" : "s"}. Read the prompt before you send it.`, true);
  const existing = drafts[threadKey(tab)] ?? "";
  if (!existing.trim() || existing === prompt.text) return place();
  tell(`Linked prompt ready. This thread already has a draft. <button type="button" data-replace-draft>Replace draft</button>`, 15000);
  sideNotice.querySelector("[data-replace-draft]").onclick = place;
}

async function untilClaudeStarts(tab) {
  for (let i = 0; i < 100 && !tabByKey(tab.key)?.view.session; i++) await new Promise((r) => setTimeout(r, 200));
  return !!tabByKey(tab.key)?.view.session;
}
/** The link's model, picked the way the composer's model menu picks one once claude is up to read
 * it; returns at once with what to tell about the link's provider and model. */
function pickLinkedModel(tab, link) {
  if (!link.provider) return [];
  if (link.provider !== "claude" && link.provider !== "claudeagent") return [`The link asks for ${link.provider}; this thread runs Claude Code.`];
  if (!link.model) return [];
  const pick = linkedModel(link.model);
  if (!pick) return [`The model ${link.model} isn't one Claude Code offers here, so your default stands in.`];
  untilClaudeStarts(tab).then((up) => up && pickModel(tab, pick));
  return [];
}

async function projectsWithRepo(repo) {
  const want = `github.com/${repo.owner}/${repo.name}`.toLowerCase();
  const lists = await Promise.all(onlineMachines().map((m) => callOn(m, { cmd: "list_projects" }).then((list) => list.map((p) => ({ ...p, machine: m })), () => [])));
  return lists.flat().filter((p) => p.repo?.toLowerCase() === want);
}
async function openThreadWith(machine, path, link) {
  await (machine === LOCAL || path === null ? openTab(machine, path) : openRemote(machine, path, false));
  const tab = active;
  linkLanding.state = OPENED;
  if (link.prompt) placeLinkedPrompt(tab, link.prompt, link.kind === "new" ? pickLinkedModel(tab, link) : []);
  return tab;
}

async function openPromptLink() {
  const parsed = linkLanding.link;
  try {
    sessionStorage.removeItem("pinned-new-link");
  } catch {}
  if (!parsed.ok) return showLinkState({ title: "Could not open the link", message: parsed.error });
  const link = parsed.value;
  showLinkState({ title: "Opening the link..." });
  try {
    if (link.kind === "pull-request") {
      const { owner, name, number } = link.pullRequest;
      const url = `https://github.com/${owner}/${name}/pull/${number}`;
      const linked = [...tabs].filter((t) => linkedPrs(t).some((pr) => pr.url === url)).sort((a, b) => latestUserAt(b) - latestUserAt(a))[0];
      if (linked) {
        linkLanding.state = OPENED;
        activate(linked);
        if (link.prompt) placeLinkedPrompt(linked, link.prompt);
        return;
      }
      const [project] = await projectsWithRepo(link.pullRequest);
      if (!project) return showLinkState({ title: `You don't have ${owner}/${name}`, message: "Clone it to work on it here.", buttons: [["clone", `Clone ${owner}/${name}`]], repo: link.pullRequest });
      const tab = await openThreadWith(project.machine, project.path, link);
      linkPr(tab, url);
      return openPr(tab, url);
    }
    if (!link.repository) return void (await openThreadWith(active?.machine ?? onlineMachines()[0], null, link));
    const [project] = await projectsWithRepo(link.repository);
    if (project) return void (await openThreadWith(project.machine, project.path, link));
    const { owner, name } = link.repository;
    showLinkState({ title: `You don't have ${owner}/${name}`, message: "Clone it to work on it here, or start without a project.", buttons: [["clone", `Clone ${owner}/${name}`], ["scratch", "Use without project", true]], repo: link.repository });
  } catch (e) {
    showLinkState({ title: "Could not open the link", message: String(e) });
  }
}

pageView.addEventListener("click", async (e) => {
  const act = shellPage() === "link" && e.target.closest("[data-link]")?.dataset.link;
  if (!act) return;
  const link = linkLanding.link.value;
  const machine = active?.machine ?? onlineMachines()[0];
  if (act === "again") return openPromptLink();
  if (act === "scratch") return openThreadWith(machine, null, link);
  const { owner, name } = linkLanding.state.repo;
  showLinkState({ title: `Cloning ${owner}/${name}`, message: `On ${machineName(machine)}.` });
  try {
    const path = hub ? await hub.hub({ cmd: "clone", machine: hubIdOf(machine), repo: `${owner}/${name}` }) : await invoke("clone_repo", { repo: `${owner}/${name}` });
    const tab = await openThreadWith(machine, path, link);
    if (link.kind === "pull-request") {
      const url = `https://github.com/${owner}/${name}/pull/${link.pullRequest.number}`;
      linkPr(tab, url);
      openPr(tab, url);
    }
  } catch (err) {
    showLinkState({ title: "Clone failed", message: String(err), buttons: [["clone", `Clone ${owner}/${name}`]], repo: linkLanding.state.repo });
  }
});

/** Opens the link once a machine is online to take it, which on the web waits for the sign-in. */
if (linkLanding.link) {
  const wait = setInterval(() => {
    if (!onlineMachines().length) return;
    clearInterval(wait);
    openPromptLink();
  }, 300);
}
