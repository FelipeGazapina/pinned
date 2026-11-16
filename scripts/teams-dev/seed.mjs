#!/usr/bin/env node
// Fills a fresh dev hub (scripts/teams-dev/dev.sh): three invented accounts, nothing else. Passwords
// are generated and written to $TEAMS_DEV_DIR/accounts.json (mode 600), never printed. It can run
// again: a user already in accounts.json signs in instead, and one the hub does not know signs up.
//
// To seed more (a team, members, a box), append a step to STEPS: each gets { accounts, token(email),
// connect(email) } and runs in order. A step must be safe to run twice.
import crypto from "node:crypto";
import { post, readAccounts, writeAccounts, signIn, connect, ACCOUNTS } from "./hub-client.mjs";

export const USERS = [
  { email: "ana@example.test", role: "team owner" },
  { email: "bruno@example.test", role: "member" },
  { email: "carla@example.test", role: "member" },
];

async function users(ctx) {
  for (const { email, role } of USERS) {
    const known = ctx.accounts[email];
    if (known) {
      try {
        await post("/api/login", { email, password: known.password });
        continue;
      } catch (e) {
        if (e.status !== 401 && e.status !== 404 && e.status !== 400) throw e;
      }
    }
    const password = crypto.randomBytes(18).toString("base64url");
    await post("/api/signup", { email, password });
    ctx.accounts[email] = { password, role };
    writeAccounts(ctx.accounts);
    console.log(`signed up ${email}`);
  }
}

// Teams steps go here, after `users`.
export const STEPS = [users];

if (import.meta.url === `file://${process.argv[1]}`) {
  if (!ACCOUNTS) {
    console.error("seed: TEAMS_DEV_DIR is not set");
    process.exit(2);
  }
  const ctx = { accounts: readAccounts(), token: signIn, connect: async (email) => connect(await signIn(email)) };
  for (const step of STEPS) await step(ctx);
  console.log(`seeded ${USERS.length} accounts; passwords are in ${ACCOUNTS}`);
}
