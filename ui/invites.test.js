import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const accountSrc = read("account.js");
const settingsSrc = read("settings.js");

const line = (src, start) => {
  const at = src.indexOf(start);
  return at < 0 ? "" : src.slice(at, src.indexOf("\n", at));
};
const fn = (src, name) => {
  const at = src.search(new RegExp(`^(async )?function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
const inviteActions = (src) => {
  const at = src.indexOf("Object.assign(actions, {");
  return src.slice(at, src.indexOf("\n});", at) + 4);
};
const escFn = line(settingsSrc, "const esc = ");

/** The account page's invite card over a hub that answers `answer`, with `pending` codes unused. */
function invites(pending, answer) {
  const calls = [];
  const drawn = [];
  const context = {
    actions: {},
    invitesUi: { of: null, signup: "invite", pending, made: null, error: "", loading: false },
    hub: { hub: async (call) => (calls.push(call), answer(call)) },
    settingsOpen: () => true,
    page: "account",
    showAccount: () => drawn.push("account page"),
    inviteDate: () => "Nov 16",
  };
  vm.createContext(context);
  vm.runInContext(
    [
      escFn,
      fn(accountSrc, "loadInvites"),
      fn(accountSrc, "invitesHtml"),
      fn(accountSrc, "inviteAction"),
      inviteActions(accountSrc),
    ].join("\n"),
    context,
  );
  return { context, calls, drawn };
}

const code = (id, hint, created_at) => ({ id, hint, created_at });
const revokeButton = (id) => ({ closest: () => ({ dataset: { invite: id } }) });

test("the card says a code doesn't expire and lists each unused code with a Revoke button", () => {
  const { context } = invites(
    [code("inv-2", "K7Q2", 1794854224), code("inv-1", "AB3C", 1794767824)],
    () => null,
  );
  const html = context.invitesHtml().replace(/\s+/g, " ");
  assert.match(html, /It works once and doesn't expire\./);
  assert.match(html, /Codes not used yet/);
  assert.match(html, /data-invite="inv-2"> <div class="who"><b>K7Q2-…<\/b><span class="hint">Created Nov 16<\/span>/);
  assert.match(html, /data-invite="inv-1"> <div class="who"><b>AB3C-…<\/b><span class="hint">Created Nov 16<\/span>/);
  assert.equal(html.match(/data-action="revoke-invite">Revoke/g).length, 2);
  assert.doesNotMatch(html, /expires/);
});

test("the card names an empty state when no code is waiting", () => {
  const { context } = invites([], () => null);
  const html = context.invitesHtml();
  assert.match(html, /No codes waiting to be used\./);
  assert.doesNotMatch(html, /machine-row|Codes not used yet/);
});

test("revoking a code tells the hub and drops it from the reloaded list", async () => {
  const codes = [code("inv-2", "K7Q2", 1794854224), code("inv-1", "AB3C", 1794767824)];
  const { context, calls, drawn } = invites(codes, (call) =>
    call.cmd === "invites" ? { signup: "invite", pending: codes.filter((c) => c.id !== "inv-2") } : null,
  );
  await context.actions["revoke-invite"](revokeButton("inv-2"));
  assert.deepEqual(
    calls.map((call) => JSON.stringify(call)),
    ['{"cmd":"revoke_invite","id":"inv-2"}', '{"cmd":"invites"}'],
  );
  assert.deepEqual(context.invitesUi.pending.map((c) => c.id), ["inv-1"]);
  assert.deepEqual(drawn, ["account page"]);
  assert.doesNotMatch(context.invitesHtml(), /data-invite="inv-2"/);
  assert.match(context.invitesHtml(), /data-invite="inv-1"/);
});

test("a revoke the hub refuses keeps the code and shows why", async () => {
  const codes = [code("inv-1", "AB3C", 1794767824)];
  const { context, calls } = invites(codes, (call) => {
    if (call.cmd === "revoke_invite") throw new Error(`this account has no unused invite ${call.id}`);
    return { signup: "invite", pending: codes };
  });
  await context.actions["revoke-invite"](revokeButton("inv-1"));
  assert.equal(calls[0].cmd, "revoke_invite");
  assert.equal(context.invitesUi.error, "Error: this account has no unused invite inv-1");
  assert.match(context.invitesHtml(), /data-invite="inv-1"/);
});
