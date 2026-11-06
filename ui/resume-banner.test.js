import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./gui-timeline.js", import.meta.url), "utf8");
const mainSrc = readFileSync(new URL("./main.js", import.meta.url), "utf8");

function runSlice(source, startAt, endAt, extra = "") {
  const start = source.indexOf(startAt);
  const end = source.indexOf(endAt, start + startAt.length);
  const context = {
    console,
    icon: (name, cls = "") => `<svg data-icon="${name}" class="${cls}"></svg>`,
    subagentShown: () => null,
  };
  vm.createContext(context);
  vm.runInContext(`${source.slice(start, end)}\n${extra}`, context);
  return context;
}

function resumeWorld() {
  const context = runSlice(
    src,
    "/** Interrupted tabs dismissed this boot",
    "function renderThread()",
    ";Object.assign(globalThis, { resumeOffered, resumeDismissed, resumeHtml });",
  );
  return context;
}

test("the banner offers interrupted tabs that are not running", () => {
  const context = resumeWorld();
  const tab = (view) => ({ key: "local:1", view });
  const idle = tab({ interrupted: false, running: false });
  const running = tab({ interrupted: true, running: true });
  const waiting = tab({ interrupted: true, running: false });
  assert.equal(context.resumeOffered(idle), false);
  assert.equal(context.resumeOffered(running), false);
  assert.equal(context.resumeOffered(waiting), true);
  assert.equal(context.resumeOffered(null), false);
  context.resumeDismissed.add("local:1");
  assert.equal(context.resumeOffered(waiting), false, "dismissed tabs stay quiet this boot");
});

test("the banner names its buttons for the thread handlers", () => {
  const context = resumeWorld();
  const html = context.resumeHtml();
  assert.match(html, /Continue working from where you stopped\?/);
  assert.match(html, /data-resume-tab/);
  assert.match(html, /data-dismiss-resume/);
});

test("a bad restored tab cannot take boot down with it", async () => {
  const start = mainSrc.indexOf("  // One unrestorable tab must not take boot down");
  const end = mainSrc.indexOf("if (signedIn.signed_in) connectHub(signedIn);") + "if (signedIn.signed_in) connectHub(signedIn);".length;
  const started = [];
  let hubbed = null;
  let opened = 0;
  const context = { console, LOCAL: "local" };
  vm.createContext(context);
  Object.assign(context, {
    machines: new Map([["local", { tabs: [{ key: "local:good" }] }]]),
    invoke: async (cmd) => (cmd === "account" ? { signed_in: true } : ["good", "gone"]),
    refreshTabs: async () => {},
    keyOf: (machine, id) => `${machine}:${id}`,
    tabByKey: (key) => (key === "local:good" ? { key, boom: true } : null),
    startTab: async (tab) => {
      started.push(tab.key);
      if (tab.boom) throw new Error("claude is gone");
    },
    openTab: async () => {
      opened += 1;
    },
    account: async () => ({ signed_in: true }),
    connectHub: (signedIn) => {
      hubbed = signedIn;
    },
  });
  const boot = `(async () => {${mainSrc.slice(start, end)}})()`;
  await vm.runInContext(boot, context);
  assert.deepEqual(started, ["local:good"], "the good tab still starts");
  assert.equal(opened, 0, "no fresh tab covers the restored one");
  assert.deepEqual(hubbed, { signed_in: true }, "the hub still connects");
});
