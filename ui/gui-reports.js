/** Report a problem or an idea to pinned from the app: the dialog (Account and pages menu), the
 * agent's request to report in the Needs you and Agent activity cards, and the My reports page.
 * The hub files and tracks the issues (`reports` and `report_issue`); this file is the screens.
 *
 * Text that came from GitHub (titles, states, close reasons) is untrusted. It is drawn with
 * textContent only: the HTML below holds an empty `data-gh` slot for it, and `paintSlots` fills the slot. */

const REPORT_TEXT = {
  menuReport: "Report a problem",
  menuMine: "My reports",
  dialogTitle: "Report to pinned",
  kinds: { problem: "Problem", idea: "Idea" },
  kindLabel: "Kind",
  titleLabel: "Title",
  fields: {
    problem: { happened: "What happened", expected: "What you expected", repeat: "How to repeat it" },
    idea: { happened: "The idea", expected: "Why it helps" },
  },
  optional: "optional",
  similarTitle: "Already reported",
  similarComments: (n) => `${n} ${n === 1 ? "comment" : "comments"}`,
  similarPick: "This is it: add my case",
  similarPicked: "Your case goes to this report",
  similarUndo: "Not this one",
  withIt: "What goes with it",
  environment: { name: "Version and system", detail: (version, system) => [version && `pinned ${version}`, system].filter(Boolean).join(", ") },
  errors: {
    name: (n) => `Last ${n} error ${n === 1 ? "line" : "lines"} from the app`,
    detail: "folder paths are removed; you see them before sending",
    none: "No errors recorded in this window",
  },
  publicNote: "This becomes a public issue on pinned's GitHub. Your conversations, code, folder paths and keys don't go with it. You don't need a GitHub account.",
  review: "Review and send",
  reviewFollow: (n) => `Review and add to #${n}`,
  checking: "Checking…",
  working: "Working…",
  saveDraft: "Save draft",
  cancel: "Cancel",
  unavailable: "Reports are not available on this hub.",
  signedOut: "Sign in under Account to send reports.",
  close: "Close",
  previewTitle: "Review before sending",
  previewAbout: "This is exactly what will be public on GitHub.",
  previewErrors: "Error lines that will be sent",
  publicTag: "Public on GitHub",
  leftToday: (n) => `${n} ${n === 1 ? "report" : "reports"} left today.`,
  send: "Send to pinned",
  sendFollow: (n) => `Add my case to #${n}`,
  sending: "Sending…",
  edit: "Edit",
  done: {
    sent: { title: (n) => `Report #${n} opened`, text: "It is on GitHub now. When someone answers or the fix ships in a version, you are told here in the app." },
    followed: { title: (n) => `Your case was added to #${n}`, text: "No duplicate issue was opened. You follow that one and are told when it moves." },
    draft: { title: () => "Draft saved", text: "Nothing was sent. The draft stays in My reports until you send or delete it." },
  },
  openOnGitHub: "Open on GitHub",
  seeMine: "See my reports",
  another: "Report another",
  pageTitle: "My reports",
  pageAbout: "What you reported to pinned and where each one stands.",
  groups: { news: "News", open: "Open", drafts: "Drafts, not sent yet", closed: "Closed" },
  noDrafts: "No drafts.",
  nothing: "You have not reported anything yet.",
  loading: "Loading…",
  loadFailed: (e) => `Could not load your reports: ${e}`,
  pageNote: "Sent reports are public issues on pinned's GitHub. What shows there is the text of the report.",
  updatePinned: "Update pinned",
  openDraft: "Open and send",
  deleteDraft: "Delete",
  untitled: "Untitled draft",
  tagIdea: "Idea",
  state: {
    sending: "Sending…",
    failed: (e) => `Could not send${e ? `: ${e}` : "."}`,
    answered: (ago) => `The pinned team replied ${ago}.`,
    fixedNew: (v, app) => `Fixed in ${v}.${app ? ` You are on ${app}.` : ""}`,
    fixed: (v) => `Fixed in ${v}.`,
    closed: (reason) => (reason ? `Closed: ${reason}` : "Closed."),
    open: (ago, byAgent) => `opened ${ago}${byAgent ? ", by the agent" : ""}`,
    followed: "someone else's report, you follow it",
  },
  agent: {
    ask: "Review the report before it is sent. It becomes a public issue on GitHub.",
    publicTag: "Public on GitHub",
    send: "Send to pinned",
    editFirst: "Edit first",
    skip: "Don't send",
    similar: (n) => `#${n} looks like the same problem.`,
    followSimilar: (n) => `Add my case to #${n}`,
    loading: "Preparing the preview…",
  },
};

/* ---- pure helpers (no DOM) ---- */

const REPORT_ERROR_LINES = 20;
const REPORT_SIMILAR_SHOWN = 3;
const REPORT_TITLE_MIN = 6;

const reportBlank = (kind = "problem") => ({ kind, title: "", happened: "", expected: "", repeat: "", attach: { environment: true, errors: [] }, to_issue: null });

/** A draft as the wire has it, whatever an agent or an older save left out. */
function reportNormalize(raw) {
  const d = raw && typeof raw === "object" ? raw : {};
  const text = (v) => (typeof v === "string" ? v : "");
  return {
    kind: d.kind === "idea" ? "idea" : "problem",
    title: text(d.title),
    happened: text(d.happened),
    expected: text(d.expected),
    repeat: text(d.repeat),
    attach: { environment: d.attach?.environment !== false, errors: Array.isArray(d.attach?.errors) ? d.attach.errors.filter((l) => typeof l === "string") : [] },
    to_issue: Number.isInteger(d.to_issue) ? d.to_issue : null,
  };
}

/** The draft the dialog sends: the error lines only while their box is ticked. */
function reportDraftOf(dlg) {
  const d = dlg.draft;
  return { kind: d.kind, title: d.title.trim(), happened: d.happened.trim(), expected: d.expected.trim(), repeat: d.kind === "idea" ? "" : d.repeat.trim(), attach: { environment: d.attach.environment, errors: dlg.errorsOn ? dlg.errorLines : [] }, to_issue: d.to_issue };
}
const reportReady = (dlg) => dlg.draft.title.trim().length > 0 && dlg.draft.happened.trim().length > 0;

const reportAsk = (ask) => ({ cmd: "reports", ask });
const reportPreviewFrame = (dlg, context) => reportAsk({ op: "preview", draft: reportDraftOf(dlg), context });
const reportSendFrame = (dlg) => ({ cmd: "report_issue", draft: reportDraftOf(dlg), shown: dlg.preview.shown, draft_id: dlg.draftId ?? null, request_id: dlg.requestId });
const reportSaveFrame = (dlg) => reportAsk({ op: "save_draft", id: dlg.draftId ?? null, draft: reportDraftOf(dlg) });
const reportSimilarFrame = (title) => reportAsk({ op: "similar", title });

/** What the hub's scrubber needs to take out of the text: this app's project and folder names. */
function reportContext() {
  const out = { projects: [], folders: [], files: [], names: [] };
  try {
    for (const tab of typeof tabs === "undefined" ? [] : tabs) {
      const folder = (typeof threadOf === "function" ? threadOf(tab).workspace?.project : null) ?? tab.view?.cwd;
      if (typeof folder === "string" && folder) out.folders.push(folder);
      if (typeof projectName === "function") out.projects.push(projectName(tab));
    }
    if (typeof machines !== "undefined") for (const m of machines.values()) if (m.view?.name) out.names.push(m.view.name);
  } catch {}
  for (const k of Object.keys(out)) out[k] = [...new Set(out[k].filter((v) => typeof v === "string" && v))];
  return out;
}

const reportSystem = () => {
  const ua = (typeof navigator !== "undefined" && navigator.userAgent) || "";
  return /Windows/.test(ua) ? "Windows" : /iPhone|iPad/.test(ua) ? "iOS" : /Android/.test(ua) ? "Android" : /Mac/.test(ua) ? "macOS" : /Linux/.test(ua) ? "Linux" : "";
};
const reportAppVersion = () => String((typeof document !== "undefined" && document.body?.dataset.version) || "").replace(/^v/, "");

/** "0.18.5" as [0, 18, 5], or null. */
const reportVersion = (text) => {
  const m = /(\d+)\.(\d+)\.(\d+)/.exec(String(text ?? ""));
  return m ? m.slice(1).map(Number) : null;
};
function reportNewer(a, b) {
  const x = reportVersion(a);
  const y = reportVersion(b);
  if (!x || !y) return false;
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i];
  return false;
}

/** A hub timestamp (unix seconds, milliseconds or ISO text) in milliseconds, or null. */
function reportTime(v) {
  if (typeof v === "number") return v < 1e12 ? v * 1000 : v;
  const t = Date.parse(v);
  return Number.isNaN(t) ? null : t;
}
function reportAgo(ms, now = Date.now()) {
  if (ms == null) return "";
  const days = Math.floor((now - ms) / 86400e3);
  return days < 1 ? "today" : days === 1 ? "yesterday" : `${days} days ago`;
}

/** The line under a report on My reports, with its tone, and what the row offers. */
function reportStateLine(r, appVersion, now = Date.now()) {
  const st = r.state ?? {};
  const T = REPORT_TEXT.state;
  switch (st.state) {
    case "sending":
      return { text: T.sending, tone: "muted" };
    case "failed":
      return { text: T.failed(st.error), tone: "error" };
    case "answered":
      return { text: T.answered(reportAgo(reportTime(st.at), now)), tone: "warn" };
    case "fixed":
      return reportNewer(st.version, appVersion) ? { text: T.fixedNew(st.version, appVersion), tone: "good", update: true } : { text: T.fixed(st.version), tone: "good" };
    case "closed":
      return { text: T.closed(st.reason), tone: "muted" };
    default: {
      const ago = reportAgo(reportTime(r.created_at), now);
      return { text: r.followed && !r.by_agent ? T.followed : T.open(ago, r.by_agent), tone: "muted" };
    }
  }
}

/** My reports in its groups, newest first. */
function reportGroups(list, appVersion, now = Date.now()) {
  const groups = { news: [], open: [], drafts: [], closed: [] };
  for (const report of list) {
    const st = report.state?.state;
    const line = reportStateLine(report, appVersion, now);
    const row = { report, line };
    if (st === "draft" || (!report.issue && !report.url && report.draft && st !== "sending" && st !== "failed")) groups.drafts.push(row);
    else if (st === "answered" || (st === "fixed" && line.update)) groups.news.push(row);
    else if (st === "fixed" || st === "closed") groups.closed.push(row);
    else groups.open.push(row);
  }
  const at = (row) => reportTime(row.report.updated_at) ?? 0;
  for (const rows of Object.values(groups)) rows.sort((a, b) => at(b) - at(a));
  return groups;
}

const reportRowTitle = (r) => r.title || r.draft?.title || REPORT_TEXT.untitled;

/** The issue's markdown as sections: `{ heading, text, code }`. Only headings and fences are read; nothing is rendered as markdown. */
function parseIssueBody(body) {
  const sections = [];
  let at = { heading: "", text: [], code: [] };
  let fenced = false;
  const flush = () => {
    if (at.heading || at.text.length || at.code.length) sections.push({ heading: at.heading, text: at.text.join("\n").trim(), code: at.code });
  };
  for (const line of String(body ?? "").split("\n")) {
    if (/^\s*```/.test(line)) {
      fenced = !fenced;
      continue;
    }
    const head = !fenced && /^#{1,6}\s+(.*)$/.exec(line);
    if (head) {
      flush();
      at = { heading: head[1].trim(), text: [], code: [] };
    } else if (fenced) at.code.push(line);
    else at.text.push(line);
  }
  flush();
  return sections.filter((s) => s.heading || s.text || s.code.length);
}

/** A GitHub link the app opens, or null for anything else. */
function reportUrl(url) {
  try {
    const u = new URL(url);
    return u.protocol === "https:" && u.hostname === "github.com" ? u.href : null;
  } catch {
    return null;
  }
}

/* ---- text from GitHub or the hub: slots, never HTML ---- */

function reportSlots() {
  const list = [];
  const slot = (value, tag = "span", cls = "") => {
    list.push(String(value ?? ""));
    return `<${tag}${cls ? ` class="${cls}"` : ""} data-gh="${list.length - 1}"></${tag}>`;
  };
  return { list, slot };
}
function paintSlots(root, s) {
  for (const el of root.querySelectorAll("[data-gh]")) el.textContent = s.list[Number(el.dataset.gh)] ?? "";
}
const reportEsc = (v) => (typeof esc === "function" ? esc(v) : String(v ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`));
const reportIcon = (name) => (typeof icon === "function" ? icon(name) : "");

function reportOpenUrl(url) {
  const safe = reportUrl(url);
  if (!safe) return;
  if (typeof TAURI !== "undefined" && TAURI && typeof invoke === "function") invoke("open_url", { url: safe });
  else window.open(safe, "_blank", "noopener");
}
/** The "Open on GitHub" control for a report's own link; nothing when the link is not GitHub's. */
const reportLinkHtml = (url, label = REPORT_TEXT.openOnGitHub, cls = "rp-link") => (reportUrl(url) ? `<button type="button" class="${cls}" data-rp="github" data-url="${reportEsc(reportUrl(url))}">${reportEsc(label)}${reportIcon("external-link")}</button>` : "");

/* ---- the hub ---- */

const reportHub = () => (typeof hub !== "undefined" && hub ? hub : null);
const reportCall = (call) => (reportHub() ? reportHub().hub(call) : Promise.reject(REPORT_TEXT.signedOut));
const reportUuid = () => (globalThis.crypto?.randomUUID ? crypto.randomUUID() : `r-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`);

const reportsUi = { dialog: null, list: null, listState: "idle", listError: "", card: new Map() };

/* ---- the dialog ---- */

function reportDialogState(opts = {}) {
  const draft = reportNormalize(opts.draft ?? reportBlank());
  const errorsOn = draft.attach.errors.length > 0;
  return {
    step: "form",
    status: "loading",
    draft,
    draftId: opts.draftId ?? null,
    errorsOn,
    errorLines: draft.attach.errors.slice(),
    picked: draft.to_issue && opts.picked ? opts.picked : draft.to_issue ? { number: draft.to_issue, title: "" } : null,
    similar: [],
    similarFor: "",
    seq: 0,
    timer: null,
    preview: null,
    requestId: null,
    busy: false,
    error: "",
    result: null,
    gh: null,
  };
}

const reportErrorCount = () => (typeof reportErrors === "undefined" ? 0 : reportErrors.count());
const reportRingLines = () => (typeof reportErrors === "undefined" ? [] : reportErrors.last(REPORT_ERROR_LINES));

function reportFormHtml(dlg) {
  const t = REPORT_TEXT;
  const d = dlg.draft;
  const labels = t.fields[d.kind];
  const e = reportEsc;
  const kinds = Object.entries(t.kinds).map(([k, label]) => `<button type="button" aria-pressed="${d.kind === k}" data-rp="kind" data-kind="${k}">${e(label)}</button>`).join("");
  const ringCount = Math.min(reportErrorCount(), REPORT_ERROR_LINES);
  const errorsLabel = dlg.errorsOn ? dlg.errorLines.length : ringCount;
  const box = (id, on, name, detail, disabled = false) => `<button type="button" class="rp-extra" role="checkbox" aria-checked="${on}" data-rp="attach" data-attach="${id}"${disabled ? " disabled" : ""}><span class="rp-box" aria-hidden="true">${on ? reportIcon("check") : ""}</span><span class="rp-extra-name">${e(name)}</span><span class="rp-extra-detail">${e(detail)}</span></button>`;
  const first = d.to_issue ? t.reviewFollow(d.to_issue) : t.review;
  return `<div class="rp-head"><h2>${e(t.dialogTitle)}</h2><div class="rp-seg" role="group" aria-label="${e(t.kindLabel)}">${kinds}</div></div>
  <div class="rp-body">
    <div class="rp-field"><label for="rp-title">${e(t.titleLabel)}</label><input id="rp-title" name="title" type="text" maxlength="200" autocomplete="off" value="${e(d.title)}"></div>
    <div class="rp-field"><label for="rp-happened">${e(labels.happened)}</label><textarea id="rp-happened" name="happened" rows="3">${e(d.happened)}</textarea></div>
    <div class="rp-field"><label for="rp-expected">${e(labels.expected)}</label><textarea id="rp-expected" name="expected" rows="2">${e(d.expected)}</textarea></div>
    ${d.kind === "problem" ? `<div class="rp-field"><label for="rp-repeat">${e(labels.repeat)} <span class="muted">(${e(t.optional)})</span></label><textarea id="rp-repeat" name="repeat" rows="2">${e(d.repeat)}</textarea></div>` : ""}
    <div data-slot="similar"></div>
    <div class="rp-extras"><span class="rp-label">${e(t.withIt)}</span>
      ${box("environment", d.attach.environment, t.environment.name, t.environment.detail(reportAppVersion(), reportSystem()))}
      ${box("errors", dlg.errorsOn, ringCount || dlg.errorsOn ? t.errors.name(errorsLabel) : t.errors.name(REPORT_ERROR_LINES), ringCount || dlg.errorsOn ? t.errors.detail : t.errors.none, !ringCount && !dlg.errorsOn)}
    </div>
    <p class="rp-note">${e(t.publicNote)}</p>
    <div class="rp-actions"><button type="button" class="primary" data-rp="review"${reportReady(dlg) && !dlg.busy ? "" : " disabled"}>${e(dlg.busy ? t.working : first)}</button><button type="button" class="outline" data-rp="save"${dlg.busy ? " disabled" : ""}>${e(t.saveDraft)}</button><button type="button" class="rp-quiet" data-rp="cancel">${e(t.cancel)}</button></div>
    <p class="rp-error" role="alert" data-slot="error"></p>
  </div>`;
}

/** The similar-reports block. `s` carries the GitHub text as slots. */
function reportSimilarHtml(dlg, s) {
  const t = REPORT_TEXT;
  if (dlg.picked) {
    return `<div class="rp-similar picked"><span class="rp-similar-text"><span class="rp-hint">${reportEsc(t.similarPicked)}</span><span class="rp-similar-title">#${dlg.picked.number}${dlg.picked.title ? " · " : ""}${dlg.picked.title ? s.slot(dlg.picked.title) : ""}</span></span><button type="button" class="outline" data-rp="unpick">${reportEsc(t.similarUndo)}</button></div>`;
  }
  return dlg.similar
    .slice(0, REPORT_SIMILAR_SHOWN)
    .map((x, i) => `<div class="rp-similar"><span class="rp-similar-text"><span class="rp-hint">${reportEsc(t.similarTitle)} · ${reportEsc(t.similarComments(x.comments ?? 0))}</span><span class="rp-similar-title">#${Number(x.number)} · ${s.slot(x.title)}</span></span><button type="button" class="outline" data-rp="pick" data-index="${i}">${reportEsc(t.similarPick)}</button></div>`)
    .join("");
}

function reportPreviewHtml(dlg, s) {
  const t = REPORT_TEXT;
  const p = dlg.preview;
  const e = reportEsc;
  const sections = parseIssueBody(p.body)
    .map((x) => `<div class="rp-section">${x.heading ? `<div class="rp-section-head">${s.slot(x.heading)}</div>` : ""}${x.text ? s.slot(x.text, "div", "rp-section-text") : ""}${x.code.length ? s.slot(x.code.join("\n"), "pre", "rp-lines") : ""}</div>`)
    .join("");
  const labels = (p.labels ?? []).map((l) => s.slot(l, "span", "rp-chip")).join("");
  const send = dlg.draft.to_issue ? t.sendFollow(dlg.draft.to_issue) : t.send;
  const left = Number.isFinite(p.left_today) ? `<span class="rp-hint">${e(t.leftToday(p.left_today))}</span>` : "";
  return `<div class="rp-head"><h2>${e(t.previewTitle)}</h2></div>
  <div class="rp-body">
    <p class="rp-note">${e(t.previewAbout)}</p>
    <div class="rp-issue">
      <div class="rp-issue-head"><span class="rp-chip public">${e(t.publicTag)}</span>${labels}</div>
      <div class="rp-issue-title">${s.slot(p.title)}</div>
      ${sections}
    </div>
    <div data-slot="similar">${dlg.draft.to_issue ? "" : reportSimilarHtml({ ...dlg, similar: p.similar ?? [], picked: null }, s)}</div>
    ${dlg.draft.to_issue ? `<p class="rp-note">${e(t.similarPicked)}: #${dlg.draft.to_issue}</p>` : ""}
    <div class="rp-actions"><button type="button" class="primary" data-rp="send"${dlg.busy ? " disabled" : ""}>${e(dlg.busy ? t.sending : send)}</button><button type="button" class="outline" data-rp="back"${dlg.busy ? " disabled" : ""}>${e(t.edit)}</button>${left}</div>
    <p class="rp-error" role="alert" data-slot="error"></p>
  </div>`;
}

function reportDoneHtml(dlg, s) {
  const t = REPORT_TEXT;
  const r = dlg.result;
  const copy = t.done[r.kind];
  return `<div class="rp-head"><h2>${reportEsc(copy.title(r.number))}</h2></div>
  <div class="rp-body"><p class="rp-text">${reportEsc(copy.text)}</p>
    <div class="rp-actions">${r.kind !== "draft" ? reportLinkHtml(r.url, t.openOnGitHub, "rp-link primary-link") : ""}<button type="button" class="${r.kind === "draft" ? "primary" : "outline"}" data-rp="mine">${reportEsc(t.seeMine)}</button><button type="button" class="rp-quiet" data-rp="another">${reportEsc(t.another)}</button></div>
  </div>`;
}

function reportPlainHtml(text, withClose = true) {
  return `<div class="rp-head"><h2>${reportEsc(REPORT_TEXT.dialogTitle)}</h2></div>
  <div class="rp-body"><p class="rp-text" role="status">${reportEsc(text)}</p>${withClose ? `<div class="rp-actions"><button type="button" class="outline" data-rp="cancel">${reportEsc(REPORT_TEXT.close)}</button></div>` : ""}</div>`;
}

function reportDialogHtml(dlg) {
  if (!reportHub()) return reportPlainHtml(REPORT_TEXT.signedOut);
  if (dlg.status === "loading") return reportPlainHtml(REPORT_TEXT.checking, false);
  if (dlg.status === "unavailable") return reportPlainHtml(REPORT_TEXT.unavailable);
  const s = reportSlots();
  const html = dlg.step === "preview" ? reportPreviewHtml(dlg, s) : dlg.step === "done" ? reportDoneHtml(dlg, s) : reportFormHtml(dlg);
  dlg.gh = s;
  return html;
}

const reportDialogEl = () => (typeof shellDialog !== "undefined" && shellDialog.dataset.kind === "report" ? shellDialog : null);

function drawReportDialog() {
  const dlg = reportsUi.dialog;
  const root = reportDialogEl();
  if (!dlg || !root) return;
  const popup = root.querySelector(".popup");
  const keep = popup.querySelector(".close");
  const page = document.createElement("div");
  page.className = "rp-dialog";
  page.innerHTML = reportDialogHtml(dlg);
  for (const old of popup.querySelectorAll(".rp-dialog")) old.remove();
  popup.insertBefore(page, keep);
  if (dlg.gh) paintSlots(page, dlg.gh);
  drawReportSimilar();
  drawReportError();
}

function drawReportSimilar() {
  const dlg = reportsUi.dialog;
  const root = reportDialogEl();
  const slot = root?.querySelector('[data-slot="similar"]');
  if (!dlg || !slot || dlg.step !== "form") return;
  const s = reportSlots();
  slot.innerHTML = reportSimilarHtml(dlg, s);
  paintSlots(slot, s);
}

function drawReportError() {
  const dlg = reportsUi.dialog;
  const el = reportDialogEl()?.querySelector('[data-slot="error"]');
  if (el && dlg) el.textContent = dlg.error;
}

function openReportDialog(opts = {}) {
  if (typeof showDialog !== "function") return;
  const dlg = (reportsUi.dialog = reportDialogState(opts));
  showDialog("");
  shellDialog.dataset.kind = "report";
  shellDialog.querySelector(".popup").classList.add("rp-popup");
  drawReportDialog();
  if (!reportHub()) return;
  reportCall(reportAsk({ op: "status" })).then(
    (answer) => (dlg.status = answer?.available ? "ready" : "unavailable"),
    () => (dlg.status = "unavailable"),
  ).then(() => {
    if (reportsUi.dialog !== dlg) return;
    drawReportDialog();
    if (dlg.status === "ready") {
      reportDialogEl()?.querySelector("#rp-title")?.focus();
      if (dlg.draft.title.length >= REPORT_TITLE_MIN) searchSimilar(dlg);
    }
  });
}

function searchSimilar(dlg) {
  const title = dlg.draft.title.trim();
  if (dlg.picked || title.length < REPORT_TITLE_MIN || title === dlg.similarFor) return;
  dlg.similarFor = title;
  const seq = ++dlg.seq;
  reportCall(reportSimilarFrame(title)).then(
    (found) => {
      if (reportsUi.dialog !== dlg || dlg.seq !== seq) return;
      dlg.similar = Array.isArray(found) ? found : [];
      drawReportSimilar();
    },
    () => {
      if (reportsUi.dialog === dlg && dlg.seq === seq) {
        dlg.similar = [];
        drawReportSimilar();
      }
    },
  );
}

async function reportPreview(dlg) {
  dlg.busy = true;
  dlg.error = "";
  drawReportDialog();
  try {
    const got = await reportCall(reportPreviewFrame(dlg, reportContext()));
    if (reportsUi.dialog !== dlg) return;
    if (!dlg.requestId || dlg.preview?.shown !== got.shown) dlg.requestId = reportUuid();
    dlg.preview = got;
    dlg.similar = Array.isArray(got.similar) ? got.similar : [];
    dlg.step = "preview";
  } catch (e) {
    dlg.error = String(e);
  }
  dlg.busy = false;
  drawReportDialog();
}

async function reportSend(dlg) {
  dlg.busy = true;
  dlg.error = "";
  drawReportDialog();
  try {
    const sent = await reportCall(reportSendFrame(dlg));
    if (reportsUi.dialog !== dlg) return;
    dlg.result = { kind: dlg.draft.to_issue ? "followed" : "sent", number: sent?.number ?? dlg.draft.to_issue, url: sent?.url };
    dlg.step = "done";
    if (dlg.confirmationId) reportsUi.card.delete(dlg.confirmationId);
    reloadReportsIfShown();
  } catch (e) {
    dlg.error = String(e);
  }
  dlg.busy = false;
  drawReportDialog();
}

async function reportSaveDraft(dlg) {
  dlg.busy = true;
  dlg.error = "";
  drawReportDialog();
  try {
    const saved = await reportCall(reportSaveFrame(dlg));
    if (reportsUi.dialog !== dlg) return;
    dlg.draftId = saved?.id ?? dlg.draftId;
    dlg.result = { kind: "draft" };
    dlg.step = "done";
    reloadReportsIfShown();
  } catch (e) {
    dlg.error = String(e);
  }
  dlg.busy = false;
  drawReportDialog();
}

const REPORT_DIALOG_ACTS = {
  kind: (dlg, b) => {
    dlg.draft.kind = b.dataset.kind;
    drawReportDialog();
  },
  attach: (dlg, b) => {
    if (b.dataset.attach === "environment") dlg.draft.attach.environment = !dlg.draft.attach.environment;
    else {
      dlg.errorsOn = !dlg.errorsOn;
      if (dlg.errorsOn) dlg.errorLines = reportRingLines();
    }
    drawReportDialog();
  },
  pick: (dlg, b) => {
    const x = dlg.similar[Number(b.dataset.index)];
    if (!x) return;
    dlg.picked = { number: Number(x.number), title: String(x.title ?? "") };
    dlg.draft.to_issue = dlg.picked.number;
    if (dlg.step === "preview") {
      dlg.preview = null;
      reportPreview(dlg);
    } else drawReportDialog();
  },
  unpick: (dlg) => {
    dlg.picked = null;
    dlg.draft.to_issue = null;
    dlg.similarFor = "";
    drawReportDialog();
    searchSimilar(dlg);
  },
  review: (dlg) => reportReady(dlg) && !dlg.busy && reportPreview(dlg),
  save: (dlg) => !dlg.busy && reportSaveDraft(dlg),
  send: (dlg) => !dlg.busy && reportSend(dlg),
  back: (dlg) => {
    dlg.step = "form";
    dlg.error = "";
    drawReportDialog();
  },
  cancel: () => closeDialog(),
  another: () => openReportDialog(),
  mine: () => {
    closeDialog();
    openPage("reports");
  },
  github: (dlg, b) => reportOpenUrl(b.dataset.url),
};

if (typeof shellDialog !== "undefined") {
  shellDialog.addEventListener("click", (e) => {
    const dlg = reportsUi.dialog;
    const b = reportDialogEl() && e.target.closest("[data-rp]");
    if (b && dlg && !b.disabled) REPORT_DIALOG_ACTS[b.dataset.rp]?.(dlg, b);
  });
  shellDialog.addEventListener("input", (e) => {
    const dlg = reportsUi.dialog;
    const field = reportDialogEl() && e.target.name;
    if (!dlg || !["title", "happened", "expected", "repeat"].includes(field)) return;
    dlg.draft[field] = e.target.value;
    const review = reportDialogEl().querySelector('[data-rp="review"]');
    if (review) review.disabled = !reportReady(dlg) || dlg.busy;
    if (field !== "title" || dlg.picked) return;
    clearTimeout(dlg.timer);
    dlg.timer = setTimeout(() => searchSimilar(dlg), 500);
  });
}

/** Opens a stored report's draft in the dialog. */
function openReportDraft(report) {
  openReportDialog({ draft: report.draft, draftId: report.id });
}

/* ---- My reports ---- */

function reloadReportsIfShown() {
  if (typeof shellPage === "function" && shellPage() === "reports") loadReports();
}

async function loadReports() {
  reportsUi.listState = "loading";
  drawReportsPage();
  try {
    const list = await reportCall(reportAsk({ op: "list" }));
    reportsUi.list = Array.isArray(list) ? list : [];
    reportsUi.listState = "ready";
  } catch (e) {
    reportsUi.listState = "error";
    reportsUi.listError = String(e);
  }
  drawReportsPage();
}

function reportsRowHtml(row, s, group) {
  const r = row.report;
  const t = REPORT_TEXT;
  const tag = (r.kind === "idea" || r.draft?.kind === "idea") ? `<span class="rp-tag">${reportEsc(t.tagIdea)}</span>` : "";
  const num = r.issue ? `<span class="rp-num">#${Number(r.issue)}</span>` : `<span class="rp-num"></span>`;
  const title = `<span class="rp-row-title">${tag}${s.slot(reportRowTitle(r))}</span>`;
  const line = group === "drafts" ? "" : `<span class="rp-row-line" data-tone="${row.line.tone}">${s.slot(row.line.text)}</span>`;
  const texts = `<span class="rp-row-texts">${title}${line}</span>`;
  let actions = "";
  if (group === "drafts") actions = `<button type="button" class="rp-link" data-rp="open-draft" data-id="${reportEsc(r.id)}">${reportEsc(t.openDraft)}</button><button type="button" class="rp-quiet" data-rp="delete-draft" data-id="${reportEsc(r.id)}">${reportEsc(t.deleteDraft)}</button>`;
  else {
    if (row.line.update) actions += `<button type="button" class="outline" data-rp="update">${reportEsc(t.updatePinned)}</button>`;
    actions += reportLinkHtml(r.url);
  }
  return `<div class="rp-row" data-group="${group}">${group === "drafts" ? "" : num}${texts}<span class="rp-row-actions">${actions}</span></div>`;
}

function reportsPageHtml(s) {
  const t = REPORT_TEXT;
  const e = reportEsc;
  const head = `<header class="rp-page-head"><div><h1>${e(t.pageTitle)}</h1><p>${e(t.pageAbout)}</p></div><button type="button" class="primary" data-rp="new">${e(t.menuReport)}</button></header>`;
  if (reportsUi.listState === "error") return `${head}<p class="rp-error" role="alert">${s.slot(t.loadFailed(reportsUi.listError))}</p>`;
  if (!reportsUi.list) return `${head}<p class="rp-hint" role="status">${e(t.loading)}</p>`;
  const groups = reportGroups(reportsUi.list, reportAppVersion());
  const section = (id, empty) => {
    const rows = groups[id];
    if (!rows.length && !empty) return "";
    return `<section class="rp-group" data-group="${id}"><h2>${e(t.groups[id])}</h2>${rows.length ? rows.map((row) => reportsRowHtml(row, s, id)).join("") : `<p class="rp-hint">${e(empty)}</p>`}</section>`;
  };
  const any = reportsUi.list.length > 0;
  return `${head}${any ? "" : `<p class="rp-hint">${e(t.nothing)}</p>`}${section("news")}${section("open")}${section("drafts", any ? t.noDrafts : "")}${section("closed")}<p class="rp-foot">${e(t.pageNote)}</p>`;
}

function drawReportsPage() {
  if (typeof shellPage !== "function" || shellPage() !== "reports") return;
  const s = reportSlots();
  const body = pageView.querySelector(".rp-page");
  if (!body) return;
  body.innerHTML = reportsPageHtml(s);
  paintSlots(body, s);
}

function showReports() {
  setPage(`${headHtml(REPORT_TEXT.pageTitle)}<div class="page-body"><div class="rp-page"></div></div>`);
  loadReports();
}

const REPORT_PAGE_ACTS = {
  new: () => openReportDialog(),
  github: (b) => reportOpenUrl(b.dataset.url),
  update: () => typeof runUpdate === "function" && runUpdate(true),
  "open-draft": (b) => {
    const report = reportsUi.list?.find((r) => r.id === b.dataset.id);
    if (report) openReportDraft(report);
  },
  "delete-draft": async (b) => {
    try {
      await reportCall(reportAsk({ op: "delete_draft", id: b.dataset.id }));
    } catch (e) {
      reportsUi.listError = String(e);
      reportsUi.listState = "error";
      return drawReportsPage();
    }
    loadReports();
  },
};

if (typeof pageView !== "undefined") {
  pageView.addEventListener("click", (e) => {
    if (typeof shellPage !== "function" || shellPage() !== "reports") return;
    const b = e.target.closest("[data-rp]");
    if (b && !b.disabled) REPORT_PAGE_ACTS[b.dataset.rp]?.(b);
  });
}
if (typeof SHELL_PAGES !== "undefined") SHELL_PAGES.reports = { show: showReports };

/** The Account and pages menu's "Report a problem". */
document.addEventListener("click", (e) => {
  if (e.target.closest?.("[data-report-new]")) openReportDialog();
});

/* ---- the agent's request: a Confirm action `report_issue` ---- */

/** What the agent drafted: the confirmation's arguments are the draft, alone or under `draft`. */
const reportCardDraft = (c) => reportNormalize(c.args?.draft ?? c.args);

/** The hub's preview of the agent's draft, fetched once per confirmation. */
function reportCardState(c) {
  let st = reportsUi.card.get(c.id);
  if (st) return st;
  st = { state: c.args?.preview ? "ready" : "loading", preview: c.args?.preview ?? null };
  reportsUi.card.set(c.id, st);
  if (st.state === "loading" && reportHub()) {
    reportCall(reportAsk({ op: "preview", draft: reportCardDraft(c), context: reportContext() })).then(
      (got) => Object.assign(st, { state: "ready", preview: got }),
      () => Object.assign(st, { state: "failed" }),
    ).then(() => typeof agentSurfaceChanged === "function" && agentSurfaceChanged());
  } else if (st.state === "loading") st.state = "failed";
  return st;
}

/** The card's body: an empty placeholder, which `paintReportCards` fills with DOM and textContent. */
function reportCardBody(c) {
  const st = reportCardState(c);
  return `<div class="rp-card" data-rp-card="${reportEsc(c.id)}" data-rp-state="${st.state}"></div>`;
}

function reportCardActions(c, busy, ny) {
  const t = REPORT_TEXT.agent;
  const b = (act, label, cls, nyCls) => (ny ? `<button type="button" class="${nyCls}" data-ny="${act}" data-agent="${act}"${busy}>${reportEsc(label)}</button>` : `<button type="button" class="${cls}" data-agent="${act}"${busy}>${reportEsc(label)}</button>`);
  const out = ny
    ? `<div class="ny-actions">${b("deny", t.skip, "", "ny-secondary")}${b("report-edit", t.editFirst, "", "ny-secondary")}${b("approve", t.send, "", "ny-primary")}</div>`
    : `<div class="actions start">${b("approve", t.send, "primary")}${b("report-edit", t.editFirst, "outline")}${b("deny", t.skip, "outline")}</div>`;
  return out;
}

function paintReportCard(el) {
  const c = typeof agentConfirmations === "undefined" ? null : agentConfirmations.find((x) => x.id === el.dataset.rpCard);
  if (!c) return;
  const st = reportsUi.card.get(c.id) ?? reportCardState(c);
  const t = REPORT_TEXT.agent;
  const draft = reportCardDraft(c);
  const h = (tag, cls, text) => {
    const node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text != null) node.textContent = text;
    return node;
  };
  const kids = [];
  const sections = st.preview?.body ? parseIssueBody(st.preview.body) : [{ heading: REPORT_TEXT.fields[draft.kind].happened, text: draft.happened, code: [] }, { heading: REPORT_TEXT.fields[draft.kind].expected, text: draft.expected, code: [] }, ...(draft.kind === "problem" && draft.repeat ? [{ heading: REPORT_TEXT.fields.problem.repeat, text: draft.repeat, code: [] }] : [])].filter((x) => x.text);
  const head = h("div", "rp-card-head");
  head.append(h("span", `rp-chip kind-${draft.kind}`, REPORT_TEXT.kinds[draft.kind]), h("span", "rp-card-title", st.preview?.title ?? draft.title));
  kids.push(head);
  const labels = st.preview?.labels ?? [];
  if (labels.length) {
    const row = h("div", "rp-card-labels");
    row.append(...labels.map((l) => h("span", "rp-chip", l)));
    kids.push(row);
  }
  const body = h("div", "rp-card-sections");
  for (const x of sections) {
    const sec = h("div", "rp-section");
    if (x.heading) sec.append(h("div", "rp-section-head", x.heading));
    if (x.text) sec.append(h("div", "rp-section-text", x.text));
    if (x.code.length) sec.append(h("pre", "rp-lines", x.code.join("\n")));
    body.append(sec);
  }
  kids.push(body);
  kids.push(h("div", "rp-card-note", `${t.publicTag}. ${t.ask}`));
  const similar = (st.preview?.similar ?? [])[0];
  if (similar) {
    const row = h("div", "rp-card-similar");
    const text = h("span", "rp-similar-title");
    text.textContent = `${t.similar(Number(similar.number))} ${similar.title ?? ""}`.trim();
    const follow = h("button", "outline");
    follow.type = "button";
    follow.dataset.agent = "report-follow";
    follow.dataset.number = String(Number(similar.number));
    follow.textContent = t.followSimilar(Number(similar.number));
    row.append(text, follow);
    kids.push(row);
  } else if (st.state === "loading") kids.push(h("div", "rp-card-note", t.loading));
  el.replaceChildren(...kids);
}

function paintReportCards(root = document) {
  for (const el of root.querySelectorAll?.("[data-rp-card]") ?? []) if (!el.firstChild) paintReportCard(el);
}

if (typeof MutationObserver !== "undefined" && typeof document !== "undefined" && document.body) {
  new MutationObserver((records) => {
    for (const rec of records) for (const node of rec.addedNodes) if (node.nodeType === 1) (node.matches?.("[data-rp-card]") ? paintReportCard(node) : paintReportCards(node));
  }).observe(document.body, { childList: true, subtree: true });
}

/** Opens the dialog with the agent's draft and denies the request, as Edit first does. */
function reportEditFirst(button, number) {
  const id = button.closest("[data-confirmation]")?.dataset.confirmation;
  const c = typeof agentConfirmations === "undefined" ? null : agentConfirmations.find((x) => x.id === id);
  if (!c) return;
  const draft = reportCardDraft(c);
  const similar = (reportsUi.card.get(c.id)?.preview?.similar ?? []).find((x) => Number(x.number) === number);
  if (number) draft.to_issue = number;
  openReportDialog({ draft, picked: similar && { number, title: String(similar.title ?? "") } });
  if (reportsUi.dialog) reportsUi.dialog.confirmationId = c.id;
  decideAgentConfirmation(c.id, false);
}

(globalThis.agentCards ??= {}).report_issue = { body: reportCardBody, actions: reportCardActions };
if (typeof AGENT_CLICKS !== "undefined") {
  AGENT_CLICKS["report-edit"] = (button) => reportEditFirst(button, 0);
  AGENT_CLICKS["report-follow"] = (button) => reportEditFirst(button, Number(button.dataset.number));
}
