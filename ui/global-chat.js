// The global orchestrator's chat: what a Captain's report looks like as text, the quick replies an
// answer can end with, what read aloud says, which threads wait on Felipe, and the settings every
// surface reads. The parsers are pure so node --test drives them without the page; gui-sidebar.js
// and gui-timeline.js paint from them, and only in the global orchestrator's own tab.

/** A Captain's report line, as the contracts fix it:
 * `[pinned] Capitão <project> (<machine name>) [<machine id> <tab id>]: <text>`. The agents write
 * `Capitão`; `Captain` reads the same. */
const REPORT_HEADER = /^\[pinned\] (?:Capitão|Captain) ([^()\[\]\n]+) \(([^()\[\]\n]*)\) \[(\S+) (\S+)\]: /gm;

/** The report blocks one user prompt holds, in order. A block runs from its header line to the
 * next header or the end, so words between two headers belong to the earlier block; `start` is
 * where the header sat, which leaves the text before the first header as the sender's own. */
function reportBlocks(text) {
  const source = String(text ?? "");
  const hits = [...source.matchAll(REPORT_HEADER)];
  return hits.map((m, i) => ({
    start: m.index,
    project: m[1].trim(),
    machineName: m[2].trim(),
    machineId: m[3],
    tabId: m[4],
    text: source.slice(m.index + m[0].length, i + 1 < hits.length ? hits[i + 1].index : source.length).trim(),
  }));
}

/** The one-line summary of several reports: how many came, and which Captains, each named once. */
function reportFoldLabel(blocks) {
  const names = [...new Set(blocks.map((b) => `Captain ${b.project}`))];
  return `${blocks.length} ${blocks.length === 1 ? "update" : "updates"} · ${names.join(", ")}`;
}

/** The quick replies an answer offers: its last non-empty line must read `Opções: a | b` (`Options: a | b` reads the same). Returns
 * the options and the answer without that line, or null when it offers nothing. */
function quickReplyOf(text) {
  const lines = String(text ?? "").split("\n");
  for (let i = lines.length - 1; i >= 0; i--) {
    if (!lines[i].trim()) continue;
    const found = lines[i].match(/^(?:Opções|Options): (.+)$/);
    if (!found) return null;
    const options = found[1].split(" | ").map((option) => option.trim()).filter(Boolean);
    if (!options.length) return null;
    lines.splice(i, 1);
    return { options, text: lines.join("\n").trimEnd() };
  }
  return null;
}

/** What read aloud says of an answer: the words, without code blocks and without the quick-reply
 * line, on one line. An unclosed code fence swallows the rest, as it does on screen. */
function speechTextOf(text) {
  const outside = String(text ?? "").split("```").filter((_, i) => i % 2 === 0).join(" ");
  const kept = outside.split("\n").filter((line) => !/^(?:Opções|Options): /.test(line.trim()));
  return kept.join(" ").replace(/\s+/g, " ").trim();
}

/** Why a thread waits on Felipe: `pergunta` for a question mid-turn (needs-input with a live menu
 * or interrupted), `falha` for a failure. A thread Claude Code is merely done with is not waiting,
 * which is the idle notice: needs-input without a menu and without interrupted. */
function waitingKindOf(view, hasMenu) {
  if (!view) return null;
  if (view.status === "failed") return "falha";
  if (view.status === "needs-input" && (hasMenu || view.interrupted === true)) return "pergunta";
  return null;
}

/** The settings as the contracts default them; a machine's answer or a save only overrides what
 * it names. */
const GLOBAL_SETTINGS_DEFAULTS = { name: "Orchestrator", host: false, busy: "queue", reports: "auto", group_reports: true, read_aloud: false };
/** The one settings cache the UI reads. Written by the ask below and by the
 * `global-orchestrator-settings` event the settings page dispatches after a save. */
const globalSettings = { ...GLOBAL_SETTINGS_DEFAULTS };
/** Each machine's answer by machine id, and which machines were already asked this round. A
 * machine that comes online later is asked on the next sidebar render; a failed ask is retried. */
const globalSettingsBy = new Map();
const globalAsked = new Set();
/** The machine that answered `host: true`. */
let globalHostId = null;

/** Whether a tab is the global orchestrator's own, guarded because this file loads before
 * composer-tags.js defines the test. */
const isGlobalTab = (tab) => typeof isGlobalOrchestratorTab === "function" && isGlobalOrchestratorTab(tab);

/** Where the global orchestrator lives: the machine that answered `host: true`, else this machine,
 * else the first machine online. Null only when no machine can hold the conversation. */
function globalHostMachine() {
  if (typeof machines === "undefined") return null;
  if (globalHostId && machines.has(globalHostId)) return globalHostId;
  if (typeof LOCAL !== "undefined" && machines.has(LOCAL)) return LOCAL;
  return [...machines.entries()].find(([, machine]) => machine.view.online)?.[0] ?? null;
}

/** Asks every online machine once for its orchestrator settings, so the host and its name turn up
 * without anyone configuring anything. What the UI reads comes from the host's answer, or the
 * defaults while no machine claims the job. A change re-renders, since the render that asked drew
 * the defaults. */
async function askGlobalSettings() {
  if (typeof machines === "undefined" || typeof callOn !== "function") return;
  const read = () => JSON.stringify([globalSettings.name, globalHostMachine(), globalSettings.group_reports, globalSettings.read_aloud]);
  const before = read();
  for (const [id, machine] of machines) {
    if (!machine.view.online || globalAsked.has(id)) continue;
    globalAsked.add(id);
    try {
      const saved = await callOn(id, { cmd: "global_orchestrator_settings" });
      if (saved && typeof saved === "object") globalSettingsBy.set(id, { ...GLOBAL_SETTINGS_DEFAULTS, ...saved });
      if (saved?.host === true && !globalHostId) globalHostId = id;
    } catch {
      globalAsked.delete(id);
    }
  }
  Object.assign(globalSettings, GLOBAL_SETTINGS_DEFAULTS, (globalHostMachine() && globalSettingsBy.get(globalHostMachine())) || {});
  if (read() !== before) {
    if (typeof renderSidebar === "function") renderSidebar();
    if (typeof renderThread === "function" && typeof active !== "undefined" && isGlobalTab(active)) renderThread();
  }
}

/** Which assistant messages were already read aloud, by tab key. A thread first seen while read
 * aloud is on starts silent, so opening the app never replays the whole conversation. */
const spokenIds = new Map();
/** Reads each new answer of the global orchestrator's tab aloud, on a phone only: pt-BR, without
 * code blocks or the quick-reply line. An answer older than ten minutes stays silent, and a newer
 * one cancels whatever is still playing. */
function speakGlobalTexts(tab, turns) {
  if (!globalSettings.read_aloud) return;
  if (typeof touchComposer !== "function" || !touchComposer()) return;
  if (typeof speechSynthesis === "undefined" || !speechSynthesis) return;
  let spoken = spokenIds.get(tab.key);
  if (!spoken) {
    spoken = new Set(turns.map((turn) => turn.answer?.id).filter(Boolean));
    spokenIds.set(tab.key, spoken);
  }
  for (const turn of turns) {
    const answer = turn.answer;
    if (!answer?.id || spoken.has(answer.id)) continue;
    spoken.add(answer.id);
    if (!turn.end || turn.end < Date.now() - 10 * 60 * 1000) continue;
    const text = speechTextOf(answer.text);
    if (!text) continue;
    const utterance = new SpeechSynthesisUtterance(text);
    utterance.lang = "pt-BR";
    speechSynthesis.cancel();
    speechSynthesis.speak(utterance);
  }
}

// The settings page saves on the host machine and dispatches the saved settings here; the ask
// starts over so the next render re-reads every machine.
if (typeof document !== "undefined" && typeof document.addEventListener === "function") {
  document.addEventListener("global-orchestrator-settings", (event) => {
    Object.assign(globalSettings, GLOBAL_SETTINGS_DEFAULTS, event?.detail ?? {});
    globalAsked.clear();
    if (typeof renderSidebar === "function") renderSidebar();
    if (typeof renderThread === "function" && typeof active !== "undefined" && isGlobalTab(active)) renderThread();
  });
}
