/** What the app says when a link is down: which machines to name as offline, and what the sign-in form
 * says when the hub can't be reached. */

/** The machines to name in the sidebar as offline, by name. `entries` is the `machines` map's entries;
 * `threadsSeen` says per machine id whether it had threads when it was last online (absent when
 * nothing is known, and then it is named). `seen` turns a last-seen time into "seen 1 min ago". */
function offlineMachines(entries, { local, threadsSeen, seen, name }) {
  return [...entries]
    .filter(([id, m]) => id !== local && m.view?.online === false && threadsSeen[id] !== false)
    .map(([id, m]) => ({ id, name: name(id), seen: m.view.last_seen ? seen(m.view.last_seen) : "" }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

/** The line a banner shows for one offline machine: "mini offline · seen 1 min ago". */
const offlineLine = (m) => `${m.name} offline${m.seen ? ` · ${m.seen}` : ""}`;

/** What the sidebar's empty list says: machines being offline is not "no threads". */
const emptyListText = (offline) => (offline ? "No threads to show while machines are offline" : "No threads yet");

/** Fetch's and the app's words for "the request never got an answer", from Chrome, Safari, Firefox and
 * the app's HTTP client. */
const NO_ANSWER = /failed to fetch|load failed|networkerror|network request failed|error sending request|connection refused|connection reset|dns error|tcp connect|timed out|unreachable/i;

function hubHost(hub) {
  const address = String(hub ?? "").trim();
  try {
    return new URL(address).host || address;
  } catch {
    return address;
  }
}

/** What the sign-in form says for an error: a plain sentence when the hub couldn't be reached, else the
 * error as the hub said it. `hub` is the hub's address. */
function signInProblem(error, hub) {
  const text = String(error);
  if (!NO_ANSWER.test(text)) return text;
  const host = hubHost(hub);
  return `Can't reach the hub${host ? ` at ${host}` : ""}. Check your connection and try again.`;
}

function hubOutOfReach(hub, computer) {
  const host = hubHost(hub);
  return `Can't reach the hub${host ? ` at ${host}` : ""}. ${computer ? `This ${computer} keeps working and reconnects` : "This page reconnects"} once the hub answers.`;
}

const HUB_NEEDED = "Can't reach the hub, and this needs it.";

/** Whether `error` is the kind `signInProblem` words as "can't reach", so the form offers Retry. */
const hubUnreachable = (error) => NO_ANSWER.test(String(error));
