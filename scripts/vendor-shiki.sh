#!/bin/sh
# Rebuilds ui/vendor/shiki: Shiki 4.2.0 with its Oniguruma WASM engine and T3 Code's pierre-dark
# theme (@pierre/theme 2.0.0), as ES modules the diff and file views import on first use, one
# chunk per language. Needs npm and network.
set -eu
REPO=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"
npm init -y >/dev/null
npm install --silent shiki@4.2.0 @pierre/theme@2.0.0 esbuild@0.25.10
cat >entry.mjs <<'JS'
import { createHighlighterCore } from "shiki/core";
import { createOnigurumaEngine } from "shiki/engine/oniguruma";
import pierreDark from "@pierre/theme/pierre-dark";

const LANGS = {
  javascript: () => import("@shikijs/langs/javascript"), jsx: () => import("@shikijs/langs/jsx"),
  typescript: () => import("@shikijs/langs/typescript"), tsx: () => import("@shikijs/langs/tsx"),
  python: () => import("@shikijs/langs/python"), rust: () => import("@shikijs/langs/rust"),
  ruby: () => import("@shikijs/langs/ruby"), go: () => import("@shikijs/langs/go"),
  java: () => import("@shikijs/langs/java"), kotlin: () => import("@shikijs/langs/kotlin"),
  swift: () => import("@shikijs/langs/swift"), c: () => import("@shikijs/langs/c"),
  cpp: () => import("@shikijs/langs/cpp"), csharp: () => import("@shikijs/langs/csharp"),
  php: () => import("@shikijs/langs/php"), bash: () => import("@shikijs/langs/bash"),
  json: () => import("@shikijs/langs/json"), jsonc: () => import("@shikijs/langs/jsonc"),
  yaml: () => import("@shikijs/langs/yaml"), toml: () => import("@shikijs/langs/toml"),
  ini: () => import("@shikijs/langs/ini"), markdown: () => import("@shikijs/langs/markdown"),
  mdx: () => import("@shikijs/langs/mdx"), css: () => import("@shikijs/langs/css"),
  scss: () => import("@shikijs/langs/scss"), less: () => import("@shikijs/langs/less"),
  html: () => import("@shikijs/langs/html"), xml: () => import("@shikijs/langs/xml"),
  vue: () => import("@shikijs/langs/vue"), svelte: () => import("@shikijs/langs/svelte"),
  sql: () => import("@shikijs/langs/sql"), lua: () => import("@shikijs/langs/lua"),
  r: () => import("@shikijs/langs/r"), graphql: () => import("@shikijs/langs/graphql"),
  "objective-c": () => import("@shikijs/langs/objective-c"), perl: () => import("@shikijs/langs/perl"),
  dart: () => import("@shikijs/langs/dart"), elixir: () => import("@shikijs/langs/elixir"),
  zig: () => import("@shikijs/langs/zig"), scala: () => import("@shikijs/langs/scala"),
  haskell: () => import("@shikijs/langs/haskell"), hcl: () => import("@shikijs/langs/hcl"),
  proto: () => import("@shikijs/langs/proto"), diff: () => import("@shikijs/langs/diff"),
  makefile: () => import("@shikijs/langs/makefile"), docker: () => import("@shikijs/langs/docker"),
};

export async function highlighter() {
  const core = await createHighlighterCore({ themes: [pierreDark], langs: [], engine: createOnigurumaEngine(import("shiki/wasm")) });
  return {
    async loadLanguage(lang) {
      if (!LANGS[lang]) throw new Error(`no grammar for ${lang}`);
      await core.loadLanguage((await LANGS[lang]()).default);
    },
    codeToTokens: (code, options) => core.codeToTokens(code, options),
  };
}
JS
rm -rf "$REPO/ui/vendor/shiki"
npx esbuild entry.mjs --bundle --splitting --format=esm --minify --outdir="$REPO/ui/vendor/shiki" --entry-names=shiki --chunk-names=[name]-[hash] --legal-comments=none --log-level=warning
cp node_modules/shiki/LICENSE "$REPO/ui/vendor/shiki/LICENSE"
