import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import { groupProjects } from "./project-groups.js";

const context = {};
vm.createContext(context);
vm.runInContext(readFileSync(new URL("./composer-tags.js", import.meta.url), "utf8"), context);

// Objects built inside the vm carry its prototypes; a JSON round trip makes them comparable here.
const here = (value) => (value === undefined ? value : JSON.parse(JSON.stringify(value)));
const call = (name) => (...args) => here(context[name](...args));
const isGlobalOrchestratorTab = call("isGlobalOrchestratorTab");
const tagSlug = call("tagSlug");
const hasTags = call("hasTags");
const composerParagraphs = call("composerParagraphs");
const tagTokenAt = call("tagTokenAt");
const paragraphTags = call("paragraphTags");
const routeParagraph = call("routeParagraph");
const routePreview = call("routePreview");
const tagParagraph = call("tagParagraph");
const insertTagText = call("insertTagText");
const tagPopoverKey = call("tagPopoverKey");
const tagOptions = call("tagOptions");
const tagCatalog = call("tagCatalog");
const tagLineHtml = call("tagLineHtml");
const composerTagPopoverHtml = call("composerTagPopoverHtml");

const host = (id, name, slug, online, captain = "none") => ({ id, name, slug, online, captain });
const catalog = {
  projects: [
    { slug: "pinned", name: "pinned", repo: "github.com/felipe/pinned", machines: [host("mm", "Mac mini", "mac-mini", true, "idle")] },
    {
      slug: "site",
      name: "site",
      repo: "github.com/felipe/site",
      machines: [host("mb", "MacBook", "macbook", true, "working"), host("mm", "Mac mini", "mac-mini", true)],
    },
    { slug: "sistema-notas", name: "sistema-notas", repo: "", machines: [host("mm", "Mac mini", "mac-mini", true)] },
    { slug: "infra", name: "infra", repo: "github.com/felipe/infra", machines: [host("vps", "Linux VPS", "linux-vps", false)] },
  ],
  machines: [
    { slug: "mac-mini", name: "Mac mini", online: true },
    { slug: "macbook", name: "MacBook", online: true },
    { slug: "linux-vps", name: "Linux VPS", online: false },
  ],
  name: "Orchestrator",
};

test("only a tab whose view says orchestrator_global is the global orchestrator", () => {
  assert.equal(isGlobalOrchestratorTab({ view: { orchestrator_global: true } }), true);
  assert.equal(isGlobalOrchestratorTab({ view: { role: "orchestrator" } }), false);
  assert.equal(isGlobalOrchestratorTab({ view: { orchestrator_global: false } }), false);
  assert.equal(isGlobalOrchestratorTab({ view: {} }), false);
  assert.equal(isGlobalOrchestratorTab(null), false);
});

test("a slug is lower case, without accents, words joined by one hyphen", () => {
  assert.equal(tagSlug("MacBook Pro"), "macbook-pro");
  assert.equal(tagSlug("Mac mini"), "mac-mini");
  assert.equal(tagSlug("sistema-notas"), "sistema-notas");
  assert.equal(tagSlug("  Site  "), "site");
  assert.equal(tagSlug("São Paulo"), "sao-paulo");
  assert.equal(tagSlug("A/B_test.v2"), "a-b-test-v2");
  assert.equal(tagSlug("Felipe's Mac"), "felipes-mac");
  assert.equal(tagSlug(""), "");
});

test("a tag starts the text or follows whitespace, so an email is not one", () => {
  assert.equal(hasTags("no #site"), true);
  assert.equal(hasTags("@macbook faz"), true);
  assert.equal(hasTags("manda pra felipe@gazapina.com"), false);
  assert.equal(hasTags("cor a#b"), false);
  assert.deepEqual(paragraphTags("escreve pra felipe@macbook no #site"), { project: "site", machine: "" });
});

test("paragraphs split on blank lines and keep where they sit in the text", () => {
  assert.deepEqual(composerParagraphs("no #site, corrige o login.\n\nno #pinned, sobe a versão.\n\n"), [
    { text: "no #site, corrige o login.", start: 0, end: 26, index: 0 },
    { text: "no #pinned, sobe a versão.", start: 28, end: 54, index: 1 },
  ]);
  assert.deepEqual(composerParagraphs("uma linha\noutra linha\n \nterceira"), [
    { text: "uma linha\noutra linha", start: 0, end: 21, index: 0 },
    { text: "terceira", start: 24, end: 32, index: 1 },
  ]);
  assert.deepEqual(composerParagraphs("   \n\n  \n"), []);
});

test("the token at the caret opens only on a sign that starts a word", () => {
  assert.deepEqual(tagTokenAt("no #si", 6), { sign: "#", query: "si", start: 3, end: 6 });
  assert.deepEqual(tagTokenAt("no #si agora", 5), { sign: "#", query: "s", start: 3, end: 6 });
  assert.deepEqual(tagTokenAt("@", 1), { sign: "@", query: "", start: 0, end: 1 });
  assert.deepEqual(tagTokenAt("vai @Mac", 8), { sign: "@", query: "mac", start: 4, end: 8 });
  assert.equal(tagTokenAt("email felipe@mac", 16), null);
  assert.equal(tagTokenAt("nada aqui", 9), null);
});

test("the first # and the first @ of a paragraph decide it", () => {
  assert.deepEqual(paragraphTags("no #Site @macbook e também #pinned @mac-mini"), { project: "site", machine: "macbook" });
  assert.deepEqual(paragraphTags("sem tag nenhuma"), { project: "", machine: "" });
});

test("a paragraph routes to its project's Captain on the one machine it is on", () => {
  assert.deepEqual(routeParagraph("sobe a v0.17.19 no #pinned", catalog), {
    preview: "sobe a v0.17.19 no #pinned",
    kind: "routed",
    text: "→ Captain pinned · Mac mini",
  });
});

test("a repo on two machines with no @ is ambiguous and offers each machine", () => {
  assert.deepEqual(routeParagraph("corrige o login no #site", catalog), {
    preview: "corrige o login no #site",
    kind: "ambiguous",
    text: "site is on 2 machines",
    choices: ["macbook", "mac-mini"],
  });
});

test("an @ picks the machine for its paragraph", () => {
  assert.equal(routeParagraph("#site @mac-mini publica", catalog).text, "→ Captain site · Mac mini");
  assert.equal(routeParagraph("#site @macbook publica", catalog).text, "→ Captain site · MacBook");
  assert.equal(routeParagraph("#pinned @linux-vps sobe", catalog).text, "→ Captain pinned · Linux VPS");
});

test("an untagged paragraph is the orchestrator's call, and an unknown project is flagged", () => {
  assert.deepEqual(routeParagraph("olha isso aqui", catalog), {
    preview: "olha isso aqui",
    kind: "untagged",
    text: "no project: Orchestrator decides",
  });
  assert.equal(routeParagraph("olha isso", { ...catalog, name: "Admiral" }).text, "no project: Admiral decides");
  assert.deepEqual(routeParagraph("no #fantasma", catalog), {
    preview: "no #fantasma",
    kind: "unknown",
    text: "unknown project #fantasma",
  });
  assert.equal(
    routeParagraph("uma frase bem comprida que passa de cinquenta letras no #pinned", catalog).preview,
    "uma frase bem comprida que passa de cinquenta letr…",
  );
});

test("a tag routes only its own paragraph", () => {
  const text = "no #site @macbook, corrige o login.\n\nsobe a versão.\n\nno #site, publica.\n\nno #fantasma, nada.";
  assert.deepEqual(
    routePreview(text, catalog).map((row) => [row.index, row.kind, row.text]),
    [
      [0, "routed", "→ Captain site · MacBook"],
      [1, "untagged", "no project: Orchestrator decides"],
      [2, "ambiguous", "site is on 2 machines"],
      [3, "unknown", "unknown project #fantasma"],
    ],
  );
});

test("a machine button writes @slug into its paragraph only", () => {
  const text = "no #site, corrige.\n\nno #site, publica.";
  const first = tagParagraph(text, 0, "macbook");
  assert.equal(first, "no #site @macbook, corrige.\n\nno #site, publica.");
  assert.equal(tagParagraph(first, 1, "mac-mini"), "no #site @macbook, corrige.\n\nno #site @mac-mini, publica.");
  assert.equal(tagParagraph(first, 0, "mac-mini"), first);
  assert.equal(tagParagraph("sem projeto\n\noutro", 1, "mac-mini"), "sem projeto\n\n@mac-mini outro");
  assert.equal(tagParagraph("só um", 4, "mac-mini"), "só um");
});

test("inserting replaces the partial tag with the full one and a space", () => {
  assert.deepEqual(insertTagText("no #si agora", 6, "site"), { text: "no #site agora", caret: 9 });
  assert.deepEqual(insertTagText("vai @mac", 8, "macbook"), { text: "vai @macbook ", caret: 13 });
  assert.deepEqual(insertTagText("#", 1, "pinned"), { text: "#pinned ", caret: 8 });
  assert.equal(insertTagText("nada", 4, "site"), null);
});

test("popover keys move, insert and close only while it is open", () => {
  assert.deepEqual(tagPopoverKey("ArrowDown", { open: true, count: 3, hi: 0 }), { handled: true, hi: 1, action: "move" });
  assert.deepEqual(tagPopoverKey("ArrowDown", { open: true, count: 3, hi: 2 }), { handled: true, hi: 0, action: "move" });
  assert.deepEqual(tagPopoverKey("ArrowUp", { open: true, count: 3, hi: 0 }), { handled: true, hi: 2, action: "move" });
  assert.deepEqual(tagPopoverKey("Enter", { open: true, count: 3, hi: 1 }), { handled: true, hi: 1, action: "insert" });
  assert.deepEqual(tagPopoverKey("Tab", { open: true, count: 3, hi: 5 }), { handled: true, hi: 2, action: "insert" });
  assert.deepEqual(tagPopoverKey("Escape", { open: true, count: 3, hi: 1 }), { handled: true, hi: 1, action: "close" });
  assert.deepEqual(tagPopoverKey("Enter", { open: true, count: 0, hi: 0 }), { handled: false, hi: 0, action: "none" });
  assert.deepEqual(tagPopoverKey("Enter", { open: false, count: 3, hi: 0 }), { handled: false, hi: 0, action: "none" });
  assert.deepEqual(tagPopoverKey("a", { open: true, count: 3, hi: 0 }), { handled: false, hi: 0, action: "none" });
  assert.deepEqual(tagPopoverKey("Enter", { open: true, count: 1, hi: 0, typed: true }), { handled: false, hi: 0, action: "none" });
  assert.deepEqual(tagPopoverKey("Tab", { open: true, count: 1, hi: 0, typed: true }), { handled: true, hi: 0, action: "insert" });
});

test("# filters projects by slug and @ filters machines by slug or name", () => {
  assert.deepEqual(tagOptions({ sign: "#", query: "si" }, catalog).map((p) => p.slug), ["site", "sistema-notas"]);
  assert.deepEqual(tagOptions({ sign: "#", query: "" }, catalog).map((p) => p.slug), ["pinned", "site", "sistema-notas", "infra"]);
  assert.deepEqual(tagOptions({ sign: "#", query: "zzz" }, catalog), []);
  assert.deepEqual(tagOptions({ sign: "@", query: "mac" }, catalog).map((m) => m.slug), ["mac-mini", "macbook"]);
  assert.deepEqual(tagOptions({ sign: "@", query: "linux" }, catalog).map((m) => m.slug), ["linux-vps"]);
  assert.deepEqual(tagOptions(null, catalog), []);
});

test("the catalog groups projects by repo as the sidebar does and reads each Captain", () => {
  const capWorking = { machine: "mb", view: { role: "orchestrator", status: "working" } };
  const capIdle = { machine: "mm", view: { role: "orchestrator", status: "idle" } };
  const global = { machine: "mm", view: { orchestrator_global: true } };
  const groups = groupProjects({
    threads: [
      { machine: "mb", path: "/Users/f/site", name: "site", repo: "github.com/felipe/site", thread: capWorking },
      { machine: "mm", path: "/Users/f/pinned", name: "pinned", repo: "github.com/felipe/pinned", thread: capIdle },
      { machine: "mm", path: "/Users/f/.pinned/orquestrador", name: "orquestrador", repo: null, thread: global },
    ],
    listed: {
      mm: [
        { path: "/Users/f/site", name: "site", repo: "github.com/felipe/site" },
        { path: "/Users/f/notas", name: "Sistema Notas", repo: null },
      ],
      vps: [{ path: "/srv/infra", name: "infra", repo: "github.com/felipe/infra" }],
    },
    grouping: true,
  });
  const repos = { "mb:/Users/f/site": "github.com/felipe/site", "mm:/Users/f/site": "github.com/felipe/site", "mm:/Users/f/pinned": "github.com/felipe/pinned", "vps:/srv/infra": "github.com/felipe/infra" };
  const built = here(
    context.tagCatalog({
      groups,
      directory: { localId: "mm", localName: "Mac mini", names: { mm: "This Mac", mb: "MacBook", vps: "Linux VPS" }, online: { mm: true, mb: true, vps: false } },
      phase: (tab) => tab.view.status,
      repoAt: (machine, path) => repos[`${machine}:${path}`] ?? null,
    }),
  );
  assert.deepEqual(built, {
    projects: [
      {
        slug: "site",
        name: "site",
        repo: "github.com/felipe/site",
        machines: [
          { id: "mb", slug: "macbook", name: "MacBook", online: true, captain: "working" },
          { id: "mm", slug: "mac-mini", name: "Mac mini", online: true, captain: "none" },
        ],
      },
      {
        slug: "pinned",
        name: "pinned",
        repo: "github.com/felipe/pinned",
        machines: [{ id: "mm", slug: "mac-mini", name: "Mac mini", online: true, captain: "idle" }],
      },
      {
        slug: "sistema-notas",
        name: "Sistema Notas",
        repo: "",
        machines: [{ id: "mm", slug: "mac-mini", name: "Mac mini", online: true, captain: "none" }],
      },
      {
        slug: "infra",
        name: "infra",
        repo: "github.com/felipe/infra",
        machines: [{ id: "vps", slug: "linux-vps", name: "Linux VPS", online: false, captain: "none" }],
      },
    ],
    machines: [
      { slug: "mac-mini", name: "Mac mini", online: true },
      { slug: "macbook", name: "MacBook", online: true },
      { slug: "linux-vps", name: "Linux VPS", online: false },
    ],
    name: "Orchestrator",
  });
  assert.equal(tagCatalog({ groups: [], directory: { localId: "local", localName: "This Mac", names: {}, online: {} }, phase: () => "idle", repoAt: () => null }).machines.length, 0);
});

test("a workers callback counts each host's running workers, and without it the host entries stand as they were", () => {
  const captain = { machine: "mm", view: { role: "orchestrator", status: "working" } };
  const groups = groupProjects({
    threads: [{ machine: "mm", path: "/Users/f/site", name: "site", repo: "github.com/felipe/site", thread: captain }],
    listed: { mm: [{ path: "/Users/f/site", name: "site", repo: "github.com/felipe/site" }] },
    grouping: true,
  });
  const args = {
    groups,
    directory: { localId: "mm", localName: "Mac mini", names: { mm: "Mac mini" }, online: { mm: true } },
    phase: () => "working",
    repoAt: () => "github.com/felipe/site",
  };
  assert.deepEqual(tagCatalog({ ...args, workers: () => 2 }).projects[0].machines, [
    { id: "mm", slug: "mac-mini", name: "Mac mini", online: true, captain: "working", workers: 2 },
  ]);
  assert.deepEqual(tagCatalog(args).projects[0].machines, [{ id: "mm", slug: "mac-mini", name: "Mac mini", online: true, captain: "working" }]);
});

test("the mirror paints a known project blue, a known machine green and an unknown tag red, letter for letter", () => {
  const known = { project: new Set(["site"]), machine: new Set(["macbook"]) };
  const esc = (text) => text.replace(/&/g, "&amp;").replace(/</g, "&lt;");
  assert.equal(
    tagLineHtml("no #site @macbook e #x <b> a@b", known, esc),
    'no <span class="tag project">#site</span> <span class="tag machine">@macbook</span> e <span class="tag project unknown">#x</span> &lt;b> a@b',
  );
});

test("the popover lists repo or no remote, machines with offline ones dimmed, and the Captain state", () => {
  const esc = (text) => text;
  const projects = tagOptions({ sign: "#", query: "" }, catalog);
  const html = composerTagPopoverHtml({ sign: "#", query: "" }, projects, 1, esc);
  assert.match(html, /^<p class="heading">Projects<\/p>/);
  assert.match(html, /<button type="button" class="highlight"><span class="sigil project">#<\/span><span class="text"><span class="name">site<\/span><span class="about">github.com\/felipe\/site · <span class="">MacBook<\/span> · <span class="">Mac mini<\/span><\/span><\/span><span class="state working">working<\/span><\/button>/);
  assert.match(html, /<span class="name">pinned<\/span>.*<span class="state idle">idle<\/span>/);
  assert.match(html, /<span class="about">no remote · <span class="">Mac mini<\/span><\/span><\/span><span class="state none">none<\/span>/);
  assert.match(html, /<button type="button" class="" data-offline="true">.*<span class="about">github.com\/felipe\/infra · <span class="off">Linux VPS<\/span><\/span><\/span><span class="state offline">offline<\/span>/);
  const machinesHtml = composerTagPopoverHtml({ sign: "@", query: "" }, tagOptions({ sign: "@", query: "" }, catalog), 0, esc);
  assert.match(machinesHtml, /<span class="name">Linux VPS<\/span><span class="about">offline<\/span><\/span><span class="state">@linux-vps<\/span>/);
  assert.equal(composerTagPopoverHtml({ sign: "#", query: "zz" }, [], 0, esc), `<p class="heading">Projects</p><p class="none">No project matches 'zz'</p>`);
});
