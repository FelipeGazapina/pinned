#!/usr/bin/env node
// Rewrites the literal colors (and `color-scheme: dark`) in the app view's CSS (ui/gui*.css) into
// the theme tokens that ui/gui-theme.css defines, so light mode and the palettes reach every rule,
// and the ones in ui/style.css that the app view's Settings shows into tokens with a fallback.
//
//   node scripts/theme-tokens.mjs            rewrite in place, then list any color it can't place
//   node scripts/theme-tokens.mjs --check    change nothing; exit 1 if a literal is left to rewrite
//   node scripts/theme-tokens.mjs ui/gui-composer.css   only these files
//
// Safe to rerun: a rewritten value holds no literal, and the literals it keeps on purpose (masks,
// scrims, web page backgrounds, the terminal view's rules) match a KEEP rule again. Custom property
// definitions are skipped: they are the tokens. A literal no rule matches is reported and left as
// it is, with exit status 1, so a new rule gets written rather than a guess.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const UI = path.join(path.dirname(fileURLToPath(import.meta.url)), "../ui");
const COLOR = /rgba?\([^()]*\)|#[0-9a-fA-F]{3,8}\b/g;
const KEEP = Symbol("keep");

// A translucent use reads the token's channels, so it computes to the same rgba() it replaces.
const mix = (token, a) => (a >= 1 ? `var(--${token})` : `rgb(var(--${token}-rgb) / ${+a.toFixed(3)})`);

function parse(lit) {
  if (lit[0] === "#") {
    let h = lit.slice(1);
    if (h.length <= 4) h = [...h].map((c) => c + c).join("");
    const n = (i) => parseInt(h.slice(i, i + 2), 16);
    return { rgb: `${n(0)},${n(2)},${n(4)}`, a: h.length === 8 ? +(n(6) / 255).toFixed(3) : 1 };
  }
  const [r, g, b, a = "1"] = lit.replace(/^rgba?\(|\)$/g, "").split(/[\s,/]+/).filter(Boolean);
  return { rgb: `${r},${g},${b}`, a: a.endsWith("%") ? parseFloat(a) / 100 : +a };
}

const isMask = (c) => /mask/.test(c.prop);
const isShadow = (c) => /shadow/.test(c.prop);
const isBorder = (c) => /^(border|outline|column-rule)/.test(c.prop);
const isBg = (c) => /^background/.test(c.prop);
const sel = (re) => (c) => re.test(c.sel);

// First match wins. `to` is a token name (mixed at the literal's alpha), a string, KEEP, or a function.
const RULES = [
  // The terminal view keeps its own dark look.
  { when: sel(/data-mode="terminal"/), to: KEEP },
  { when: isMask, to: KEEP },
  // Web pages, PDFs and the switch knob are white in both appearances.
  { when: (c) => c.rgb === "255,255,255" && isBg(c) && c.a === 1, to: KEEP },
  // The primary button's top highlight, as T3's button.tsx draws it in both appearances.
  { when: (c) => c.rgb === "255,255,255" && isShadow(c) && /primary/.test(c.sel), to: KEEP },
  { when: (c) => c.rgb === "255,255,255" && c.prop === "color", to: "on-primary" },
  { when: (c) => c.rgb === "255,255,255" && isBg(c) && c.a === 0.04, to: "var(--g-accent)" },
  { when: (c) => c.rgb === "255,255,255" && isBg(c) && c.a === 0.06, to: "var(--g-hover)" },
  { when: (c) => c.rgb === "255,255,255" && c.prop === "border-right" && /^#sidebar$/.test(c.sel), to: "var(--g-side-border)" },
  { when: (c) => c.rgb === "255,255,255" && isBorder(c) && c.a === 0.06, to: "var(--g-border)" },
  { when: (c) => c.rgb === "255,255,255" && isBorder(c) && c.a === 0.08, to: "var(--g-input)" },
  { when: (c) => c.rgb === "255,255,255" && c.prop === "scrollbar-color", to: "var(--g-scrollbar)" },
  { when: (c) => c.rgb === "255,255,255", to: (c) => mix("g-ink", c.a) },
  // Dialog scrims stay black, as T3's are in both appearances.
  { when: (c) => c.rgb === "0,0,0" && isBg(c) && c.a !== 0.3, to: KEEP },
  { when: (c) => c.rgb === "0,0,0" && isBg(c), to: "var(--g-field)" },
  { when: (c) => c.rgb === "0,0,0" && isShadow(c), to: (c) => mix("g-shadow", c.a) },
  { when: (c) => c.rgb === "10,10,10", to: "canvas" },
  { when: (c) => c.rgb === "17,17,17", to: "raised" },
  { when: (c) => c.rgb === "20,20,20" && /bubble/.test(c.sel), to: "message" },
  { when: (c) => c.rgb === "20,20,20", to: "overlay" },
  { when: (c) => c.rgb === "28,28,28", to: "color-mix(in srgb, var(--g-ink) 4%, var(--g-overlay))" },
  { when: (c) => c.rgb === "25,25,25", to: "var(--g-border)" },
  { when: (c) => c.rgb === "30,30,30", to: "control" },
  { when: (c) => c.rgb === "245,245,245", to: "fg" },
  { when: (c) => c.rgb === "129,129,129", to: "muted" },
  { when: (c) => c.rgb === "163,163,163" && /hljs/.test(c.sel), to: "hl-param" },
  { when: (c) => c.rgb === "163,163,163", to: "side-muted" },
  { when: (c) => c.rgb === "241,243,247", to: "side-fg" },
  { when: (c) => c.rgb === "52,107,241", to: "primary" },
  { when: (c) => c.rgb === "48,97,217", to: "primary-hover" },
  { when: (c) => c.rgb === "81,162,255", to: "info" },
  { when: (c) => c.rgb === "254,154,0", to: "warning" },
  { when: (c) => ["251,65,74", "239,68,68"].includes(c.rgb), to: "error" },
  { when: (c) => ["110,231,183", "94,233,181"].includes(c.rgb), to: "pr-open" },
  { when: (c) => ["196,181,253", "196,180,255"].includes(c.rgb), to: "pr-merged" },
  { when: (c) => ["252,165,165", "255,162,162"].includes(c.rgb), to: "pr-closed" },
  { when: (c) => ["161,161,170", "159,159,169"].includes(c.rgb), to: "pr-draft" },
  { when: (c) => c.rgb === "245,158,11", to: "pr-pending" },
  { when: (c) => c.rgb === "105,177,255", to: "blue" },
  { when: (c) => c.rgb === "25,40,60", to: "color-mix(in srgb, var(--g-blue) 15%, var(--g-canvas))" },
  { when: (c) => c.rgb === "129,166,255", to: "link" },
  { when: (c) => c.rgb === "229,229,229", to: "code-fg" },
  { when: (c) => c.rgb === "250,250,250", to: "hl-fg" },
  { when: (c) => c.rgb === "166,166,166", to: "line-no" },
  { when: (c) => c.rgb === "138,106,24", to: "match-bg" },
  { when: (c) => c.rgb === "248,248,248", to: "match-fg" },
  { when: (c) => c.rgb === "241,169,44", to: "match-active-bg" },
  { when: (c) => c.rgb === "29,26,18", to: "match-active-fg" },
  { when: (c) => ["9,21,15", "10,18,32"].includes(c.rgb), to: "add-row" },
  { when: (c) => ["11,26,18", "12,22,40"].includes(c.rgb), to: "add-no" },
  { when: (c) => ["25,13,14", "28,18,9"].includes(c.rgb), to: "del-row" },
  { when: (c) => ["29,15,16", "34,21,10"].includes(c.rgb), to: "del-no" },
  // pierre-dark's syntax colors, as hljs classes in the diff.
  { when: (c) => c.rgb === "255,103,141", to: "hl-keyword" },
  { when: (c) => c.rgb === "94,204,113", to: "hl-string" },
  { when: (c) => c.rgb === "104,205,242", to: "hl-number" },
  { when: (c) => c.rgb === "157,106,251", to: "hl-function" },
  { when: (c) => c.rgb === "213,104,234", to: "hl-type" },
  { when: (c) => c.rgb === "255,163,89", to: "hl-variable" },
  { when: (c) => c.rgb === "99,99,99", to: "hl-punctuation" },
  { when: (c) => c.rgb === "115,115,115", to: "hl-comment" },
  { when: (c) => c.rgb === "255,133,94", to: "hl-tag" },
  // Claude's orange, the red conflict mark, and the editor's selection read on both appearances.
  { when: (c) => ["217,119,87", "220,38,38", "0,159,255"].includes(c.rgb), to: KEEP },
];

// The terminal view's stylesheet: it keeps its dark look, so each token falls back to the literal it
// replaces, and gui-theme.css sets the --s-* tokens only while the app view shows Settings in light
// mode or a palette.
const fallback = (token) => (c) => `var(--s-${token}, ${c.lit})`;
const STYLE_RULES = [
  { when: isMask, to: KEEP },
  { when: (c) => c.rgb === "255,255,255" && c.a < 1, to: (c) => `rgb(var(--s-ink-rgb, 255 255 255) / ${+c.a.toFixed(3)})` },
  { when: (c) => c.rgb === "0,0,0" && isBg(c), to: fallback("field") },
  { when: (c) => c.rgb === "15,17,21" && c.a < 1, to: (c) => `rgb(var(--s-bg-rgb, 15 17 21) / ${+c.a.toFixed(3)})` },
  { when: (c) => c.rgb === "15,17,21" && c.prop !== "color", to: fallback("bg") },
  { when: (c) => ["25,28,34", "20,23,28", "35,39,46"].includes(c.rgb), to: fallback("raised") },
  { when: (c) => ["61,67,77", "74,81,92"].includes(c.rgb), to: fallback("line") },
  { when: (c) => c.rgb === "255,107,107", to: fallback("error") },
  // Text on a colored fill, shadows, and the brand, status and error fills read on both appearances.
  { when: () => true, to: KEEP },
];

function rewrite(c, rules) {
  const rule = rules.find((r) => r.when(c));
  if (!rule) return null;
  if (rule.to === KEEP) return KEEP;
  if (typeof rule.to === "function") return rule.to(c);
  return rule.to.includes("(") ? rule.to : mix(`g-${rule.to}`.replace(/^g-hl-/, "hl-"), c.a);
}

/** Each color literal outside comments, with the property and selector it sits in. */
function literals(src) {
  const shadow = src.replace(/\/\*[\s\S]*?\*\//g, (m) => " ".repeat(m.length));
  const out = [];
  for (const m of shadow.matchAll(COLOR)) {
    let j = m.index;
    for (let depth = 0; j > 0; j--) {
      const ch = shadow[j - 1];
      if (ch === ")") depth++;
      else if (ch === "(") depth--;
      else if (depth <= 0 && ";{}".includes(ch)) break;
    }
    const prop = shadow.slice(j, m.index).split(":")[0].trim();
    let k = j - 1;
    while (k > 0 && shadow[k] !== "{") k--;
    let s = k;
    while (s > 0 && !"{};".includes(shadow[s - 1])) s--;
    out.push({ index: m.index, lit: m[0], prop, sel: shadow.slice(s, k).trim().replace(/\s+/g, " "), line: src.slice(0, m.index).split("\n").length });
  }
  return out;
}

const check = process.argv.includes("--check");
const named = process.argv.slice(2).filter((a) => !a.startsWith("--"));
const files = named.length ? named.map((f) => path.resolve(f)) : [...fs.readdirSync(UI).filter((f) => /^gui.*\.css$/.test(f) && f !== "gui-theme.css"), "style.css"].map((f) => path.join(UI, f));

let pending = 0;
const unknown = [];
for (const file of files) {
  const src = fs.readFileSync(file, "utf8");
  let next = src;
  const rules = path.basename(file) === "style.css" ? STYLE_RULES : RULES;
  for (const l of literals(src).reverse()) {
    if (l.prop.startsWith("--") || /var\(--s-[\w-]+, $/.test(src.slice(l.index - 40, l.index))) continue;
    const to = rewrite({ ...l, ...parse(l.lit) }, rules);
    if (to === KEEP) continue;
    if (to === null) {
      unknown.push(`${path.basename(file)}:${l.line} ${l.prop}: ${l.lit} in ${l.sel}`);
      continue;
    }
    pending++;
    next = next.slice(0, l.index) + to + next.slice(l.index + l.lit.length);
  }
  if (rules === RULES) next = next.replace(/color-scheme: dark\b/g, () => (pending++, "color-scheme: var(--g-scheme)"));
  if (!check && next !== src) fs.writeFileSync(file, next);
}
// Every token a rewritten rule reads must be defined, by gui-theme.css or another gui*.css file.
const undefinedTokens = new Set();
const sheets = fs.readdirSync(UI).filter((f) => /^gui.*\.css$/.test(f)).map((f) => fs.readFileSync(path.join(UI, f), "utf8"));
const defined = new Set(sheets.flatMap((css) => [...css.matchAll(/(--(?:g|hl)-[\w-]+):/g)].map((m) => m[1])));
for (const file of files) {
  for (const [, name] of fs.readFileSync(file, "utf8").matchAll(/var\((--(?:g|hl)-[\w-]+)/g)) {
    if (!defined.has(name)) undefinedTokens.add(`${path.basename(file)} reads ${name}, which ui/gui-theme.css doesn't define`);
  }
}
for (const u of unknown) console.log(`no rule for ${u}`);
for (const u of undefinedTokens) console.log(u);
if (check && pending) console.log(`${pending} values left to rewrite; run node scripts/theme-tokens.mjs`);
else if (!check) console.log(`rewrote ${pending} values in ${files.length} files`);
process.exit(unknown.length || undefinedTokens.size || (check && pending) ? 1 : 0);
