#!/usr/bin/env node
// The hub-facing steps of e2e-empty.sh, run against a stack dev.sh up started. Prints emails and
// Slack method names only: never a password, token or key.
//
//   TEAMS_DEV_DIR=<dir> node e2e-steps.mjs signin | slack | reports | empty <old-accounts.json>
import crypto from "node:crypto";
import fs from "node:fs";
import { connect, signIn, post, readAccounts } from "./hub-client.mjs";

const SLACK = process.env.TEAMS_DEV_SLACK || "http://127.0.0.1:8813";
const EMAILS = ["ana@example.test", "bruno@example.test", "carla@example.test"];
const rand = () => crypto.randomBytes(6).toString("hex");
const sink = async () => (await fetch(`${SLACK}/sink`)).json();
const must = (ok, why) => {
  if (!ok) throw new Error(why);
};

const steps = {
  // The three seeded users sign in.
  async signin() {
    for (const email of EMAILS) {
      await signIn(email);
      console.log(`signed in ${email}`);
    }
  },

  // A Slack token the fake knows goes through the hub to the fake; one it does not is refused.
  async slack() {
    await fetch(`${SLACK}/sink/reset`, { method: "POST" });
    const hub = await connect(await signIn("ana@example.test"));
    try {
      const connected = await hub.call({ cmd: "connect_slack", token: `xoxb-dev-${rand()}` });
      must(connected?.bot?.team === "Pinned Dev", "connect_slack did not answer with the fake's workspace");
      const methods = (await sink()).filter((e) => e.ok).map((e) => e.method);
      must(methods.includes("auth.test"), `the fake Slack saw no auth.test (saw: ${methods.join(", ") || "nothing"})`);
      console.log(`the fake Slack saw ${[...new Set(methods)].join(", ")}`);
      const before = (await sink()).length;
      const refused = await hub.call({ cmd: "connect_slack", token: `xoxb-real-${rand()}` }).then(() => null, (e) => e.message);
      must(refused && /invalid_auth/.test(refused), `a real-looking token was not refused with invalid_auth (got: ${refused})`);
      const after = (await sink()).slice(before);
      must(after.length > 0 && after.every((e) => !e.ok && e.error === "invalid_auth"), "the fake did not answer invalid_auth");
      console.log("a token that is not xoxb-dev-… was refused by the fake with invalid_auth");
    } finally {
      hub.close();
    }
  },

  // Reporting is on, and a search answers: the hub's token is a fake- placeholder that only the fake GitHub
  // takes, so an answer means the hub asked the fake and not the real GitHub.
  async reports() {
    const hub = await connect(await signIn("ana@example.test"));
    try {
      const status = await hub.call({ cmd: "reports", ask: { op: "status" } });
      must(status?.available === true, `reporting is not available on the dev hub (got: ${JSON.stringify(status)})`);
      const found = await hub.call({ cmd: "reports", ask: { op: "similar", title: "Upload stops at 90%" } });
      must(Array.isArray(found), `the search did not answer a list (got: ${JSON.stringify(found)})`);
      console.log(`reports are available and a search answered ${found.length} issues from the fake GitHub`);
    } finally {
      hub.close();
    }
  },

  // After a reset: the same three accounts with new passwords, no box, no Slack, no Upstash.
  async empty(oldFile) {
    const old = JSON.parse(fs.readFileSync(oldFile, "utf8"));
    const now = readAccounts();
    must(JSON.stringify(Object.keys(now).sort()) === JSON.stringify(EMAILS), "accounts.json does not hold exactly the three seeded accounts");
    for (const email of EMAILS) {
      const stale = await post("/api/login", { email, password: old[email].password }).then(() => true, () => false);
      must(!stale, `${email} still signs in with its password from before the reset`);
      const hub = await connect(await signIn(email));
      try {
        const settings = await hub.call({ cmd: "get_settings" });
        must(!settings.slack && !(settings.slack_bots ?? []).length, `${email} still has Slack connected`);
        must(!settings.upstash, `${email} still has Upstash connected`);
        const machines = await hub.call({ cmd: "machines" });
        must(!machines.length, `${email} still has a machine or box`);
      } finally {
        hub.close();
      }
      console.log(`${email}: no box, no Slack, no Upstash, new password`);
    }
  },
};

const [name, ...rest] = process.argv.slice(2);
if (!steps[name]) {
  console.error("usage: e2e-steps.mjs signin | slack | reports | empty <old-accounts.json>");
  process.exit(2);
}
try {
  await steps[name](...rest);
} catch (e) {
  console.error(e.message);
  process.exit(1);
}
