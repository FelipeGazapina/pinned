// A minimal stand-in for the design canvas runtime the .dc.html boards load: {{ expr }} in text and
// attributes, <sc-if value>, <sc-for list as>, <dc-import name> (a sibling board mounted with its
// attributes as props), on* handlers, data-props defaults, and a DCLogic base with setState.
//
// When the board is drawn, <html data-dc> reads "ready", or "error" with the reasons in
// window.__dcErrors: a hole that threw, or an import that could not load. dc-import fetches its
// sibling, so a board that imports one must be served over http, not opened as a file.
class DCLogic {
  constructor(props) {
    this.props = props;
    this.state = {};
  }
  setState(patch) {
    Object.assign(this.state, typeof patch === "function" ? patch(this.state) : patch);
    window.__dcRender?.();
  }
}
window.DCLogic = DCLogic;
window.__dcErrors = [];

const ONLY = /^\s*\{\{([\s\S]+?)\}\}\s*$/;
const HOLE = /\{\{([\s\S]+?)\}\}/g;

const evalIn = (expr, scope) => {
  try {
    return new Function("scope", `with (scope) { return (${expr}); }`)(scope);
  } catch (e) {
    window.__dcErrors.push(`{{${expr.trim()}}}: ${e.message}`);
    return undefined;
  }
};
const interp = (text, scope) => text.replace(HOLE, (_, e) => {
  const v = evalIn(e, scope);
  return v == null ? "" : String(v);
});

/** The parts of a board document: what its <helmet> adds to <head>, its template, and its logic. */
function parseBoard(doc) {
  const root = doc.querySelector("x-dc");
  const helmet = root.querySelector(":scope > helmet");
  const head = helmet ? [...helmet.children] : [];
  helmet?.remove();
  const template = [...root.childNodes];
  const scriptEl = doc.querySelector("script[data-dc-script]");
  const propsSpec = JSON.parse(scriptEl.dataset.props || "{}");
  const defaults = {};
  for (const [k, v] of Object.entries(propsSpec)) if (!k.startsWith("$") && v && "default" in v) defaults[k] = v.default;
  const Component = new Function("DCLogic", `${scriptEl.textContent}\nreturn Component;`)(DCLogic);
  return { head, template, propsSpec, defaults, Component };
}

const boards = new Map();
function loadBoard(name) {
  if (!boards.has(name)) {
    boards.set(name, (async () => {
      const res = await fetch(new URL(`./${name}.dc.html`, location.href));
      if (!res.ok) throw new Error(`${name}.dc.html: HTTP ${res.status}`);
      const board = parseBoard(new DOMParser().parseFromString(await res.text(), "text/html"));
      for (const el of board.head) document.head.appendChild(document.importNode(el, true));
      return board;
    })());
  }
  return boards.get(name);
}

const importsIn = (nodes) => nodes.flatMap((n) => (n.nodeType === 1 ? [...(n.matches("dc-import") ? [n] : []), ...n.querySelectorAll("dc-import")] : []));
async function preload(nodes) {
  const names = [...new Set(importsIn(nodes).map((n) => n.getAttribute("name")))];
  await Promise.all(names.map(async (name) => {
    try {
      await preload((await loadBoard(name)).template);
    } catch (e) {
      window.__dcErrors.push(`<dc-import name="${name}">: ${e.message}`);
    }
  }));
}

// An imported board keeps one logic instance per <dc-import> element, so its state survives a re-render.
const instances = new WeakMap();
const loaded = new Map();

function importProps(node, scope, spec) {
  const props = {};
  for (const { name, value } of node.attributes) {
    if (name === "name" || name.startsWith("hint-")) continue;
    const only = value.match(ONLY);
    let v = only ? evalIn(only[1], scope) : value.includes("{{") ? interp(value, scope) : value;
    if (spec[name]?.editor === "boolean" && (v === "true" || v === "false")) v = v === "true";
    props[name] = v;
  }
  return props;
}

function renderImport(node, scope, out) {
  const name = node.getAttribute("name");
  const board = loaded.get(name);
  if (!board) return;
  const props = { ...board.defaults, ...importProps(node, scope, board.propsSpec) };
  let logic = instances.get(node);
  if (!logic) instances.set(node, (logic = new board.Component(props)));
  logic.props = props;
  renderAll(board.template, logic.renderVals(), out);
}

function renderNode(node, scope, out) {
  if (node.nodeType === 3) {
    out.appendChild(document.createTextNode(node.data.includes("{{") ? interp(node.data, scope) : node.data));
    return;
  }
  if (node.nodeType !== 1) return;
  const tag = node.tagName.toLowerCase();
  if (tag === "dc-import") return renderImport(node, scope, out);
  if (tag === "sc-if") {
    if (evalIn(node.getAttribute("value").match(ONLY)[1], scope)) renderAll(node.childNodes, scope, out);
    return;
  }
  if (tag === "sc-for") {
    const list = evalIn(node.getAttribute("list").match(ONLY)[1], scope) || [];
    const as = node.getAttribute("as");
    for (const item of list) {
      const inner = Object.create(scope);
      inner[as] = item;
      renderAll(node.childNodes, inner, out);
    }
    return;
  }
  const el = node.namespaceURI === "http://www.w3.org/2000/svg" ? document.createElementNS(node.namespaceURI, node.tagName) : document.createElement(tag);
  let selectValue;
  for (const { name, value } of node.attributes) {
    const only = value.match(ONLY);
    if (name.startsWith("on") && only) {
      const fn = evalIn(only[1], scope);
      if (typeof fn === "function") el.addEventListener(name.slice(2), fn);
      continue;
    }
    if (only) {
      const v = evalIn(only[1], scope);
      if (name === "value" && tag === "select") selectValue = v;
      else if (name === "value") el.setAttribute("value", v ?? "");
      else if (v === false || v == null) {
        if (name.startsWith("aria-")) el.setAttribute(name, "false");
      } else el.setAttribute(name, v === true ? (name.startsWith("aria-") ? "true" : "") : String(v));
      continue;
    }
    el.setAttribute(name, value.includes("{{") ? interp(value, scope) : value);
  }
  renderAll(node.childNodes, scope, el);
  if (selectValue !== undefined) el.value = selectValue;
  out.appendChild(el);
}

function renderAll(nodes, scope, out) {
  for (const c of [...nodes]) renderNode(c, scope, out);
}

document.addEventListener("DOMContentLoaded", async () => {
  const root = document.querySelector("x-dc");
  const board = parseBoard(document);
  for (const el of board.head) document.head.appendChild(el);
  const template = document.createElement("div");
  template.append(...board.template);
  await preload([...template.childNodes]);
  for (const [name, pending] of boards) {
    const done = await pending.catch(() => null);
    if (done) loaded.set(name, done);
  }
  const logic = new board.Component({ ...board.defaults });

  window.__dcRender = () => {
    const frag = document.createDocumentFragment();
    renderAll(template.childNodes, logic.renderVals(), frag);
    root.replaceChildren(frag);
  };
  window.__dcRender();
  window.__dc = logic;
  window.__dcPreview = board.propsSpec.$preview ?? null;
  document.documentElement.dataset.dc = window.__dcErrors.length ? "error" : "ready";
});
