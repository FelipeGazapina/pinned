// Screenshots of the Teams entry screens (ui/gui-teams.js) in headless Chromium, dark and light, at 1360 px,
// next to the design prototypes when PROTO_DIR holds them (Entrada.html, Criar.html, Entrar.html).
//
//   PW_DIR=/path/with/node_modules/playwright-core  PROTO_DIR=/path/to/proto  OUT=/path/to/out \
//     node scripts/teams-dev/ui-shots-entry.mjs
//
// The real ui/ is served by a static server started here and stopped at the end; the hub is faked with
// Playwright's routeWebSocket, answering the team ops with invented data. A prototype is a template
// whose runtime (support.js) is not shipped, so a tiny stand-in for it is served from this file: it knows
// <sc-if>, <sc-for>, {{ }} and the on* handlers, and keeps the page's state in window.__dc.
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { extname, join, normalize, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const UI = resolve(here, "../../ui");
const OUT = resolve(process.env.OUT ?? "teams-entry-shots");
const PROTO = process.env.PROTO_DIR ? resolve(process.env.PROTO_DIR) : null;
const CHROMIUM = process.env.CHROMIUM ?? (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined);
const WIDTH = Number(process.env.WIDTH ?? 1360);
const HEIGHT = Number(process.env.HEIGHT ?? 900);
const require = createRequire(process.env.PW_DIR ? join(resolve(process.env.PW_DIR), "package.json") : import.meta.url);
const { chromium } = require(process.env.PW_DIR ? resolve(process.env.PW_DIR) : "playwright-core");
mkdirSync(OUT, { recursive: true });

// ---- Invented data ---------------------------------------------------------------------------------------
const ME = { id: "a1", email: "ana.prado@example.com" };
const OWN_PROJECTS = [
  ["storefront", "https://github.com/northstudio/storefront"],
  ["orders-api", "git@github.com:northstudio/orders-api.git"],
  ["delivery-app", "https://github.com/northstudio/delivery-app"],
  ["personal-site", "https://github.com/ana/personal-site"],
];
const THEIR_PROJECTS = [
  { key: "github.com/northstudio/storefront", name: "storefront", repo: "https://github.com/northstudio/storefront", member: "a2" },
  { key: "github.com/northstudio/orders-api", name: "orders-api", repo: "https://github.com/northstudio/orders-api", member: "a2" },
  { key: "github.com/northstudio/delivery-app", name: "delivery-app", repo: "https://github.com/northstudio/delivery-app", member: "a3" },
];
const team = (id, name, role, credential = {}) => ({ id, name, owner: role === "owner" ? "a1" : "a2", role, credential: { upstash: false, claude: false, github: false, slack: false, ...credential } });

/** A hub that answers team calls from a mutable world. */
function fakeHub(world) {
  return (call) => {
    if (call.cmd !== "team") return call.cmd === "get_settings" ? {} : null;
    const { op } = call.op;
    world.frames.push(call);
    if (op === "list") return world.teams;
    if (op === "create") {
      const t = team("t-new", call.op.name, "owner");
      world.teams = [...world.teams, t];
      return t;
    }
    if (op === "join") {
      if (call.op.code !== world.code) throw "that invite code is not valid, or was used already";
      const t = team("t-join", "North Studio", "member", { upstash: true, claude: true, github: true });
      world.teams = [...world.teams, t];
      return t;
    }
    if (op === "invite") return { code: world.codes.shift() ?? "QRST-UVWX", hint: "…UVWX" };
    if (op === "projects") return THEIR_PROJECTS;
    if (op === "attach_project" || op === "rename" || op === "detach_project") return null;
    if (op === "hub") {
      const key = { connect_upstash: "upstash", set_box_claude_token: "claude", connect_github: "github" }[call.op.call.cmd];
      const t = world.teams.find((x) => x.id === call.team);
      if (key && t) t.credential[key] = true;
      return null;
    }
    return null;
  };
}

// ---- Static server ---------------------------------------------------------------------------------------
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml", ".png": "image/png", ".woff2": "font/woff2", ".webmanifest": "application/manifest+json" };
function serve(root, extra = {}) {
  const server = createServer((req, res) => {
    const path = decodeURIComponent(new URL(req.url, "http://x").pathname);
    if (path in extra) return res.writeHead(200, { "content-type": "text/javascript" }).end(extra[path]);
    if (path === "/api/version") return res.writeHead(200).end("0.0.0-dev");
    const file = normalize(join(root, path === "/" ? "index.html" : path));
    if (!file.startsWith(root) || !existsSync(file) || !extname(file)) return res.writeHead(404).end();
    res.writeHead(200, { "content-type": TYPES[extname(file)] ?? "application/octet-stream" }).end(readFileSync(file));
  });
  return new Promise((ok) => server.listen(0, "127.0.0.1", () => ok({ server, url: `http://127.0.0.1:${server.address().port}` })));
}

// ---- A stand-in for the prototypes' missing runtime -------------------------------------------------------
const SUPPORT = String.raw`
class DCLogic { constructor(props) { this.props = props; this.state = {}; }
  setState(p) { Object.assign(this.state, typeof p === "function" ? p(this.state) : p); window.__dcRender(); } }
window.DCLogic = DCLogic;
addEventListener("DOMContentLoaded", () => {
  const host = document.querySelector("x-dc");
  for (const h of host.querySelectorAll("helmet")) { for (const s of h.querySelectorAll("style")) document.head.append(s); h.remove(); }
  const tpl = host.cloneNode(true);
  const code = document.querySelector("script[data-dc-script]").textContent;
  const Component = new Function("DCLogic", code + "; return Component;")(DCLogic);
  const logic = new Component({}); window.__dc = logic;
  const ev = (expr, scope) => new Function("scope", "with(scope){return (" + expr + ")}")(scope);
  const sub = (str, scope) => str.replace(/\{\{\s*([\s\S]+?)\s*\}\}/g, (_, e) => String(ev(e, scope)));
  const whole = (str) => { const m = /^\{\{\s*([\s\S]+?)\s*\}\}$/.exec(str.trim()); return m ? m[1] : null; };
  function render(node, scope, into) {
    if (node.nodeType === 3) return into.append(document.createTextNode(sub(node.textContent, scope)));
    if (node.nodeType !== 1) return;
    const tag = node.tagName.toLowerCase();
    if (tag === "sc-if") { if (ev(whole(node.getAttribute("value")), scope)) for (const c of node.childNodes) render(c, scope, into); return; }
    if (tag === "sc-for") { const as = node.getAttribute("as"); for (const item of ev(whole(node.getAttribute("list")), scope)) for (const c of node.childNodes) render(c, { ...scope, [as]: item }, into); return; }
    const el = document.createElement(tag);
    for (const a of node.attributes) {
      if (a.name.startsWith("hint-")) continue;
      const w = whole(a.value);
      if (/^on/.test(a.name) && w) { el[a.name.toLowerCase()] = ev(w, scope); continue; }
      el.setAttribute(a.name, sub(a.value, scope));
    }
    for (const c of node.childNodes) render(c, scope, el);
    into.append(el);
  }
  window.__dcRender = () => { const out = document.createElement("div"); out.id = "dc-root"; for (const c of tpl.childNodes) render(c, logic.renderVals(), out); const old = document.getElementById("dc-root"); old ? old.replaceWith(out) : document.body.append(out); };
  host.remove(); window.__dcRender();
});`;

// ---- Screens ---------------------------------------------------------------------------------------------
const browser = await chromium.launch({ executablePath: CHROMIUM, args: ["--no-sandbox"] });
const app = await serve(UI);
const proto = PROTO ? await serve(PROTO, { "/support.js": SUPPORT }) : null;
const shots = [];
/** The page the script is driving, so a step that times out leaves a picture of where it was. */
let current = null;
process.on("uncaughtException", async (e) => {
  console.error(e.message);
  await current?.screenshot({ path: join(OUT, "failed.png") }).catch(() => {});
  process.exit(1);
});

async function newApp(scheme, world) {
  const context = await browser.newContext({ viewport: { width: WIDTH, height: HEIGHT }, colorScheme: scheme, serviceWorkers: "block" });
  const page = await context.newPage();
  current = page;
  page.on("pageerror", (e) => /PinnedProjects/.test(e.message) || console.error("pageerror:", e.message));
  const answer = fakeHub(world);
  await page.routeWebSocket(/\/ws\/client/, (ws) => {
    ws.onMessage((raw) => {
      const frame = JSON.parse(String(raw));
      if (frame.type === "auth") return ws.send(JSON.stringify({ type: "hello", account: ME, machines: [], tabs: {} }));
      if (frame.type !== "hub") return;
      try {
        ws.send(JSON.stringify({ type: "reply", id: frame.id, ok: answer(frame.call) }));
      } catch (e) {
        ws.send(JSON.stringify({ type: "reply", id: frame.id, err: String(e) }));
      }
    });
  });
  await page.addInitScript(() => {
    localStorage.setItem("hub-token", "fake");
    localStorage.setItem("gui-mode", "gui");
  });
  await page.goto(app.url);
  await page.waitForFunction(() => document.body.dataset.hub === "online");
  // The person's own projects, as the projects page would list them.
  await page.evaluate((own) => {
    globalThis.globalProjectsCatalog = () => ({ projects: own.map(([name, repo]) => ({ slug: name, name, repo, machines: [] })), machines: [] });
  }, OWN_PROJECTS);
  return { context, page };
}
const freshWorld = () => ({ teams: [team("t1", "Studio Two", "owner", { upstash: true }), team("t3", "Side Gig", "member")], frames: [], code: "ABCD-EFGH", codes: ["WXYZ-2481", "QRST-6093"] });
const shot = async (page, name) => {
  const file = join(OUT, `${name}.png`);
  await page.screenshot({ path: file });
  shots.push(file);
};
const type = (page, selector, text) => page.fill(selector, text);
const click = (page, text) => page.getByRole("button", { name: text, exact: false }).first().click();

for (const scheme of ["dark", "light"]) {
  // Entrada: the account menu with the Where you work block, then a team chosen.
  {
    const { context, page } = await newApp(scheme, freshWorld());
    await page.click("#account-pages");
    await page.waitForSelector(".tm-row >> nth=1");
    await shot(page, `app-entrada-menu-${scheme}`);
    await page.locator(".tm-row", { hasText: "Studio Two" }).click();
    await page.click("#account-pages");
    await shot(page, `app-entrada-team-${scheme}`);
    await context.close();
  }
  // Criar: every step, the key form, an invite code, and the end.
  {
    const world = freshWorld();
    world.teams = [];
    const { context, page } = await newApp(scheme, world);
    await page.click("#account-pages");
    await page.locator(".tm-create").click();
    await page.waitForSelector(".tm-wiz");
    await shot(page, `app-criar-1-name-${scheme}`);
    await type(page, "#tm-team-name", "North Studio");
    await click(page, "Continue");
    await page.waitForSelector("text=Step 2 of 4");
    await page.locator(".tm-check").nth(0).click();
    await page.locator(".tm-check").nth(1).click();
    await shot(page, `app-criar-2-projects-${scheme}`);
    await click(page, "Continue");
    await page.waitForSelector("text=Step 3 of 4");
    await shot(page, `app-criar-3-ai-${scheme}`);
    await page.locator('[data-tm="connect"][data-cred="upstash"]').click();
    await shot(page, `app-criar-3-ai-form-${scheme}`);
    await page.fill('input[data-field="key"]', "box_demo_key");
    await page.fill('input[data-field="claude_token"]', "sk-ant-oat01-demo");
    await page.locator(".tm-form button[type=submit]").click();
    await page.waitForSelector("text=Connected");
    await shot(page, `app-criar-3-ai-connected-${scheme}`);
    await click(page, "Connect later");
    await page.waitForSelector("text=Step 4 of 4");
    await page.locator('[data-tm="invite"]').click();
    await page.waitForSelector(".tm-code");
    await page.locator('[data-tm="invite"]').click();
    await page.waitForFunction(() => document.querySelectorAll(".tm-code").length === 2);
    await shot(page, `app-criar-4-people-${scheme}`);
    await click(page, "Finish");
    await page.waitForSelector("text=North Studio is ready");
    await shot(page, `app-criar-5-done-${scheme}`);
    await context.close();
  }
  // Entrar: the code, a bad code, then the page after joining.
  {
    const { context, page } = await newApp(scheme, freshWorld());
    await page.click("#account-pages");
    await page.locator(".tm-join").click();
    await page.waitForSelector("#tm-join-code");
    await shot(page, `app-entrar-0-code-${scheme}`);
    await type(page, "#tm-join-code", "WRONG-CODE");
    await page.locator(".tm-form button[type=submit]").click();
    await page.waitForSelector(".tm-error");
    await shot(page, `app-entrar-0-code-error-${scheme}`);
    await type(page, "#tm-join-code", "ABCD-EFGH");
    await page.locator(".tm-form button[type=submit]").click();
    await page.waitForSelector("text=You joined");
    await shot(page, `app-entrar-1-pick-${scheme}`);
    await context.close();
  }
}

// The prototypes, in the states the app screens are compared with.
if (proto) {
  const states = {
    Entrada: [["menu", {}]],
    Criar: [["1", { step: 1 }], ["2", { step: 2 }], ["3", { step: 3 }], ["3-connected", { step: 3, connected: true }], ["4", { step: 4 }], ["5", { step: 5 }]],
    Entrar: [["pick", {}]],
  };
  const context = await browser.newContext({ viewport: { width: WIDTH, height: HEIGHT } });
  const page = await context.newPage();
  for (const [name, list] of Object.entries(states)) {
    for (const [label, state] of list) {
      await page.goto(`${proto.url}/${name}.html`);
      await page.waitForFunction(() => window.__dc);
      await page.evaluate((s) => window.__dc.setState(s), state);
      await shot(page, `proto-${name.toLowerCase()}-${label}-dark`);
    }
  }
  await context.close();
}

await browser.close();
app.server.close();
proto?.server.close();
console.log(`${shots.length} screenshots in ${OUT}`);
