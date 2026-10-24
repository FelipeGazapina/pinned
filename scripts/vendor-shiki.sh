#!/bin/sh
# Rebuilds ui/vendor/shiki: Shiki 4.2.0 with its Oniguruma WASM engine and T3 Code's pierre-dark
# theme (@pierre/theme 2.0.0), as ES modules the diff and file views and the thread's code blocks
# import on first use, one chunk per language Shiki bundles. Needs npm and network.
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
import { bundledLanguages } from "shiki/langs";
import pierreDark from "@pierre/theme/pierre-dark";

// Every grammar Shiki bundles, by id and alias, as T3 Code highlights code blocks in any of them.
const LANGS = bundledLanguages;

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
