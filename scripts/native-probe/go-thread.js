(async () => {
  const base = "http://127.0.0.1:__PORT__";
  const report = (r) => fetch(`${base}/report`, { method: "POST", body: JSON.stringify(r) });
  const wanted = "__VARIANTS__".split(",").filter(Boolean);
  try {
    const added = await callOn(LOCAL, { cmd: "add_account", provider: "claude", name: "Go", auth: "opencode-go", key: "oc_sk_sweep" });
    setAccounts(LOCAL, [...(machines.get(LOCAL)?.accounts ?? []).filter((a) => a.id !== added.id), added]);
    for (const v of GO_THREAD_VARIANTS.filter((v) => !wanted.length || wanted.includes(v.variant))) {
      await report(await goThread({ ...v, machine: LOCAL, account: added.id, folder: "__FOLDER__", mock: "http://127.0.0.1:__GO_PORT__" }));
    }
  } catch (e) {
    await report({ variant: "setup", ok: false, failures: [String(e?.stack ?? e)], seen: {} });
  }
  fetch(`${base}/done`, { method: "POST" });
})();
