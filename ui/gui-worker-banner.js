/** "Worker waiting." over the composer of a thread whose worker threads wait on the user: the oldest
 * ask first, a "1 of N" that steps through the rest, Decline and Approve where one tap answers it,
 * and Open, which opens the worker. Answering leaves the open thread's draft and focus alone. */
const workerBanner = Object.assign(document.createElement("div"), { id: "worker-banner", hidden: true });
workerBanner.setAttribute("role", "status");
document.getElementById("limit-note").before(workerBanner);
let workerShown = null;
const workerAnswering = new Set();

/** The parent's waiting workers, the one that has waited longest first. */
const waitingWorkers = (tab) => workersOf(tab).filter((r) => r.ask && r.tab).sort((a, b) => (a.since ?? Infinity) - (b.since ?? Infinity));

const ASK_WORDS = { approval: "needs your approval", question: "has a question", plan: "has a plan to approve" };
/** The banner for `row`, the `at`th of `count` waiting workers. */
function workerBannerHtml(row, at, count, busy) {
  const ask = row.ask;
  const command = ask.kind === "approval" && ask.command ? ask.command.split("\n")[0].trim() : "";
  const offers = (choice) => !!ask.choices?.some((c) => c.choice === choice);
  const off = busy ? " disabled" : "";
  return `${icon("shield", "lead")}<span class="text"><span class="tone">Worker waiting.</span><span class="who">${esc(row.title)}</span><span class="what">${ASK_WORDS[ask.kind] ?? ASK_WORDS.question}${command ? " to run" : ""}</span>${command ? `<span class="cmd">${esc(command)}</span>` : ""}</span>
    ${count > 1 ? `<button type="button" class="step" data-worker-step aria-label="Show the next waiting worker">${at + 1} of ${count}</button>` : ""}${offers("decline") ? `<button type="button" class="outline" data-worker-answer="decline"${off}>Decline</button>` : ""}${offers("approve") ? `<button type="button" class="primary" data-worker-answer="approve"${off}>Approve</button>` : ""}<button type="button" class="ghost" data-worker-open>Open</button>`;
}

function renderWorkerBanner() {
  const tab = active;
  const rows = tab && guiOn() && !terminalOnly(tab) ? waitingWorkers(tab) : [];
  const at = Math.max(0, rows.findIndex((r) => r.key === workerShown));
  const row = rows[at];
  workerBanner.hidden = !row;
  composer.dataset.workerWaiting = !!row;
  const html = row ? workerBannerHtml(row, at, rows.length, workerAnswering.has(row.key)) : "";
  workerBanner.dataset.worker = row?.key ?? "";
  setHtml(workerBanner, html);
}
onModelChange(renderWorkerBanner);

// A press keeps the focus where it was, in the prompt most of all.
workerBanner.addEventListener("pointerdown", (e) => e.target.closest("button") && e.preventDefault());
workerBanner.addEventListener("click", async (e) => {
  const rows = active ? waitingWorkers(active) : [];
  const row = rows.find((r) => r.key === workerBanner.dataset.worker);
  if (!row) return;
  if (e.target.closest("[data-worker-step]")) {
    workerShown = rows[(rows.indexOf(row) + 1) % rows.length].key;
    return renderWorkerBanner();
  }
  if (e.target.closest("[data-worker-open]")) return activate(row.tab);
  const answer = e.target.closest("[data-worker-answer]");
  if (!answer || workerAnswering.has(row.key)) return;
  workerAnswering.add(row.key);
  renderWorkerBanner();
  try {
    if (!(await answerAsk(row.tab, answer.dataset.workerAnswer))) showComposerNote(`${row.title} changed its question. Open it to answer.`, true);
  } catch (err) {
    showComposerNote(String(err), true);
  } finally {
    workerAnswering.delete(row.key);
    renderWorkerBanner();
  }
});
