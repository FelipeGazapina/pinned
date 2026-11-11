// Worker-Approval.dc.html: the worker "Tidy the README" open, waiting for approval to run
// git push origin readme-tidy. The live side opens that worker's thread: the "Worker of" strip, the
// Captain's brief, the turn, and the approval in place of the composer. The board's two changed
// files (README.md +12 −29, docs/install.md +6 −2) are written as a turn of the host machine's
// project history, the record a finished turn leaves, since the fake worker edits no file.
const fs = require("fs");
const path = require("path");

const FILES = [
  { path: "README.md", added: 12, removed: 29 },
  { path: "docs/install.md", added: 6, removed: 2 },
];

/** The history file machine.rs keeps a project's turns in: history::file_for, FNV-1a of the path. */
function historyFile(dir, project) {
  let hash = 0xcbf29ce484222325n;
  for (const byte of Buffer.from(project)) hash = BigInt.asUintN(64, (hash ^ BigInt(byte)) * 0x100000001b3n);
  return path.join(dir, "node", "history", `${path.basename(project)}-${hash.toString(16).padStart(16, "0")}.jsonl`);
}

const patchOf = (files) =>
  files.map((f) => [
    `diff --git a/${f.path} b/${f.path}`, `--- a/${f.path}`, `+++ b/${f.path}`, `@@ -1,${f.removed} +1,${f.added} @@`,
    ...Array.from({ length: f.removed }, (_, i) => `-old line ${i + 1}`), ...Array.from({ length: f.added }, (_, i) => `+new line ${i + 1}`),
  ].join("\n")).join("\n") + "\n";

module.exports = {
  board: "Worker-Approval",
  viewport: { width: 1440, height: 900 },
  regions: [
    { name: "sidebar", x: 0, y: 0, width: 264, height: 900 },
    { name: "header", x: 264, y: 0, width: 1176, height: 49 },
    { name: "timeline", x: 264, y: 49, width: 1176, height: 589 },
    { name: "approval", x: 264, y: 638, width: 1176, height: 262 },
  ],
  async seed(page, ctx) {
    const key = ctx.threads.readme;
    const { session, project } = await ctx.waitFor("the README worker's session and project", () => page.evaluate((k) => {
      const t = threadOf(tabByKey(k));
      return t.session && t.workspace?.project ? { session: t.session, project: t.workspace.project } : null;
    }, key));
    const file = historyFile(ctx.dir, project);
    const lines = fs.existsSync(file) ? fs.readFileSync(file, "utf8").split("\n").filter(Boolean).map((l) => JSON.parse(l)) : [];
    if (!lines.some((t) => t.session === session)) {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      const turn = { session, title: "Tidy the README", prompt: "Tidy the README", ended: Math.floor(Date.now() / 1000), files: FILES, patch: patchOf(FILES), truncated: false };
      fs.appendFileSync(file, `${JSON.stringify(turn)}\n`);
    }
    await page.evaluate((k) => Object.assign(threadOf(tabByKey(k)), { history: null, gitDiff: null }), key);
  },
  async open(page, ctx) {
    await page.evaluate((key) => activate(tabByKey(key)), ctx.threads.readme);
    await require("./main").pickEditor(page, ctx, "Zed");
    await ctx.waitFor("the worker's changed files", () => page.evaluate(() => !!document.querySelector("#thread .changes")), 15000).catch((e) => ctx.warn(e.message));
  },
};
