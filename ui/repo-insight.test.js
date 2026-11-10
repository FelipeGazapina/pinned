import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const accountSrc = read("account.js");
const fn = (src, name) => {
  const at = src.search(new RegExp(`^(async )?function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};

/** The Account page's repo access switch over a hub that answers `answer`. */
function card(settings, answer) {
  const calls = [];
  const input = { checked: false, onchange: null };
  const error = { textContent: "" };
  const context = {
    settings,
    connectorCall: async (cmd, args) => (calls.push({ cmd, ...args }), answer(cmd, args)),
    document: { getElementById: (id) => ({ "repo-insight": input, "repo-insight-error": error })[id] },
  };
  vm.createContext(context);
  vm.runInContext([fn(accountSrc, "repoInsightHtml"), fn(accountSrc, "bindRepoInsight")].join("\n"), context);
  return { context, calls, input, error };
}

test("the switch is off until the account turned it on, and says what it allows in English", () => {
  const off = card({}, () => null).context.repoInsightHtml();
  assert.match(off, /Orchestrator repo access/);
  assert.match(off, /never changes them/);
  assert.doesNotMatch(off, / checked/);
  assert.match(card({ repo_insight: { on: true } }, () => null).context.repoInsightHtml(), / checked/);
});

test("turning it on sends set_repo_insight and keeps what the hub saved", async () => {
  const { context, calls, input } = card({}, (_, args) => args.settings);
  context.bindRepoInsight();
  input.checked = true;
  await input.onchange();
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [{ cmd: "set_repo_insight", settings: { on: true } }]);
  assert.equal(context.settings.repo_insight.on, true);
});

test("a refused change flips the switch back and shows why", async () => {
  const { context, input, error } = card({}, () => Promise.reject("an agent asked; approve it in the app"));
  context.bindRepoInsight();
  input.checked = true;
  await input.onchange();
  assert.equal(input.checked, false);
  assert.equal(error.textContent, "an agent asked; approve it in the app");
});
