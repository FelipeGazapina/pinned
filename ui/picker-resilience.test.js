import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const shell = readFileSync(new URL("./gui-shell.js", import.meta.url), "utf8");
const main = readFileSync(new URL("./main.js", import.meta.url), "utf8");

function element(html = "") {
  return {
    innerHTML: html,
    innerText: "",
    textContent: "",
    hidden: false,
    dataset: {},
    value: "",
    querySelector: () => element(),
    querySelectorAll: () => [],
    replaceChildren: () => {},
    insertAdjacentHTML: () => {},
    append: () => {},
    focus: () => {},
    scrollIntoView: () => {},
    setAttribute: () => {},
    classList: { toggle: () => {} },
  };
}

// A view that throws must say so in the frame, never leave it blank and mute.
function paletteWorld(groups) {
  const start = shell.indexOf("function renderPalette()");
  const end = shell.indexOf("\n}\n", start) + 3;
  const results = element();
  const context = {
    console,
    palette: { views: [{ view: "v", query: "" }], hi: -1, shown: [] },
    paletteView: () => ({ view: "v", query: "" }),
    specFor: () => ({ groups }),
    paletteInput: element(),
    paletteEl: { dataset: {}, querySelector: (sel) => (sel === ".results" ? results : element()) },
    paletteResults: results,
    icon: () => "",
    esc: (s) => String(s),
    selectable: () => [],
  };
  vm.createContext(context);
  vm.runInContext(shell.slice(start, end), context);
  return { context, results };
}

test("a throwing view names the snag in the frame", () => {
  const { context, results } = paletteWorld(() => {
    throw new Error("a directory needs a local id and name");
  });
  context.renderPalette();
  assert.match(results.innerHTML, /hit a snag/);
  assert.match(results.innerHTML, /a directory needs a local id and name/);
});

test("a working view still paints its items", () => {
  const { context, results } = paletteWorld(() => [{ label: "G", items: [{ title: "Proj", run: () => {} }] }]);
  context.renderPalette();
  assert.match(results.innerHTML, /Proj/);
});

// Local projects paint without waiting for shared ones.
function projectsWorld() {
  const start = main.indexOf("function paintProjects(machine");
  const end = main.indexOf("async function pickFolder");
  const box = element();
  const context = {
    console,
    document: { createElement: () => element() },
    LOCAL: "local",
    choices: [],
    picking: { machine: "local", tab: null },
    picker: { querySelector: () => box, querySelectorAll: () => [] },
    PinnedProjects: { sessionOn: () => ({ label: "Here", path: "/p" }) },
    tilde: (p) => p,
    esc: (s) => String(s),
    icon: () => "",
    machineName: () => "M",
    choiceButtons: () => [],
    choose: () => {},
    cloneAndOpen: () => {},
    cloneFromGithub: () => {},
    pickLocalFolder: () => {},
    browseFolders: () => {},
    pickFolder: () => {},
    machineDirectory: () => ({ localId: "local", localName: "M", names: {}, online: {} }),
  };
  vm.createContext(context);
  vm.runInContext(main.slice(start, end), context);
  return context;
}

test("shared projects join the local list when they arrive", () => {
  const context = projectsWorld();
  // Names cross the vm realm, so they compare as JSON.
  const names = () => JSON.stringify(context.choices.map((c) => c.name));
  const mine = [{ name: "Mine", path: "/repo", repo: "git@github.com:o/r.git" }];
  context.paintProjects("local", null, mine, []);
  assert.equal(names(), JSON.stringify(["Mine", "Clone from GitHub…", "Open folder…"]));
  context.paintProjects("local", null, mine, [{ name: "Theirs", key: "o/t", repo: "git@github.com:o/t.git" }]);
  assert.equal(names(), JSON.stringify(["Mine", "Theirs", "Clone from GitHub…", "Open folder…"]));
});
