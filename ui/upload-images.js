const LIMIT = 40;

const messageOf = (err) => (typeof err === "string" ? err : err?.message || String(err ?? ""));

const missingFile = (err) => {
  const message = messageOf(err);
  return message.startsWith("could not read ")
    || message.endsWith(" is not an upload")
    || message.endsWith(" is not a picture")
    || / is over \d+ MB$/.test(message)
    || message === "this machine keeps no files";
};

export function createUploadImages() {
  const cache = new Map();
  const waiting = new Set();
  let generation = 0;

  const store = (key, value) => {
    if (!cache.has(key) && cache.size >= LIMIT) cache.delete(cache.keys().next().value);
    cache.set(key, value);
  };

  return {
    pull(key, read) {
      if (waiting.has(key)) return Promise.resolve("wait");
      const hit = cache.get(key);
      if (hit) return hit;
      const started = generation;
      const pending = Promise.resolve()
        .then(read)
        .then((src) => {
          const kept = src ? src : "drop";
          store(key, Promise.resolve(kept));
          return kept;
        })
        .catch((err) => {
          cache.delete(key);
          if (missingFile(err)) {
            store(key, Promise.resolve("drop"));
            return "drop";
          }
          if (started !== generation) return "retry";
          waiting.add(key);
          return "wait";
        });
      store(key, pending);
      return pending;
    },
    reconnect() {
      generation += 1;
      const keys = [...waiting];
      waiting.clear();
      return keys;
    },
  };
}
