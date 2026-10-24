// Builds ui/vendor/shiki/: in a scratch folder, `npm i shiki@3.13.0 @pierre/theme@2.0.0`, then
// `npx esbuild@0.24.2 shiki.mjs --bundle --minify --format=esm --splitting --legal-comments=none --outdir=out "--chunk-names=[name]-[hash]"`
// and copy out/ there, keeping the header of ui/vendor/shiki/shiki.js on the new shiki.js.
import { createHighlighterCore } from "shiki/core";
import { createOnigurumaEngine } from "shiki/engine/oniguruma";
import pierreDark from "@pierre/theme/themes/pierre-dark.json";
import pierreLight from "@pierre/theme/themes/pierre-light.json";

// The languages code blocks highlight, with their aliases; any other shows as plain text.
const GRAMMARS = {
  "astro": [[], () => import("@shikijs/langs/astro")],
  "bat": [["batch"], () => import("@shikijs/langs/bat")],
  "c": [[], () => import("@shikijs/langs/c")],
  "clojure": [["clj"], () => import("@shikijs/langs/clojure")],
  "cmake": [[], () => import("@shikijs/langs/cmake")],
  "coffee": [["coffeescript"], () => import("@shikijs/langs/coffee")],
  "cpp": [["c++"], () => import("@shikijs/langs/cpp")],
  "csharp": [["c#","cs"], () => import("@shikijs/langs/csharp")],
  "css": [[], () => import("@shikijs/langs/css")],
  "csv": [[], () => import("@shikijs/langs/csv")],
  "dart": [[], () => import("@shikijs/langs/dart")],
  "diff": [[], () => import("@shikijs/langs/diff")],
  "docker": [["dockerfile"], () => import("@shikijs/langs/docker")],
  "dotenv": [[], () => import("@shikijs/langs/dotenv")],
  "elixir": [[], () => import("@shikijs/langs/elixir")],
  "elm": [[], () => import("@shikijs/langs/elm")],
  "erlang": [["erl"], () => import("@shikijs/langs/erlang")],
  "fish": [[], () => import("@shikijs/langs/fish")],
  "fsharp": [["f#","fs"], () => import("@shikijs/langs/fsharp")],
  "git-commit": [[], () => import("@shikijs/langs/git-commit")],
  "glsl": [[], () => import("@shikijs/langs/glsl")],
  "go": [[], () => import("@shikijs/langs/go")],
  "graphql": [["gql"], () => import("@shikijs/langs/graphql")],
  "groovy": [[], () => import("@shikijs/langs/groovy")],
  "haskell": [["hs"], () => import("@shikijs/langs/haskell")],
  "hcl": [[], () => import("@shikijs/langs/hcl")],
  "html": [[], () => import("@shikijs/langs/html")],
  "http": [[], () => import("@shikijs/langs/http")],
  "ini": [["properties"], () => import("@shikijs/langs/ini")],
  "java": [[], () => import("@shikijs/langs/java")],
  "javascript": [["js"], () => import("@shikijs/langs/javascript")],
  "jinja": [[], () => import("@shikijs/langs/jinja")],
  "json": [[], () => import("@shikijs/langs/json")],
  "json5": [[], () => import("@shikijs/langs/json5")],
  "jsonc": [[], () => import("@shikijs/langs/jsonc")],
  "jsonl": [[], () => import("@shikijs/langs/jsonl")],
  "jsx": [[], () => import("@shikijs/langs/jsx")],
  "julia": [["jl"], () => import("@shikijs/langs/julia")],
  "kotlin": [["kt","kts"], () => import("@shikijs/langs/kotlin")],
  "latex": [[], () => import("@shikijs/langs/latex")],
  "less": [[], () => import("@shikijs/langs/less")],
  "lua": [[], () => import("@shikijs/langs/lua")],
  "make": [["makefile"], () => import("@shikijs/langs/make")],
  "markdown": [["md"], () => import("@shikijs/langs/markdown")],
  "mdx": [[], () => import("@shikijs/langs/mdx")],
  "nginx": [[], () => import("@shikijs/langs/nginx")],
  "nix": [[], () => import("@shikijs/langs/nix")],
  "objective-c": [["objc"], () => import("@shikijs/langs/objective-c")],
  "ocaml": [[], () => import("@shikijs/langs/ocaml")],
  "perl": [[], () => import("@shikijs/langs/perl")],
  "php": [[], () => import("@shikijs/langs/php")],
  "powershell": [["ps","ps1"], () => import("@shikijs/langs/powershell")],
  "prisma": [[], () => import("@shikijs/langs/prisma")],
  "proto": [["protobuf"], () => import("@shikijs/langs/proto")],
  "python": [["py"], () => import("@shikijs/langs/python")],
  "r": [[], () => import("@shikijs/langs/r")],
  "regexp": [["regex"], () => import("@shikijs/langs/regexp")],
  "ruby": [["rb"], () => import("@shikijs/langs/ruby")],
  "rust": [["rs"], () => import("@shikijs/langs/rust")],
  "sass": [[], () => import("@shikijs/langs/sass")],
  "scala": [[], () => import("@shikijs/langs/scala")],
  "scss": [[], () => import("@shikijs/langs/scss")],
  "shellscript": [["bash","sh","shell","zsh"], () => import("@shikijs/langs/shellscript")],
  "shellsession": [["console"], () => import("@shikijs/langs/shellsession")],
  "solidity": [[], () => import("@shikijs/langs/solidity")],
  "sql": [[], () => import("@shikijs/langs/sql")],
  "svelte": [[], () => import("@shikijs/langs/svelte")],
  "swift": [[], () => import("@shikijs/langs/swift")],
  "terraform": [["tf","tfvars"], () => import("@shikijs/langs/terraform")],
  "toml": [[], () => import("@shikijs/langs/toml")],
  "tsx": [[], () => import("@shikijs/langs/tsx")],
  "typescript": [["ts"], () => import("@shikijs/langs/typescript")],
  "vue": [[], () => import("@shikijs/langs/vue")],
  "xml": [[], () => import("@shikijs/langs/xml")],
  "yaml": [["yml"], () => import("@shikijs/langs/yaml")],
  "zig": [[], () => import("@shikijs/langs/zig")],
};
const LANGS = new Map();
for (const [id, [aliases, load]] of Object.entries(GRAMMARS)) for (const name of [id, ...aliases]) LANGS.set(name, { id, import: load });

const highlighter = createHighlighterCore({ themes: [pierreDark, pierreLight], langs: [], engine: createOnigurumaEngine(import("shiki/wasm")) });
const loading = new Map();

window.SHIKI = {
  /** The language's id once its grammar is loaded, or null for one this bundle lacks. */
  async load(name) {
    const info = LANGS.get(name);
    if (!info) return null;
    if (!loading.has(info.id)) loading.set(info.id, highlighter.then((h) => info.import().then((m) => h.loadLanguage(m.default)).then(() => h)));
    await loading.get(info.id);
    return info.id;
  },
  /** Lines of { content, color, fontStyle } tokens, or null until the language is loaded. */
  tokens(code, name, theme = "pierre-dark") {
    const info = LANGS.get(name);
    const h = info && loadedHighlighter;
    if (!h || !h.getLoadedLanguages().includes(info.id)) return null;
    return h.codeToTokensBase(code, { lang: info.id, theme });
  },
};
let loadedHighlighter = null;
highlighter.then((h) => (loadedHighlighter = h));
window.dispatchEvent(new Event("shiki"));
