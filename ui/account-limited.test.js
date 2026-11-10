import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

/** accounts.js's card, run as the page runs it. */
function card(account) {
  const context = {
    console,
    addEventListener() {},
    LOCAL: "local",
    actions: {},
    settingsSection() {},
    DEFAULT_ACCOUNT: "claude",
    accountName: () => "Claude Code",
    machineName: () => "MacBook",
    agentAsking: () => null,
    agentAskingHtml: () => "",
    icon: () => "",
    esc: (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/"/g, "&quot;"),
    document: { getElementById: () => null },
  };
  vm.createContext(context);
  vm.runInContext(readFileSync(new URL("./accounts.js", import.meta.url), "utf8") + "\nthis.accountCardHtml = accountCardHtml;", context);
  return context.accountCardHtml(account);
}

const go = { id: "personal", provider: "claude", auth: "opencode-go", name: "personal", provider_name: "OpenCode Go", builtin: false, signed_in: true };

test("an account a usage limit stopped shows Limited with its reason and when it resumes", () => {
  const html = card({ ...go, limited: true, limit_reason: "weekly usage at 100%", limit_resume_at: 1794783600 });
  assert.match(html, /<span class="pill limited">Limited<\/span>/);
  assert.match(html, /<p class="limit-note" data-limit>Limited: weekly usage at 100%\. Resumes [^<]*\d[^<]* Delegated tasks go to the next model on the ladder\.<\/p>/);
});

test("a limit with no reset time says pinned tries the account again, and an account with no limit shows nothing", () => {
  assert.match(card({ ...go, limited: true, limit_reason: "API Error: Request rejected (429) · Go usage limit exceeded" }), /Limited: API Error: Request rejected \(429\) · Go usage limit exceeded\. pinned tries it again soon\./);
  assert.doesNotMatch(card(go), /limit-note|pill limited/);
});
