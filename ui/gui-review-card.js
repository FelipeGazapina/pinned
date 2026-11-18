/** Review rounds: `ThreadPage.reviews`, kept on the thread and drawn as a card per round in the worker's
 * thread and as one line per reviewed child in the Captain's. A round comes from pinned's record, never
 * from the thread's text, and every string in it goes through `esc`. */

const REVIEW_STATES = {
  passed: "Passed",
  sent: "Sent back",
  limit: "Limit reached",
  nothing_to_review: "Nothing to review",
  found: "Found · waiting for the Captain",
};

/** Keeps a page's review rounds on the thread, and answers whether that changes what is drawn. */
function takeReviews(t, reviews = []) {
  t.reviews = Array.isArray(reviews) ? reviews : [];
  const key = JSON.stringify(t.reviews);
  if (t.reviewsKey === key) return false;
  t.reviewsKey = key;
  return true;
}

const reviewKnown = (round) => Object.hasOwn(REVIEW_STATES, round.state) || round.state === "not_reviewed";
/** The slug of a state pinned knows, else "unknown": the only part of a state that reaches an attribute. */
const reviewSlug = (round) => (reviewKnown(round) ? round.state : "unknown");
/** What a round's state reads as, in plain text for `esc`. Anything pinned does not know is no pass. */
function reviewLabel(round) {
  if (round.state === "not_reviewed") return `Not reviewed${round.note ? ` · ${round.note}` : ""}`;
  return Object.hasOwn(REVIEW_STATES, round.state) ? REVIEW_STATES[round.state] : `Review: ${round.state}`;
}

const reviewFindingHtml = (f) => {
  const lines = Array.isArray(f.lines) && f.lines.length ? `:${f.lines.join(",")}` : "";
  return `<li class="review-finding"><b class="review-rule">${esc(f.rule)}</b> <code class="review-file">${esc(`${f.file}${lines}`)}</code> <span class="review-what">${esc(f.what)}</span></li>`;
};

function reviewCardHtml(round) {
  const findings = Array.isArray(round.findings) ? round.findings : [];
  const note = round.note && round.state !== "not_reviewed" ? `<span class="review-note">${esc(round.note)}</span>` : "";
  const model = round.model ? `<span class="review-model">${esc(round.model)}</span>` : "";
  const meta = model || note ? `<div class="review-meta">${model}${note}</div>` : "";
  const list = findings.length ? `<ul class="review-findings">${findings.map(reviewFindingHtml).join("")}</ul>` : "";
  return `<section class="review-card" data-review-round="${esc(round.round)}" data-review-state="${reviewSlug(round)}"><div class="review-head"><span class="review-title">Reviewer · round ${esc(round.round)}</span><span class="review-state">${esc(reviewLabel(round))}</span></div>${meta}${list}</section>`;
}

const reviewRounds = (entry) => [...(Array.isArray(entry.reviews) ? entry.reviews : [])].sort((a, b) => a.round - b.round);

/** The Captain's one line for a reviewed child task, from its latest round. */
function reviewLineHtml(tab, entry) {
  const last = reviewRounds(entry).at(-1);
  if (!last) return "";
  const n = Array.isArray(last.findings) ? last.findings.length : 0;
  const label = reviewLabel(last);
  const plain = ["not_reviewed", "nothing_to_review"].includes(last.state) || !reviewKnown(last);
  const said = plain ? label : `Reviewed · ${n} ${n === 1 ? "finding" : "findings"} · ${label.toLowerCase()}`;
  const open = `<button type="button" class="outline" data-worker-open="${esc(`${tab.machine}:${entry.tab}`)}">Open thread</button>`;
  const ask = last.state === "limit" ? `<div class="review-ask" role="status">The review limit was reached on ${esc(entry.name)} with findings left. Stop and ask the user before sending it another round.</div>` : "";
  return `<div class="review-line" data-review-task="${esc(entry.task)}" data-review-state="${reviewSlug(last)}"><b class="review-name">${esc(entry.name)}</b><span class="review-said">${esc(said)}</span>${open}</div>${ask}`;
}

/** The cards of the tab's own worker rounds, then the lines of the children its thread started. */
function reviewsHtml(tab, t) {
  return (t.reviews ?? [])
    .map((entry) => (entry.tab ? reviewLineHtml(tab, entry) : reviewRounds(entry).map(reviewCardHtml).join("")))
    .join("");
}
