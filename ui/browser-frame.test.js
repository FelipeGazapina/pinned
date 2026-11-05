import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function uiSource(name) {
  return readFileSync(new URL(name, import.meta.url), "utf8");
}

function sourceBetween(src, startAt, endAt) {
  const start = src.indexOf(startAt);
  const end = src.indexOf(endAt, start + startAt.length);
  if (start < 0 || end < 0) throw new Error(`missing slice ${startAt}`);
  return src.slice(start, end);
}

const failureScreen = vm.createContext({ IS_MAC: true, URL });
vm.runInContext(sourceBetween(uiSource("./settings.js"), "const esc =", "\n"), failureScreen);
vm.runInContext(sourceBetween(uiSource("./gui-panel.js"), "const hostOf =", "\n};\n") + "\n};", failureScreen);
vm.runInContext(sourceBetween(uiSource("./gui-browser.js"), "const PAGE_ERRORS =", "function showFailure"), failureScreen);

const unframedHtml = (failure) => vm.runInContext("failureHtml", failureScreen)({ failure: { kind: "unframed", ...failure } });

test("a page that refuses a frame names its host and rule and offers the browser", () => {
  const html = unframedHtml({ url: "https://duckduckgo.com/?q=miro", rule: "X-Frame-Options: SAMEORIGIN" });
  assert.match(html, /<h1>This site can’t be shown here<\/h1>/);
  assert.match(html, /<b>duckduckgo\.com<\/b>/);
  assert.match(html, /<p class="code">X-Frame-Options: SAMEORIGIN<\/p>/);
  assert.match(html, /data-failure="open"[^>]*>Open in browser</);
});

test("the rule comes from a page's response, so its markup stays text", () => {
  const html = unframedHtml({ url: "https://evil.example/", rule: "Content-Security-Policy: frame-ancestors <img src=x onerror=alert(1)>" });
  assert.equal(html.includes("<img"), false);
  assert.match(html, /frame-ancestors &#60;img src=x onerror=alert\(1\)&#62;/);
});
