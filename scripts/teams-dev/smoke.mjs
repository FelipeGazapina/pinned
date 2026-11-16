#!/usr/bin/env node
// Proves the fake Box works with the hub: ana connects Upstash with made-up keys (the fake takes any
// key starting box_dev-), then, with `open`, opens a box through the hub and waits for its node to
// come online. Run it against a stack that dev.sh up started:
//
//   TEAMS_DEV_DIR=<dir> node scripts/teams-dev/smoke.mjs [open [owner/repo] | disconnect]
//
// `disconnect` ends the account's boxes (the hub asks the fake to delete them) and drops the keys.
// It prints key hints only, never a key, token or password.
import crypto from "node:crypto";
import { connect, signIn, readAccounts } from "./hub-client.mjs";

const email = process.env.SMOKE_EMAIL || "ana@example.test";
if (!readAccounts()[email]) throw new Error(`${email} is not seeded: run dev.sh up first`);
const hex = () => crypto.randomBytes(12).toString("hex");
const hub = await connect(await signIn(email));
try {
  if (process.argv[2] === "disconnect") {
    await hub.call({ cmd: "disconnect_upstash" });
    console.log("disconnect_upstash ok");
    process.exit(0);
  }
  const account = await hub.call({ cmd: "connect_upstash", key: `box_dev-${hex()}`, claude_token: `sk-ant-oat01-DEV-FAKE-${hex()}` });
  console.log("connect_upstash ok:", JSON.stringify(account));
  if (process.argv[2] === "open") {
    // Boxes are off for every kind of actor until the account picks a permission.
    await hub.call({ cmd: "set_box_access", access: { workflow: { mode: "fresh" }, session: { mode: "fresh" }, worker: { mode: "fresh" }, orchestrator: { mode: "fresh" } } });
    const repo = process.argv[3] || "example/alpha";
    const opened = await hub.call({ cmd: "open_box", repo });
    console.log("open_box ok:", JSON.stringify(opened));
    const started = Date.now();
    for (;;) {
      const machines = await hub.call({ cmd: "machines" });
      const box = machines.find((m) => m.id === opened.machine);
      const status = await hub.call({ cmd: "box_status", machine: opened.machine }).catch((e) => ({ error: e.message }));
      console.log(`+${Math.round((Date.now() - started) / 1000)}s online=${box?.online} phase=${status.phase ?? status.error}`);
      if (box?.online || Date.now() - started > 90_000) break;
      await new Promise((r) => setTimeout(r, 2000));
    }
  }
} finally {
  hub.close();
}
