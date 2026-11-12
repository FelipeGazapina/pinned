/** Composer dictation: hold-to-talk is out, talk-until-done is in.
 *
 * One tap on the mic starts a session that only ends when the message is sent
 * or the mic is tapped again — never on a pause. While it runs:
 * - interim results show as provisional text at the caret,
 * - each final result confirms one phrase there and the caret moves past it,
 * - typing or editing confirms the provisional text on the spot, and a click
 *   or a typed edit moves where the next phrase goes.
 *
 * The transcript is cleaned with the same ideas as the speech-to-text app:
 * a developer glossary steers spelling (kubectl, useState, async/await…),
 * filler sounds are dropped, and spoken code shapes ("snake case X",
 * "utils dot ts") become written code. The heavy LLM rewrite pass the other
 * app does stays out of 0.17.2; the deterministic layer below is what ships.
 *
 * In the Mac app a tap of fn (Globe) alone does what the mic does.
 *
 * When a session that wrote something ends, a badge in that thread asks how it
 * went and then whether to share its text with the hub to improve dictation.
 * Nothing is sent unless the user says Save. After one Save the app remembers
 * the consent and the badge asks only how it went, sending the answer.
 *
 * The language button next to the mic cycles DICTATION_LANGUAGES and
 * remembers the choice; a running session switches without ending.
 *
 * Engines, in order: the app's on-device helper (macOS Speech framework,
 * `dictation_start`/`dictation_stop`), then the browser's SpeechRecognition
 * where the WebView offers it. Elsewhere the mic explains itself.
 */

const DICTATION_LANGUAGES = [
  { locale: "en-US", label: "EN", name: "English" },
  { locale: "pt-BR", label: "PT", name: "Português" },
];
const DICTATION_LOCALE_KEY = "dictation-locale";
let dictationChosenLocale = null;
try {
  dictationChosenLocale = localStorage.getItem(DICTATION_LOCALE_KEY);
} catch {}

function dictationLanguage(locale = dictationLocale()) {
  return DICTATION_LANGUAGES.find((language) => language.locale === locale);
}

function dictationLocale() {
  if (DICTATION_LANGUAGES.some((language) => language.locale === dictationChosenLocale)) return dictationChosenLocale;
  return (navigator.language || "en-US").startsWith("pt") ? "pt-BR" : "en-US";
}

/** The next language in the table becomes the choice. Memory keeps it when storage can't. */
function cycleDictationLocale() {
  const index = DICTATION_LANGUAGES.findIndex((language) => language.locale === dictationLocale());
  dictationChosenLocale = DICTATION_LANGUAGES[(index + 1) % DICTATION_LANGUAGES.length].locale;
  try {
    localStorage.setItem(DICTATION_LOCALE_KEY, dictationChosenLocale);
  } catch {}
  return dictationChosenLocale;
}

const DICTATION_CONSENT_KEY = "dictation-feedback-consent";
let dictationConsent = false;
try {
  dictationConsent = localStorage.getItem(DICTATION_CONSENT_KEY) === "yes";
} catch {}

function rememberDictationConsent() {
  dictationConsent = true;
  try {
    localStorage.setItem(DICTATION_CONSENT_KEY, "yes");
  } catch {}
}

// Terms Whisper/SFSpeech tend to mangle in developer speech. Applied as
// whole-word, case-insensitive fixes on confirmed phrases only, so interim
// text never flickers under the speaker.
// Each `heard` is a word-boundary-safe alternation (no ^$ anchors: the caller
// wraps it with lookarounds for whole-word matching).
const DICTATION_GLOSSARY = [
  ["kubectl", /cube\s*control|kube\s*cuttle/],
  ["GitHub", /get\s*hub/],
  ["useState", /use\s*state/],
  ["useEffect", /use\s*effect/],
  ["async/await", /async\s*await/],
  ["PostgreSQL", /postgres\s*q\s*l|post\s*gress/],
  ["CI/CD", /c\s*i\s*c\s*d/],
  ["pull request", /pool\s*request/],
  ["monorepo", /mono\s*repo/],
  ["localhost", /local\s*host/],
  ["stdout", /standard\s*out/],
  ["stderr", /standard\s*error/],
  ["regex", /rejex/],
  ["OAuth", /o\s*auth/],
  ["JWT", /j\s*w\s*t/],
  ["UUID", /u\s*u\s*i\s*d/],
  ["webhook", /web\s*hook/],
  ["pytest", /pie\s*test/],
  ["Tailwind", /tail\s*wind/],
  ["TypeScript", /type\s*script/],
  ["JavaScript", /java\s*script/],
  ["Next.js", /next\s*(dot\s*)?j\s*s/],
  ["Node.js", /node\s*(dot\s*)?j\s*s/],
  ["Vite", /veet/],
];
// Cheap whole-word fixes on top: spoken name -> written name.
const DICTATION_TERMS = {
  "kubectl": "kubectl",
  "github": "GitHub",
  "postgres": "PostgreSQL",
  "docker": "Docker",
  "kubernetes": "Kubernetes",
  "terraform": "Terraform",
  "typescript": "TypeScript",
  "javascript": "JavaScript",
  "python": "Python",
  "golang": "Go",
  "rust": "Rust",
  "react": "React",
  "nextjs": "Next.js",
  "nodejs": "Node.js",
  "usestate": "useState",
  "useeffect": "useEffect",
  "async await": "async/await",
  "ci cd": "CI/CD",
  "pull request": "pull request",
  "monorepo": "monorepo",
  "localhost": "localhost",
  "tailwind": "Tailwind",
  "claude": "Claude",
  "ollama": "Ollama",
  "whisper": "Whisper",
  "refactor": "refactor",
  "endpoint": "endpoint",
  "middleware": "middleware",
  "schema": "schema",
};
// The words the on-device recognizer is told to expect, so it writes "commit" where it would
// hear "comet": every written form the cleanup produces, plus the trade's everyday words.
const DICTATION_VOCABULARY = [...new Set([
  ...DICTATION_GLOSSARY.map(([written]) => written),
  ...Object.values(DICTATION_TERMS),
  "comment", "comments", "commit", "commits", "PR", "pull request", "merge", "rebase", "branch", "worktree",
  "deploy", "refactor", "endpoint", "subagent", "hub", "pinned", "Claude", "Cursor", "Railway", "Postgres",
  "Tauri", "Rust", "Swift", "lint", "diff", "repo", "stash", "cherry-pick",
])];
// The hub keeps up to this many characters of each text in a shared dictation.
const DICTATION_FEEDBACK_CHARS = 20_000;
// What the recognizer hears on silence or noise. Dropped only when it is the
// entire confirmed phrase, exactly like the speech-to-text app.
const DICTATION_HALLUCINATIONS = new Set([
  "", "you", "thank you", "thanks", "thank you for watching", "thanks for watching",
  "bye", "subtitles by the amara.org community", ".",
  "obrigado", "obrigada", "tchau",
]);

const dictationState = {
  want: false, // the session the user asked for; only send or the mic ends it
  engine: null, // 'native' | 'web' | null while starting
  live: null, // the interim phrase on screen: { text, span: [from, to], value } while prompt.value === value
  session: null, // the running session's final phrases: { raw: [], cleaned: [], startedAt }
  feedback: null, // the badge: { step: "rate" | "consent" | "saving" | "saved" | "failed", record, rating, thread, sent, error }
  rendering: false, // our own prompt.value writes, not the user's typing
  rec: null, // web SpeechRecognition instance
  locale: null, // what the running engine listens in; cleanup follows it, not the button
  noted: false, // a dictation note is up and the next transcript clears it
  restarts: 0,
  checking: false, // a click is asking for permissions; a second click waits for it
  unlisten: [],
};

/** The two macOS permissions native dictation needs, in the order a click explains them. */
const DICTATION_PERMISSIONS = [
  { key: "microphone", name: "Microphone" },
  { key: "speechRecognition", name: "Speech Recognition" },
];

/** What one mic click does, from the helper's report of each permission (notDetermined, denied,
 * restricted or granted): explain the first one denied or restricted, with a button to its
 * System Settings pane; record when both are granted; otherwise start the helper, whose request
 * shows the system prompt. */
function dictationPermissionAction(status) {
  for (const { key, name } of DICTATION_PERMISSIONS) {
    const state = status?.[key];
    if (state === "denied") {
      return { action: "explain", settings: key, message: `pinned isn't allowed to use ${name}. Turn on pinned under Privacy & Security > ${name} in System Settings, then click the mic again.` };
    }
    if (state === "restricted") {
      return { action: "explain", settings: key, message: `${name} is restricted on this Mac, so pinned can't use it. Screen Time or a profile set by your organization controls this.` };
    }
  }
  return DICTATION_PERMISSIONS.every(({ key }) => status?.[key] === "granted") ? { action: "record" } : { action: "prompt" };
}

/** Asks macOS again on every click and remembers nothing, so a grant made in System Settings
 * counts on the next click. A failed check starts the helper, which reports its own error. */
async function dictationPermissionGate(ask = () => invoke("dictation_permissions")) {
  try {
    return dictationPermissionAction(await ask());
  } catch {
    return { action: "prompt" };
  }
}

/** The message for a browser that refused the microphone, or null when the failure is not a
 * block and speech recognition should still try. */
function webMicBlockedMessage(errorName) {
  if (errorName !== "NotAllowedError") return null;
  return "Your browser blocked the microphone for this site. Click the lock or site settings icon in the address bar, allow the microphone, then click the mic again.";
}

/** Deterministic cleanup for one confirmed phrase, ported from the
 * speech-to-text app's basic_cleanup + glossary. Never translates. */
function dictationCleanup(raw, locale = dictationLocale()) {
  let text = ` ${raw} `;
  // Filler sounds only ("um" is a real word in Portuguese, so it stays there).
  const fillers = locale.startsWith("pt") ? /\s+(a+h+n+|e+r+m+|h+m+|m+h*m+)[,.]?\s+/gi : /\s+(u+m+|u+h+|a+h+n*|e+r+m+|h+m+|m+h*m+)[,.]?\s+/gi;
  text = text.replace(fillers, " ");
  text = text.replace(/\s+/g, " ").trim().replace(/\s+([,.!?;:])/g, "$1").replace(/^[,.;:]\s*/, "");
  if (DICTATION_HALLUCINATIONS.has(text.trim().replace(/[^\w\s.]/g, "").trim().toLowerCase().replace(/\s+/g, " ").replace(/[.]+$/, ""))) return "";
  // Spoken code shapes with explicit instruction words (safe: the instruction is dropped).
  text = dictationCodeShapes(text);
  // Glossary: whole-word fixes that keep the speaker's wording otherwise.
  for (const [written, heard] of DICTATION_GLOSSARY) {
    text = text.replace(new RegExp(`(?<!\\w)${heard.source}(?!\\w)`, "gi"), written);
  }
  for (const [spoken, written] of Object.entries(DICTATION_TERMS)) {
    text = text.replace(new RegExp(`(?<!\\w)${spoken.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(?!\\w)`, "gi"), written);
  }
  return text.trim();
}

/** "snake case max retry count" -> "max_retry_count", "utils dot ts" ->
 * "utils.ts", "feature slash login" -> "feature/login". Only word-dot-word /
 * word-slash-word with code-like neighbors, so prose ("see. Dot.") survives. */
function dictationCodeShapes(text) {
  const caseOf = (kind, words) => {
    const parts = words.toLowerCase().split(/\s+/).filter(Boolean);
    if (!parts.length) return "";
    if (kind === "camel") return parts[0] + parts.slice(1).map((w) => w[0].toUpperCase() + w.slice(1)).join("");
    if (kind === "pascal") return parts.map((w) => w[0].toUpperCase() + w.slice(1)).join("");
    if (kind === "snake") return parts.join("_");
    if (kind === "kebab") return parts.join("-");
    if (kind === "constant" || kind === "all caps") return parts.join("_").toUpperCase();
    return words;
  };
  text = text.replace(/\b(camel|pascal|snake|kebab|constant)\s+case\s+([a-z][a-z0-9]*(?:\s+[a-z0-9]+){0,5})/gi, (_, k, w) => caseOf(k.toLowerCase(), w));
  text = text.replace(/\ball\s+caps\s+([a-z][a-z0-9]*(?:\s+[a-z0-9]+){0,5})/gi, (_, w) => caseOf("all caps", w));
  // "dash dash force" -> "--force", "dot" / "slash" between code words.
  text = text.replace(/\bdash\s*dash\s*/gi, "--").replace(/\bhyphen\s*hyphen\s*/gi, "--");
  text = text.replace(/([A-Za-z0-9])\s+dot\s+(ts|tsx|js|jsx|json|css|md|py|rs|go|toml|yaml|yml|lock)\b/gi, "$1.$2");
  text = text.replace(/([A-Za-z0-9_-]+)\s+dot\s+([A-Za-z0-9][A-Za-z0-9_-]*)/g, (m, a, b) => /^(com|org|io|dev|app)$/i.test(b) && a.length > 12 ? m : `${a}.${b}`);
  text = text.replace(/([A-Za-z0-9_.-]+)\s+slash\s+([A-Za-z0-9_.-]+)/g, "$1/$2");
  text = text.replace(/\bnew\s+line\b/gi, "\n").replace(/\bnova\s+linha\b/gi, "\n");
  return text;
}

/** The mic button's truth: recording while the user wants dictation. */
function dictationActive() {
  return dictationState.want;
}

/** The desktop app's helper is macOS-only; the web UI falls back to the browser's engine. */
const DICTATION_NEEDS_MAC = "Dictation needs macOS";
function dictationUnavailable() {
  return !!TAURI && !IS_MAC;
}

function renderDictationButton() {
  const button = document.getElementById("composer-dictate");
  if (!button) return;
  if (dictationUnavailable()) {
    button.disabled = true;
    button.dataset.active = false;
    button.setAttribute("aria-pressed", false);
    button.title = DICTATION_NEEDS_MAC;
    button.setAttribute("aria-label", DICTATION_NEEDS_MAC);
    return;
  }
  button.dataset.active = dictationActive();
  button.setAttribute("aria-pressed", dictationActive());
  button.title = `${dictationActive() ? "Stop dictation" : "Dictate with your voice"}${TAURI && IS_MAC ? " (fn)" : ""}`;
  const lang = document.getElementById("composer-dictate-lang");
  if (lang) {
    const { label, name } = dictationLanguage();
    lang.textContent = label;
    lang.title = `Dictation language: ${name}. Click to switch.`;
    lang.setAttribute("aria-label", lang.title);
  }
  composer.dataset.dictating = dictationActive();
  composer.dataset.provisional = !!(dictationActive() && liveDictation());
}

function dictationShow(note, error = false) {
  showComposerNote(note, error);
}

/** A status line that stays until the user hears from the engine again. */
function dictationNotice(note) {
  dictationState.noted = true;
  dictationShow(note);
}

function dictationClearNotice() {
  if (!dictationState.noted) return;
  dictationState.noted = false;
  dictationShow("");
}

/** The interim phrase, while the prompt still holds what dictation last wrote. Any edit,
 * typed or not, freezes it where it is. */
function liveDictation() {
  const live = dictationState.live;
  return live && live.value === prompt.value ? live : null;
}

/** Writes `text` over the live phrase, or at the caret when none shows. A final phrase
 * stays where it landed and the next one goes wherever the caret is then. */
function writeDictation(text, final) {
  const placed = placeDictation(prompt.value, liveDictation()?.span ?? null, [prompt.selectionStart, prompt.selectionEnd], text);
  dictationState.rendering = true;
  prompt.value = placed.value;
  prompt.setSelectionRange(...placed.selection);
  dictationState.rendering = false;
  dictationState.live = final ? null : { text, span: placed.span, value: placed.value };
  autosize();
  revealPromptCaret();
  if (active) saveDraft(active, prompt.value);
  renderDictationButton();
}

/** Scrolls the prompt so its caret shows: the bottom at the end, else the caret's line. */
function revealPromptCaret() {
  const { value, selectionStart: start, selectionEnd: end, scrollTop } = prompt;
  if (end >= value.length) return void (prompt.scrollTop = prompt.scrollHeight);
  // The text up to the caret, in a box too short to hold it, is as tall as the caret is low.
  const height = prompt.style.height;
  prompt.style.height = "0px";
  prompt.value = value.slice(0, end);
  const caretBottom = prompt.scrollHeight;
  prompt.value = value;
  prompt.style.height = height;
  prompt.setSelectionRange(start, end);
  const line = parseFloat(getComputedStyle(prompt).lineHeight) || 20;
  prompt.scrollTop = Math.min(Math.max(scrollTop, caretBottom - prompt.clientHeight), Math.max(0, caretBottom - line));
}

function dictationConfirmProvisional() {
  const live = liveDictation();
  if (live) confirmDictation(live.text);
}

/** A phrase the recognizer finished: cleaned, written in place, and kept for the feedback badge. */
function confirmDictation(raw) {
  const cleaned = dictationCleanup(raw, dictationState.locale);
  dictationState.session?.raw.push(raw);
  dictationState.session?.cleaned.push(cleaned);
  writeDictation(cleaned, true);
}

function joinDictation(base, addition) {
  if (!addition) return base;
  if (!base) return addition;
  if (base.endsWith(" ") || base.endsWith("\n") || /^[.,!?;:)\]}]/.test(addition)) return base + addition;
  return `${base} ${addition}`;
}

/** Dictated `text` placed in `value` over `span` (the live phrase) or, without one, over the
 * selection, spaced against the characters on both sides. The caret follows the text unless
 * the user put it outside the live phrase. `span` comes back covering the text and any space
 * added around it, so the next interim result replaces exactly that. */
function placeDictation(value, span, selection, text) {
  const [from, to] = span ?? selection;
  const before = joinDictation(value.slice(0, from), text);
  const after = value.slice(to);
  const glue = text && after && !/^[\s.,!?;:)\]}]/.test(after) && !/\s$/.test(text) ? " " : "";
  const next = before + glue + after;
  const caret = before.length;
  const follow = (at) => (!span || (at > from && at <= to) ? caret : at <= from ? at : at + next.length - value.length);
  return { value: next, selection: selection.map(follow), span: [from, caret + glue.length] };
}

/** One transcript from either engine. Interim replaces the live phrase; final
 * confirms it in place, cleaned. */
function onDictationResult(text, isFinal) {
  if (!dictationState.want) return;
  dictationClearNotice();
  if (isFinal) {
    dictationState.restarts = 0;
    confirmDictation(text);
  } else {
    writeDictation(text.trim(), false);
  }
}

function onDictationError(message) {
  if (!dictationState.want) return;
  const native = dictationState.engine === "native";
  stopDictation();
  dictationShow(`Dictation stopped: ${message}`, true);
  // A prompt the user just answered with Don't Allow ends here; say how to turn it on.
  if (native && IS_MAC) recheckDictationPermission();
}

/** The permission message under the composer; `settings` names the System Settings pane its
 * button opens, and without it the button hides (a browser's settings can't be opened for it). */
const dictationPermissionNote = document.getElementById("dictation-permission");
const DICTATION_ALLOWED_NOTE = "Microphone and Speech Recognition are on. Click the mic to dictate.";

function showDictationPermission({ message, settings }) {
  if (!dictationPermissionNote) return;
  dictationShow("");
  dictationPermissionNote.querySelector("span").textContent = message;
  const button = dictationPermissionNote.querySelector("button");
  button.hidden = !settings;
  button.dataset.settings = settings ?? "";
  dictationPermissionNote.hidden = false;
}

function hideDictationPermission() {
  if (dictationPermissionNote) dictationPermissionNote.hidden = true;
}

async function recheckDictationPermission() {
  const gate = await dictationPermissionGate();
  if (gate.action === "explain") return showDictationPermission(gate);
  if (dictationPermissionNote?.hidden) return;
  hideDictationPermission();
  if (gate.action === "record") dictationShow(DICTATION_ALLOWED_NOTE);
}

/** Asked on every click: macOS through the helper, a browser through getUserMedia. */
async function dictationAllowed() {
  hideDictationPermission();
  if (TAURI && IS_MAC) {
    const gate = await dictationPermissionGate();
    if (gate.action !== "explain") return true;
    showDictationPermission(gate);
    return false;
  }
  if (TAURI || !navigator.mediaDevices?.getUserMedia) return true;
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    for (const track of stream.getTracks()) track.stop();
    return true;
  } catch (err) {
    const message = webMicBlockedMessage(err?.name);
    if (!message) return true;
    showDictationPermission({ message });
    return false;
  }
}

async function startDictation() {
  if (dictationState.want || dictationUnavailable()) return;
  dictationState.want = true;
  dictationState.live = null;
  dictationState.session = { raw: [], cleaned: [], startedAt: Date.now() };
  dropDictationFeedback();
  if (composerNote.textContent === DICTATION_ALLOWED_NOTE) dictationShow("");
  dictationState.restarts = 0;
  dictationState.locale = dictationLocale();
  renderDictationButton();
  prompt.focus();
  // Native first: the app's on-device helper. The browser engine covers the web build.
  if (TAURI) {
    try {
      dictationState.unlisten.push(await listen("dictation://result", ({ text, isFinal }) => onDictationResult(text, isFinal)));
      dictationState.unlisten.push(await listen("dictation://status", ({ running, error }) => {
        if (error) onDictationError(error);
        else if (!running && dictationState.want && dictationState.engine === "native") startDictationEngine();
      }));
      dictationState.unlisten.push(await listen("dictation://notice", ({ message }) => dictationState.want && dictationNotice(message)));
      await startNativeDictation(dictationState.locale);
      dictationState.engine = "native";
      renderDictationButton();
      return;
    } catch (err) {
      for (const off of dictationState.unlisten.splice(0)) off();
      // Fall through to the web engine, which may still work.
    }
  }
  if (!startWebDictation()) {
    dictationState.want = false;
    renderDictationButton();
    dictationShow("Dictation isn't available in this build: no on-device helper and no browser speech recognition.", true);
  }
}

/** The session the helper dropped (it never ends one itself); reopen it while wanted. */
function startDictationEngine() {
  if (!dictationState.want || dictationState.engine !== "native") return;
  startNativeDictation(dictationState.locale).catch((err) => onDictationError(String(err)));
}

function startNativeDictation(locale) {
  return invoke("dictation_start", { locale, vocabulary: DICTATION_VOCABULARY });
}

function stopDictation() {
  if (!dictationState.want && !dictationState.engine) return;
  const { session, engine } = dictationState;
  dictationState.session = null;
  dictationState.want = false;
  // Whatever is on screen is what the user keeps.
  dictationState.live = null;
  dictationClearNotice();
  if (dictationState.engine === "native" && TAURI) invoke("dictation_stop").catch(() => {});
  dropWebDictation();
  for (const off of dictationState.unlisten.splice(0)) off();
  dictationState.engine = null;
  renderDictationButton();
  if (session?.cleaned.some(Boolean) && account?.user_token) offerDictationFeedback(dictationRecord(session, engine));
}

function dropWebDictation() {
  if (!dictationState.rec) return;
  try {
    dictationState.rec.onend = null;
    dictationState.rec.stop();
  } catch {}
  dictationState.rec = null;
}

/** The language button. A running session keeps going: what was heard so far
 * is confirmed in the old language, and the engine reopens in the new one. */
async function switchDictationLanguage() {
  const locale = cycleDictationLocale();
  renderDictationButton();
  if (!dictationActive() || !dictationState.engine) return;
  dictationConfirmProvisional();
  dictationState.locale = locale;
  dictationState.restarts = 0;
  dictationNotice(`Dictating in ${dictationLanguage(locale).name}`);
  if (dictationState.engine === "web") {
    dropWebDictation();
    if (!startWebDictation()) onDictationError("the browser could not switch language");
    return;
  }
  // dictation_stop also fires the status listener's reopen; both starts carry
  // the new locale, and whichever lands second finds the helper running.
  try {
    await invoke("dictation_stop");
    if (dictationState.want && dictationState.engine === "native") await startNativeDictation(locale);
  } catch (err) {
    onDictationError(String(err));
  }
}

/** What a shared session carries, cut to the hub's bounds. The rating comes from the badge. */
function dictationRecord(session, engine) {
  const text = (value) => value.slice(0, DICTATION_FEEDBACK_CHARS);
  return {
    raw: text(session.raw.join("\n")),
    cleaned: text(session.cleaned.join("\n")),
    final_text: text(prompt.value),
    locale: dictationState.locale,
    engine,
    duration_ms: Date.now() - session.startedAt,
  };
}

/** The badge belongs to the thread the dictation ended in; leaving that thread drops it. */
function offerDictationFeedback(record) {
  dictationState.feedback = { step: "rate", record, rating: null, thread: composer.dataset.tab, sent: false, error: "" };
  renderDictationFeedback();
}

function dropDictationFeedback() {
  dictationState.feedback = null;
  renderDictationFeedback();
}

/** A send ends a session. Its badge is about the message going out, so it stays until the next send. */
function dictationOnSend() {
  if (dictationActive()) stopDictation();
  const feedback = dictationState.feedback;
  if (feedback?.sent) dropDictationFeedback();
  else if (feedback) feedback.sent = true;
}

const DICTATION_FEEDBACK_STEPS = {
  rate: () => `<span>How was this dictation?</span><button type="button" data-rating="good">Good</button><button type="button" data-rating="bad">Bad</button>`,
  consent: () => `<span>Save this dictation's text to help improve dictation?</span><button type="button" data-save="yes">Save</button><button type="button" data-save="no">Don't save</button>`,
  saving: () => `<span>Saving…</span>`,
  saved: () => `<span>Saved. Thank you.</span>`,
  failed: ({ error }) => `<span>The dictation wasn't saved: ${esc(error)}</span>`,
};

function renderDictationFeedback() {
  const badge = document.getElementById("dictation-feedback");
  if (!badge) return;
  const feedback = dictationState.feedback;
  badge.hidden = !feedback;
  badge.innerHTML = feedback ? DICTATION_FEEDBACK_STEPS[feedback.step](feedback) : "";
}

async function saveDictationFeedback(feedback) {
  feedback.step = "saving";
  renderDictationFeedback();
  try {
    const app_version = await (TAURI ? TAURI.app.getVersion() : fetch("/api/version").then((r) => r.text()));
    const record = { ...feedback.record, rating: feedback.rating, app_version: app_version || "unknown", platform: dictationPlatform() };
    if (TAURI) await invoke("dictation_feedback", { feedback: record });
    else {
      const res = await fetch("/api/dictation/feedback", {
        method: "POST",
        headers: { "content-type": "application/json", authorization: `Bearer ${account.user_token}` },
        body: JSON.stringify(record),
      });
      if (!res.ok) throw (await res.json().catch(() => ({}))).error ?? res.statusText;
    }
    feedback.step = "saved";
  } catch (err) {
    Object.assign(feedback, { step: "failed", error: String(err) });
  }
  if (dictationState.feedback !== feedback) return;
  renderDictationFeedback();
  setTimeout(() => dictationState.feedback === feedback && dropDictationFeedback(), feedback.step === "saved" ? 2500 : 6000);
}

function dictationPlatform() {
  if (!TAURI) return "web";
  return IS_MAC ? "macos" : /Win/.test(navigator.platform) ? "windows" : "linux";
}

document.getElementById("dictation-feedback")?.addEventListener("click", (e) => {
  const feedback = dictationState.feedback;
  const button = e.target.closest("button");
  if (!feedback || !button) return;
  if (button.dataset.rating) {
    Object.assign(feedback, { step: "consent", rating: button.dataset.rating });
    if (dictationConsent) saveDictationFeedback(feedback);
    else renderDictationFeedback();
  } else if (button.dataset.save === "yes") {
    rememberDictationConsent();
    saveDictationFeedback(feedback);
  } else if (button.dataset.save === "no") dropDictationFeedback();
});

/** The mic: the only thing besides sending that ends a session. */
async function toggleDictation() {
  if (dictationActive()) return stopDictation();
  if (dictationState.checking) return;
  dictationState.checking = true;
  try {
    if (!(await dictationAllowed())) return;
  } finally {
    dictationState.checking = false;
  }
  startDictation();
}

function startWebDictation() {
  const Engine = window.SpeechRecognition || window.webkitSpeechRecognition;
  if (!Engine) return false;
  const rec = new Engine();
  rec.continuous = true;
  rec.interimResults = true;
  rec.lang = dictationState.locale;
  rec.onresult = (event) => {
    let interim = "";
    for (let i = event.resultIndex; i < event.results.length; i++) {
      const res = event.results[i];
      if (res.isFinal) onDictationResult(res[0].transcript, true);
      else interim += res[0].transcript;
    }
    if (interim) onDictationResult(interim, false);
  };
  rec.onerror = (event) => {
    if (event.error !== "not-allowed") return;
    onDictationError("microphone blocked by the browser");
    showDictationPermission({ message: webMicBlockedMessage("NotAllowedError") });
  };
  rec.onend = () => {
    // The browser ends on silence; we don't. Reopen while the session is wanted.
    if (!dictationState.want || dictationState.engine !== "web") return;
    if (dictationState.restarts++ > 20) return onDictationError("the browser stopped listening");
    try {
      rec.start();
    } catch {
      setTimeout(() => dictationState.want && dictationState.engine === "web" && startWebDictation(), 300);
    }
  };
  try {
    rec.start();
  } catch {
    return false;
  }
  dictationState.engine = "web";
  dictationState.rec = rec;
  renderDictationButton();
  return true;
}

/** A new thread keeps the session: the mic stays on, the mirrors restart here. */
function dictationRetarget() {
  dictationState.live = null;
  if (dictationState.feedback && dictationState.feedback.thread !== composer.dataset.tab) dropDictationFeedback();
}

document.getElementById("composer-dictate")?.addEventListener("click", toggleDictation);
document.getElementById("composer-dictate-lang")?.addEventListener("click", switchDictationLanguage);
dictationPermissionNote?.querySelector("button").addEventListener("click", (e) => {
  invoke("open_privacy_settings", { permission: e.currentTarget.dataset.settings }).catch((err) => dictationShow(String(err), true));
});
// Back from System Settings: a grant made there shows without a restart.
addEventListener("focus", () => {
  if (TAURI && IS_MAC && dictationPermissionNote && !dictationPermissionNote.hidden) recheckDictationPermission();
});
// A tap of fn alone, which only the app's native side can see.
if (TAURI) listen("dictation://toggle", () => {
  toggleDictation();
  prompt.focus();
});
renderDictationButton();

// Typing confirms the provisional text: what the user touches becomes final.
prompt.addEventListener("input", () => {
  if (dictationState.rendering || !dictationState.want) return;
  dictationState.live = null;
  renderDictationButton();
});
