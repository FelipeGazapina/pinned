// A minimal stand-in for the design canvas runtime the .dc.html boards load: {{ expr }} in text and
// attributes, <sc-if value>, <sc-for list as>, on* handlers, and a DCLogic base with setState.
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

document.addEventListener("DOMContentLoaded", () => {
  const root = document.querySelector("x-dc");
  const helmet = root.querySelector("helmet");
  if (helmet) {
    for (const el of [...helmet.children]) document.head.appendChild(el);
    helmet.remove();
  }
  const template = document.createElement("div");
  for (const n of [...root.childNodes]) template.appendChild(n);
  const scriptEl = document.querySelector("script[data-dc-script]");
  const propsSpec = JSON.parse(scriptEl.dataset.props || "{}");
  const props = {};
  for (const [k, v] of Object.entries(propsSpec)) if (!k.startsWith("$") && "default" in v) props[k] = v.default;
  const Component = new Function("DCLogic", `${scriptEl.textContent}\nreturn Component;`)(DCLogic);
  const logic = new Component(props);

  const evalIn = (expr, scope) => new Function("scope", `with (scope) { return (${expr}); }`)(scope);
  const ONLY = /^\s*\{\{([\s\S]+?)\}\}\s*$/;
  const interp = (text, scope) => text.replace(/\{\{([\s\S]+?)\}\}/g, (_, e) => {
    const v = evalIn(e, scope);
    return v == null ? "" : String(v);
  });

  function renderNode(node, scope, out) {
    if (node.nodeType === 3) {
      out.appendChild(document.createTextNode(node.data.includes("{{") ? interp(node.data, scope) : node.data));
      return;
    }
    if (node.nodeType !== 1) return;
    const tag = node.tagName.toLowerCase();
    if (tag === "sc-if") {
      if (evalIn(node.getAttribute("value").match(ONLY)[1], scope)) for (const c of node.childNodes) renderNode(c, scope, out);
      return;
    }
    if (tag === "sc-for") {
      const list = evalIn(node.getAttribute("list").match(ONLY)[1], scope) || [];
      const as = node.getAttribute("as");
      for (const item of list) {
        const inner = Object.create(scope);
        inner[as] = item;
        for (const c of node.childNodes) renderNode(c, inner, out);
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
    for (const c of node.childNodes) renderNode(c, scope, el);
    if (selectValue !== undefined) el.value = selectValue;
    out.appendChild(el);
  }

  window.__dcRender = () => {
    const scope = logic.renderVals();
    const frag = document.createDocumentFragment();
    for (const c of template.childNodes) renderNode(c, scope, frag);
    root.replaceChildren(frag);
  };
  window.__dcRender();
  window.__dc = logic;
});
