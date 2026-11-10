import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const settingsSrc = read("settings.js");
const shellSrc = read("gui-shell.js");
const historySrc = read("history.js");

const lines = (src, from, to) => src.slice(src.indexOf(from), src.indexOf("\n", src.indexOf(to)));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^(async )?function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
const registration = (name) => lines(settingsSrc, `settingsSection({ page: "${name}"`, `settingsSection({ page: "${name}"`);

/** Settings with General remembered as the last page, and General's slow part (asking every machine
 * for the global orchestrator settings) held until the test lets it answer. */
function settings() {
  let answerMachines;
  const machinesAnswer = new Promise((resolve) => (answerMachines = resolve));
  const settingsPage = { innerHTML: "", dataset: {}, scrollTop: 0 };
  const nav = { innerHTML: "" };
  const context = {
    settingsPage,
    document: { body: { dataset: {} }, querySelector: () => nav },
    picker: {},
    active: null,
    load: (key) => (key === "settings-page" ? "general" : null),
    save: () => {},
    icon: () => "",
    esc: (text) => String(text),
    connectorCall: async () => ({}),
    featuresSettingsHtml: async () => "<section>features</section>",
    orchestrationSettingsHtml: async () => "<section>workers</section>",
    globalOrchestratorSettingsHtml: () => machinesAnswer.then(() => "<section>orchestrator</section>"),
    sidebarSettingsHtml: () => "",
    sendShortcutHtml: () => "",
    browserSettingsHtml: () => "",
    bindFeaturesSettings: () => {},
    bindOrchestrationSettings: () => {},
    bindGlobalOrchestratorSettings: () => {},
    bindSidebarSettings: () => {},
    bindSendShortcut: () => {},
    bindBrowserSettings: () => {},
    showArchive: () => (settingsPage.innerHTML = "<h2>Archive</h2>"),
    showMachinesAndRecheckSync: () => (settingsPage.innerHTML = "<h2>Machines</h2>"),
  };
  vm.createContext(context);
  vm.runInContext(
    [
      lines(settingsSrc, "const SETTINGS_ORDER = ", "let page = "),
      lines(settingsSrc, "const settingsOpen = ", "const settingsOpen = "),
      fn(settingsSrc, "toggleSettings"),
      fn(settingsSrc, "renderSettingsNav"),
      fn(settingsSrc, "showPage"),
      fn(shellSrc, "openSettings"),
      fn(historySrc, "showGeneral"),
      'settingsSection({ page: "general", label: "General", icon: "settings", show: () => showGeneral() });',
      registration("machines"),
      registration("archive"),
    ].join("\n"),
    context,
  );
  const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
  const answer = async () => {
    answerMachines();
    await settle();
  };
  return { context, settingsPage, nav, answer, settle };
}

for (const [name, heading] of [["archive", "Archive"], ["machines", "Machines"]]) {
  test(`opening Settings on ${heading} stays on ${heading} after General finishes loading`, async () => {
    const { context, settingsPage, nav, answer } = settings();
    await context.openSettings(name);
    assert.equal(settingsPage.innerHTML, `<h2>${heading}</h2>`);
    await answer();
    assert.equal(settingsPage.innerHTML, `<h2>${heading}</h2>`);
    assert.match(nav.innerHTML, new RegExp(`data-page="${name}" class="active"`));
  });
}

test("leaving General for Archive before it finishes loading stays on Archive", async () => {
  const { context, settingsPage, answer, settle } = settings();
  await context.openSettings();
  await settle();
  context.showPage("archive");
  await answer();
  assert.equal(settingsPage.innerHTML, "<h2>Archive</h2>");
});

test("General draws once its slow part answers, when the user stays on it", async () => {
  const { context, settingsPage, answer } = settings();
  await context.openSettings();
  await answer();
  assert.equal(
    settingsPage.innerHTML.replace(/\s+/g, " "),
    "<h2>General</h2> <section>features</section> <section>workers</section> <section>orchestrator</section> ",
  );
});
