import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const context = {};
vm.createContext(context);
for (const file of ["composer-tags.js", "requests-model.js"]) {
  vm.runInContext(readFileSync(new URL(`./${file}`, import.meta.url), "utf8"), context);
}

const here = (value) => (value === undefined ? value : JSON.parse(JSON.stringify(value)));
const splitRequest = (...args) => here(context.splitRequest(...args));

const mini = { slug: "mac-mini", name: "Mac mini", online: true, captain: "idle" };
const book = { slug: "macbook", name: "MacBook", online: true, captain: "none" };
const catalog = {
  projects: [
    { slug: "pinned", name: "Pinned", repo: "", machines: [mini] },
    { slug: "site", name: "Site", repo: "", machines: [book, mini] },
  ],
  machines: [mini, book],
  name: "Orchestrator",
};

test("a text with no tags is one item with project null", () => {
  assert.deepEqual(splitRequest("Fix the login bug. It fails on Safari.", catalog), [
    { title: "Fix the login bug", text: "Fix the login bug. It fails on Safari.", project: null, machine: null },
  ]);
});

test("an empty or blank text gives no items", () => {
  assert.deepEqual(splitRequest("", catalog), []);
  assert.deepEqual(splitRequest("  \n\n  ", catalog), []);
  assert.deepEqual(splitRequest(undefined, catalog), []);
});

test("each tagged paragraph starts an item, and its project and machine come from the route", () => {
  assert.deepEqual(splitRequest("#pinned Fix the login bug\n\n#site @macbook Update the footer", catalog), [
    { title: "Fix the login bug", text: "#pinned Fix the login bug", project: "Pinned", machine: "Mac mini" },
    { title: "Update the footer", text: "#site @macbook Update the footer", project: "Site", machine: "MacBook" },
  ]);
});

test("an untagged paragraph joins the item above it: two tagged and one untagged give 2 items, the second including it", () => {
  const items = splitRequest("#pinned Fix the login bug\n\n#site Update the footer\n\nAlso check the links", catalog);
  assert.equal(items.length, 2);
  assert.equal(items[0].text, "#pinned Fix the login bug");
  assert.equal(items[1].text, "#site Update the footer\n\nAlso check the links");
  assert.equal(items[1].project, "Site");
  assert.equal(items[1].machine, null);
});

test("an untagged paragraph between two tagged ones joins the first", () => {
  const items = splitRequest("#pinned Fix the login bug\n\nIt fails on Safari\n\n#site Update the footer", catalog);
  assert.deepEqual(items.map((i) => i.text), ["#pinned Fix the login bug\n\nIt fails on Safari", "#site Update the footer"]);
});

test("untagged paragraphs before the first tag form their own item with project null", () => {
  const items = splitRequest("Some context first\n\n#pinned Fix the login bug", catalog);
  assert.deepEqual(items.map((i) => [i.title, i.project]), [["Some context first", null], ["Fix the login bug", "Pinned"]]);
});

test("a tag the catalog does not know, or a project on two machines with no @, still starts an item but has no project", () => {
  assert.deepEqual(splitRequest("#nope Do a thing\n\n#site Do another", catalog).map((i) => [i.project, i.machine]), [[null, null], [null, null]]);
});

test("the title is the first sentence without its tags", () => {
  assert.equal(splitRequest("#pinned @mac-mini Fix the login bug. Then tidy the README.", catalog)[0].title, "Fix the login bug");
  assert.equal(splitRequest("Why does it fail? Please look.", catalog)[0].title, "Why does it fail?");
  assert.equal(splitRequest("First line\nSecond line", catalog)[0].title, "First line");
});

test("the title is cut at a word at 60 characters, with an ellipsis", () => {
  const long = "Refactor the authentication middleware so that every request carries a trace id and logs it";
  assert.equal(splitRequest(`#pinned ${long}`, catalog)[0].title, "Refactor the authentication middleware so that every…");
  assert.equal(splitRequest("Refactor the authentication middleware so that every requests", catalog)[0].title, "Refactor the authentication middleware so that every…");
});

test("a title of exactly 60 characters is not cut", () => {
  const sixty = "Refactor the authentication middleware so that every request";
  assert.equal(sixty.length, 60);
  assert.equal(splitRequest(sixty, catalog)[0].title, sixty);
});

test("a title with no space in 60 characters is cut at 59 characters plus the ellipsis", () => {
  const word = "x".repeat(80);
  assert.equal(splitRequest(word, catalog)[0].title, `${"x".repeat(59)}…`);
});
