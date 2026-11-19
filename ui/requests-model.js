// What the Requests screens draw about the global Orchestrator's requests, derived once from the
// glances the host sends, so no screen re-derives it. Pure: no DOM and no wire. Times are unix
// milliseconds. A request is a thread with its own Orchestrator tab, and one `RequestGlance` is
//   { id, title, project|null, opened_at, kind, closed_at|null, needs, needs_at|null,
//     last_user_at|null, last: { who, text, at }|null, replies, captains: [{ machine, path,
//     project, machine_name }], tab|null }.
// Loaded after composer-tags.js, whose paragraph and #tag parsing (`composerParagraphs`,
// `paragraphTags`, `routeParagraph`, `tagSlug`, `TAG_WORD`) it reuses.
//
// requestState(glance, { planGlances = [], tabWaits = false } = {}) -> "done" | "needs" | "working"
//   One state per request, first match wins. done: `closed_at` is set. needs: `needs` is set and
//   not stale, or any of `planGlances` (the request's plans) is `needs_you`, or `tabWaits` (its
//   own tab waits on input). A need is stale when the user wrote after it (`last_user_at >
//   needs_at`); without a `needs_at` it is never stale. Otherwise working.
//
// requestCounts(glances, ctx) -> { all, needs, working, done }
//   `ctx(glance)` returns requestState's second argument for that glance (optional).
//
// requestFilter(glances, filter, ctx) -> glance[]
//   `filter` is "all" | "needs" | "working" | "done". Sorted needs, then working, then done; in
//   each group the newest activity first (`last.at`, else `opened_at`). A new array.
//
// splitRequest(text, catalog) -> [{ title, text, project, machine }]
//   One item per paragraph that carries a #project tag; an untagged paragraph joins the item
//   above it (one before the first tag is an item of its own). `text` joins the paragraphs with a
//   blank line. `title` is the item's first sentence without its tags, at most 60 characters with
//   the "…" counted, cut at a word. `project` and `machine` are the names `routeParagraph` routed
//   the first paragraph to, null when it is untagged, unknown or ambiguous. `catalog` is
//   `tagCatalog`'s result.
//
// closeTo(paragraph, openGlances) -> glance | null
//   The open request a paragraph most looks like, or null. Only a request of the same project
//   (the paragraph's #tag against `tagSlug` of the glance's project) and not closed counts. Words
//   are lower-cased, accent-folded and kept at 4 letters or more; the score is the Jaccard index
//   of the paragraph's words (tags left out) and those of the glance's `title` plus `last.text`.
//   The best score at or above CLOSE_TO wins, the first of equal scores. A paragraph with no
//   #project never matches.
//
// requestPlans(glance, tabsByMachine) -> [{ machine, tab, plan }]
//   Every plan glance whose `requests` holds the glance's id, and every plan on the request's own
//   tab (`tab.request === glance.id`). One entry per (machine, tab.id, plan.id). Order: the own
//   tab, then each of `glance.captains` in order (a tab on that machine whose `cwd` is the
//   Captain's `path`), then any other tab, and by machine, tab id and plan id inside a group.
//
// requestProgress(plans) -> { done, total }
//   The steps done and in all, summed over the plans. Takes requestPlans' entries or bare plan
//   glances. Zeros with no plans: "7 of 8 steps" is drawn from it.
//
// unread(glance, seen) -> number
//   The replies the user has not seen: max(0, glance.replies - (seen ?? 0)).

/** How alike a paragraph and an open request must be for the composer to offer the request. */
const CLOSE_TO = 0.4;

const REQUEST_ORDER = { needs: 0, working: 1, done: 2 };
const REQUEST_TITLE_MAX = 60;

function requestState(glance, { planGlances = [], tabWaits = false } = {}) {
  if (glance.closed_at != null) return "done";
  const stale = glance.needs_at != null && glance.last_user_at != null && glance.last_user_at > glance.needs_at;
  if ((glance.needs && !stale) || planGlances.some((p) => p.state === "needs_you") || tabWaits) return "needs";
  return "working";
}

function requestActivity(glance) {
  return glance.last?.at ?? glance.opened_at;
}

function requestCounts(glances, ctx) {
  const counts = { all: 0, needs: 0, working: 0, done: 0 };
  for (const glance of glances) {
    counts.all++;
    counts[requestState(glance, ctx?.(glance))]++;
  }
  return counts;
}

function requestFilter(glances, filter, ctx) {
  return glances
    .map((glance) => ({ glance, state: requestState(glance, ctx?.(glance)) }))
    .filter(({ state }) => filter === "all" || state === filter)
    .sort((a, b) => REQUEST_ORDER[a.state] - REQUEST_ORDER[b.state] || requestActivity(b.glance) - requestActivity(a.glance))
    .map(({ glance }) => glance);
}

/** The first sentence of `text` with its tags removed, cut at a word to REQUEST_TITLE_MAX. */
function requestTitle(text) {
  const plain = String(text).replace(TAG_WORD, " ").replace(/[ \t]+/g, " ");
  const line = plain.split("\n").map((l) => l.trim()).find(Boolean) ?? "";
  const sentence = (line.match(/^.*?[.!?](?=\s|$)/)?.[0] ?? line).replace(/\.$/, "").trim();
  if (sentence.length <= REQUEST_TITLE_MAX) return sentence;
  const room = sentence.slice(0, REQUEST_TITLE_MAX - 1);
  const atWord = sentence.charAt(REQUEST_TITLE_MAX - 1) === " " ? room : room.replace(/\s+\S*$/, "");
  return `${(atWord || room).trimEnd()}…`;
}

function splitRequest(text, catalog) {
  const items = [];
  for (const paragraph of composerParagraphs(text)) {
    const body = paragraph.text.trim();
    const tagged = paragraphTags(body).project !== "";
    if (!tagged && items.length) {
      items.at(-1).paragraphs.push(body);
      continue;
    }
    const route = routeParagraph(body, catalog);
    const routed = route.kind === "routed";
    items.push({ paragraphs: [body], project: routed ? route.project : null, machine: routed ? route.machine : null });
  }
  return items.map(({ paragraphs, project, machine }) => {
    const joined = paragraphs.join("\n\n");
    return { title: requestTitle(joined), text: joined, project, machine };
  });
}

/** The set of words of 4 letters or more, lower-cased and accent-folded. */
function requestWords(text) {
  const folded = String(text ?? "").normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
  return new Set(folded.split(/[^a-z0-9]+/).filter((word) => word.length >= 4));
}

function closeTo(paragraph, openGlances) {
  const slug = paragraphTags(paragraph).project;
  if (!slug) return null;
  const mine = requestWords(String(paragraph).replace(TAG_WORD, " "));
  let best = null;
  let bestScore = 0;
  for (const glance of openGlances) {
    if (glance.closed_at != null || !glance.project || tagSlug(glance.project) !== slug) continue;
    const theirs = requestWords(`${glance.title} ${glance.last?.text ?? ""}`);
    let shared = 0;
    for (const word of mine) if (theirs.has(word)) shared++;
    const union = mine.size + theirs.size - shared;
    const score = union ? shared / union : 0;
    if (score >= CLOSE_TO && score > bestScore) {
      best = glance;
      bestScore = score;
    }
  }
  return best;
}

function requestPlans(glance, tabsByMachine = {}) {
  const rows = [];
  const seen = new Set();
  for (const [machine, tabs] of Object.entries(tabsByMachine ?? {})) {
    for (const tab of tabs ?? []) {
      const own = tab.request === glance.id;
      const captain = glance.captains.findIndex((c) => c.machine === machine && c.path === tab.cwd);
      for (const plan of tab.plans ?? []) {
        const key = `${machine}\n${tab.id}\n${plan.id}`;
        if (seen.has(key) || !(own || (plan.requests ?? []).includes(glance.id))) continue;
        seen.add(key);
        const rank = own ? 0 : captain >= 0 ? 1 + captain : 1 + glance.captains.length;
        rows.push({ rank, machine, tab, plan });
      }
    }
  }
  const order = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
  rows.sort((a, b) => a.rank - b.rank || order(a.machine, b.machine) || order(a.tab.id, b.tab.id) || order(a.plan.id, b.plan.id));
  return rows.map(({ machine, tab, plan }) => ({ machine, tab, plan }));
}

function requestProgress(plans = []) {
  let done = 0;
  let total = 0;
  for (const entry of plans) {
    const plan = entry.plan ?? entry;
    done += plan.done;
    total += plan.total;
  }
  return { done, total };
}

function unread(glance, seen) {
  return Math.max(0, glance.replies - (seen ?? 0));
}
