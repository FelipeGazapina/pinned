// Orchestrator.dc.html: the orchestrator's chat on mac-mini with two Captain reports, the Captains
// rail on the right, and the sidebar's account menu open. Today's app has no account menu at the
// sidebar's foot (its pages sit in an icon row), so that region compares against the sidebar.
//
// seed sends the board's tagged request to the orchestrator for real. Its fake claude hands each
// paragraph to that project's Captain with send_to_captain: pinned's Captain is open on mac-mini and
// takes it at once, site's starts on macbook. Each Captain answers with the board's words. site's
// report reaches the orchestrator by itself. pinned's CI worker is still at work, so the node holds
// that report for up to CAPTAIN_HOLD; seed sends it up with the node's own orchestrator_report, as
// the node does when the hold ends. The two reports arrive one after the other, so the chat draws
// two cards where the board draws one card of two. The orchestrator's answer to the last one is the
// board's summary with its quick replies. open leaves the #api draft in the composer.
const fs = require("fs");
const path = require("path");
const { WORLD, homeOf } = require("./world");

const HOST = WORLD.orchestrator.machine;
const SITE = WORLD.projects.find((p) => p.name === "site").machine;
const REQUEST = "#pinned release 0.17.21 once the CI is green.\n\n#site publish v2 with its changelog.";
const TO_PINNED = "Cut the release of v0.17.21 as soon as its CI is green.";
const TO_SITE = "Publish v2 of the site with its changelog.";
const PINNED_REPORT = "The changelog is merged in #231. The CI is on its last suite. One worker waits for approval to push the README branch.";
const SITE_REPORT = "Published v2 and wrote the changelog.";
const SITE_ANSWER = "site is live on v2. Waiting on pinned.";
const ANSWER = "Both Captains reported. site is live on v2. pinned is one approval away from cutting the release.\n\nOptions: Approve the push | Hold the release | Show me the diff first";
const DRAFT = "#api limit the webhook to 60 calls a minute";

/** Adds `turns` to a machine's scripted fake claude turns, once. */
function script(ctx, machine, turns) {
  const file = path.join(homeOf(ctx, machine), "fake-turns.json");
  const now = JSON.parse(fs.readFileSync(file, "utf8"));
  fs.writeFileSync(file, JSON.stringify([...now.filter((t) => !turns.some((n) => n.prompt === t.prompt)), ...turns], null, 1));
}

// Runs in the page: the orchestrator's thread, as the reports and the answers show in it.
const progress = () => {
  const global = tabs.find(isGlobalOrchestratorTab);
  const items = global ? threadOf(global).items : [];
  const said = (start) => items.some((i) => i.kind === "text" && (i.text ?? "").startsWith(start));
  return {
    running: !!global?.view.running,
    idle: global?.view.status === "done",
    sent: items.some((i) => i.kind === "user" && i.text.startsWith("#pinned release 0.17.21")),
    reports: items.filter((i) => i.kind === "user").flatMap((i) => reportBlocks(i.text).map((b) => b.project)),
    site: said("site is live on v2."),
    answered: said("Both Captains reported"),
  };
};
const at = (page) => page.evaluate(progress);

module.exports = {
  board: "Orchestrator",
  viewport: { width: 1440, height: 900 },
  regions: [
    { name: "sidebar", x: 0, y: 0, width: 264, height: 900 },
    { name: "menu", x: 8, y: 614, width: 248, height: 228 },
    { name: "header", x: 264, y: 0, width: 1176, height: 49 },
    { name: "chat", x: 264, y: 49, width: 880, height: 851 },
    { name: "captains", x: 1143, y: 49, width: 297, height: 851 },
  ],
  async seed(page, ctx) {
    if ((await at(page)).sent) return;
    script(ctx, HOST, [{ prompt: TO_PINNED, say: PINNED_REPORT }, { prompt: SITE_REPORT, say: SITE_ANSWER }, { prompt: PINNED_REPORT, say: ANSWER }]);
    script(ctx, SITE, [{ prompt: TO_SITE, say: SITE_REPORT }]);
    // pinned has a second checkout on mac-mini (the updater's worktree), so each message names its folder.
    const folder = (machine, name) => fs.realpathSync(path.join(homeOf(ctx, machine), "work", name));
    const calls = [
      { name: "send_to_captain", input: { project: folder(HOST, "pinned"), machine: HOST, message: TO_PINNED, request_id: "redesign-pinned" } },
      { name: "send_to_captain", input: { project: folder(SITE, "site"), machine: SITE, message: TO_SITE, request_id: "redesign-site" } },
    ];
    fs.writeFileSync(path.join(homeOf(ctx, HOST), "taste-calls.jsonl"), calls.map((c) => JSON.stringify(c)).join("\n"));
    await page.evaluate(() => {
      const global = tabs.find(isGlobalOrchestratorTab);
      activate(global);
      if (!global.view.running) startTab(global);
    });
    await ctx.waitFor("the orchestrator to start", async () => (await at(page)).running, 30000);
    await page.evaluate((text) => {
      prompt.value = text;
      send();
    }, REQUEST);
    await ctx.waitFor("site's Captain to report", async () => (await at(page)).site, 60000);
    await ctx.waitFor("the orchestrator to settle", async () => (await at(page)).idle, 30000);
    await page.evaluate((text) => {
      const captain = tabs.find((t) => t.view.role === "orchestrator" && projectName(t) === "pinned");
      return callOn(captain.machine, {
        cmd: "orchestrator_report",
        request_id: "redesign-pinned-report",
        report: { machine: captain.machine, machine_name: machineName(captain.machine), project: "pinned", path: captain.view.cwd, tab: captain.view.id, text, answers: ["redesign-pinned"] },
      });
    }, PINNED_REPORT);
    await ctx.waitFor("the orchestrator's answer to both", async () => (await at(page)).answered, 30000).catch((e) => ctx.warn(e.message));
  },
  async open(page) {
    await page.evaluate((draft) => {
      activate(tabs.find(isGlobalOrchestratorTab));
      prompt.value = draft;
      prompt.dispatchEvent(new Event("input"));
      prompt.blur();
    }, DRAFT);
  },
};
