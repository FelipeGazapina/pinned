/** The last error lines this window saw, for "Report a problem". They stay in memory here and
 * leave only when the person ticks the box in the report dialog and sends. */
const reportErrors = (() => {
  const MAX = 50;
  const LINE = 300;
  const ring = [];
  const stamp = () => new Date().toISOString().slice(11, 19);
  const words = (value) => (value instanceof Error ? value.message || String(value) : typeof value === "string" ? value : safeJson(value));
  function safeJson(value) {
    try {
      return JSON.stringify(value) ?? String(value);
    } catch {
      return String(value);
    }
  }
  const last = (n) => ring.slice(-n);

  /** Keeps one line, trimmed to LINE characters; the oldest line goes when the ring is full. */
  function record(text) {
    const line = String(text).replace(/\s+/g, " ").trim().slice(0, LINE);
    if (!line) return;
    ring.push(`${stamp()} ${line}`);
    if (ring.length > MAX) ring.splice(0, ring.length - MAX);
  }

  window.addEventListener("error", (e) => {
    const file = String(e.filename ?? "").split(/[\\/]/).pop();
    record(`${e.message || "Script error"}${file ? ` (${file}:${e.lineno ?? 0})` : ""}`);
  });
  window.addEventListener("unhandledrejection", (e) => record(`Unhandled rejection: ${words(e.reason)}`));
  const original = console.error;
  console.error = (...args) => {
    try {
      record(args.map(words).join(" "));
    } catch {}
    return original.apply(console, args);
  };

  return { record, last, count: () => ring.length, MAX };
})();
