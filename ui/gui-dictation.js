/** Composer dictation: hold-to-talk is out, talk-until-done is in.
 *
 * One tap on the mic starts a session that only ends when the message is sent
 * or the mic is tapped again — never on a pause. While it runs:
 * - interim results show as provisional text after the caret area,
 * - each final result confirms one phrase into the prompt,
 * - typing or editing confirms the provisional text on the spot.
 *
 * The transcript is cleaned with the same ideas as the speech-to-text app:
 * a developer glossary steers spelling (kubectl, useState, async/await…),
 * filler sounds are dropped, and spoken code shapes ("snake case X",
 * "utils dot ts") become written code. The heavy LLM rewrite pass the other
 * app does stays out of 0.17.2; the deterministic layer below is what ships.
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
  confirmed: "", // frozen text: the prompt at start + every confirmed phrase
  provisional: "", // the live interim text, not confirmed yet
  rendering: false, // our own prompt.value writes, not the user's typing
  rec: null, // web SpeechRecognition instance
  locale: null, // what the running engine listens in; cleanup follows it, not the button
  noted: false, // a dictation note is up and the next transcript clears it
  restarts: 0,
  unlisten: [],
};

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

function renderDictationButton() {
  const button = document.getElementById("composer-dictate");
  if (!button) return;
  button.dataset.active = dictationActive();
  button.setAttribute("aria-pressed", dictationActive());
  button.title = dictationActive() ? "Stop dictation" : "Dictate with your voice";
  const lang = document.getElementById("composer-dictate-lang");
  if (lang) {
    const { label, name } = dictationLanguage();
    lang.textContent = label;
    lang.title = `Dictation language: ${name}. Click to switch.`;
    lang.setAttribute("aria-label", lang.title);
  }
  composer.dataset.dictating = dictationActive();
  composer.dataset.provisional = !!(dictationActive() && dictationState.provisional);
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

/** Displayed value = confirmed text + live provisional tail. */
function renderDictation() {
  const tail = dictationState.provisional ? (dictationState.confirmed && !dictationState.confirmed.endsWith(" ") && !dictationState.confirmed.endsWith("\n") ? " " : "") + dictationState.provisional : "";
  dictationState.rendering = true;
  prompt.value = dictationState.confirmed + tail;
  dictationState.rendering = false;
  autosize();
  if (active) saveDraft(active, prompt.value);
  renderDictationButton();
}

function dictationConfirmProvisional() {
  if (!dictationState.provisional) return;
  const cleaned = dictationCleanup(dictationState.provisional, dictationState.locale);
  dictationState.confirmed = joinDictation(dictationState.confirmed, cleaned);
  dictationState.provisional = "";
}

function joinDictation(base, addition) {
  if (!addition) return base;
  if (!base) return addition;
  if (base.endsWith(" ") || base.endsWith("\n") || /^[.,!?;:)\]}]/.test(addition)) return base + addition;
  return `${base} ${addition}`;
}

/** One transcript from either engine. Interim replaces the tail; final
 * confirms a phrase and clears the tail for the next one. */
function onDictationResult(text, isFinal) {
  if (!dictationState.want) return;
  dictationClearNotice();
  if (isFinal) {
    dictationState.restarts = 0;
    const cleaned = dictationCleanup(text, dictationState.locale);
    dictationState.confirmed = joinDictation(dictationState.confirmed, cleaned);
    dictationState.provisional = "";
    // A new phrase starts: what came before is confirmed, period.
    renderDictation();
  } else {
    dictationState.provisional = text.trim();
    renderDictation();
  }
}

function onDictationError(message) {
  if (!dictationState.want) return;
  stopDictation();
  dictationShow(`Dictation stopped: ${message}`, true);
}

async function startDictation() {
  if (dictationState.want) return;
  dictationState.want = true;
  dictationState.confirmed = prompt.value;
  dictationState.provisional = "";
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
      await invoke("dictation_start", { locale: dictationState.locale });
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
  invoke("dictation_start", { locale: dictationState.locale }).catch((err) => onDictationError(String(err)));
}

function stopDictation() {
  if (!dictationState.want && !dictationState.engine) return;
  dictationState.want = false;
  // Whatever is on screen is what the user keeps.
  dictationState.confirmed = prompt.value;
  dictationState.provisional = "";
  dictationClearNotice();
  if (dictationState.engine === "native" && TAURI) invoke("dictation_stop").catch(() => {});
  dropWebDictation();
  for (const off of dictationState.unlisten.splice(0)) off();
  dictationState.engine = null;
  renderDictationButton();
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
  renderDictation();
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
    if (dictationState.want && dictationState.engine === "native") await invoke("dictation_start", { locale });
  } catch (err) {
    onDictationError(String(err));
  }
}

/** The mic: the only thing besides sending that ends a session. */
function toggleDictation() {
  if (dictationActive()) stopDictation();
  else startDictation();
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
    if (event.error === "not-allowed") onDictationError("microphone blocked by the browser");
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
  if (!dictationActive()) return;
  dictationState.confirmed = prompt.value;
  dictationState.provisional = "";
}

document.getElementById("composer-dictate")?.addEventListener("click", toggleDictation);
document.getElementById("composer-dictate-lang")?.addEventListener("click", switchDictationLanguage);
renderDictationButton();

// Typing confirms the provisional text: what the user touches becomes final.
prompt.addEventListener("input", () => {
  if (dictationState.rendering || !dictationState.want) return;
  dictationState.confirmed = prompt.value;
  dictationState.provisional = "";
  renderDictationButton();
});
