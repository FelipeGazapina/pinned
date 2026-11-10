import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const line = (src, start) => {
  const at = src.indexOf(start);
  assert.ok(at >= 0, `${start} is in the source`);
  return src.slice(at, src.indexOf("\n", at));
};
const block = (src, start, end) => {
  const at = src.indexOf(start);
  assert.ok(at >= 0, `${start} is in the source`);
  return src.slice(at, src.indexOf(end, at) + end.length);
};

/** Node reads its default locale at start-up from the environment, so a Portuguese browser is a Node run under `LC_ALL=pt_BR.UTF-8`. */
function inPortugueseBrowser(code) {
  const out = execFileSync(process.execPath, ["-e", `${code}`], {
    env: { ...process.env, LC_ALL: "pt_BR.UTF-8", LANG: "pt_BR.UTF-8", TZ: "UTC" },
    encoding: "utf8",
  });
  return JSON.parse(out);
}

const transport = read("transport.js");
const locale = line(transport, "const APP_LOCALE = ");
const probe = (source, expression) =>
  inPortugueseBrowser(`
    const browser = Intl.DateTimeFormat().resolvedOptions().locale;
    ${locale}
    ${source}
    console.log(JSON.stringify({ browser, value: ${expression} }));
  `);

// 2026-11-10 15:05 UTC, and a day other than today for shortTime.
const NOV_10 = Date.UTC(2026, 10, 10, 15, 5) / 1000;
const MAR_5 = Date.UTC(2020, 2, 5, 15, 5) / 1000;

test("the invite card dates a code in English when the browser speaks Portuguese", () => {
  const { browser, value } = probe(line(read("account.js"), "const inviteDate = "), `inviteDate(${NOV_10})`);
  assert.equal(browser, "pt-BR");
  assert.equal(value, "Nov 10");
});

test("the account's expiry clock reads in English when the browser speaks Portuguese", () => {
  const { browser, value } = probe(line(read("account.js"), "const clockTime = "), `clockTime(${NOV_10})`);
  assert.equal(browser, "pt-BR");
  assert.equal(value, "03:05 PM");
});

test("the project history's time reads in English when the browser speaks Portuguese", () => {
  const { browser, value } = probe(line(read("history.js"), "const when = "), `when(${NOV_10})`);
  assert.equal(browser, "pt-BR");
  assert.equal(value, "11/10/26, 3:05 PM");
});

test("a thread's time in the sidebar reads in English when the browser speaks Portuguese", () => {
  const { browser, value } = probe(block(read("gui-panel.js"), "const shortTime = ", "\n};\n"), `shortTime(${MAR_5})`);
  assert.equal(browser, "pt-BR");
  assert.equal(value, "Mar 5");
});

test("the snooze menu's times and weekday read in English when the browser speaks Portuguese", () => {
  const source = block(read("gui-sidebar.js"), "function snoozePresets(", "\n}\n");
  const { browser, value } = probe(source, `snoozePresets(new Date(2026, 10, 10, 15, 5)).map((p) => p.when)`);
  assert.equal(browser, "pt-BR");
  assert.deepEqual(value, ["4:05 PM", "6:05 PM", "6:00 PM", "9:00 AM", "Mon 9:00 AM"]);
});
