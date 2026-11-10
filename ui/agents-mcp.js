/** External agents over MCP: the Connectors card that lists them, and the consent card a pending
 * OAuth authorization opens. The hub sends the browser to /connect-agent?request=…; this page keeps
 * the request across a sign-in and asks the signed-in account, over the hub socket, whether to let
 * the agent in. The agent never sees the account's login. */
const AGENT_SCOPES = {
  sessions: "Start coding sessions in the projects you pick, send them messages and read them",
  orchestrator: "Message the Captain of the projects you pick and read its replies",
};
const AGENT_REQUEST_KEY = "pinned-agent-request";

const agentsUi = { list: null, endpoint: null, error: "", consent: null };

try {
  if (!TAURI && location.pathname === "/connect-agent") {
    const request = new URLSearchParams(location.search).get("request");
    if (request) sessionStorage.setItem(AGENT_REQUEST_KEY, request);
    history.replaceState(null, "", "/");
  }
} catch {}

const agentRequestId = () => {
  try {
    return sessionStorage.getItem(AGENT_REQUEST_KEY);
  } catch {
    return null;
  }
};
const forgetAgentRequest = () => {
  try {
    sessionStorage.removeItem(AGENT_REQUEST_KEY);
  } catch {}
  agentsUi.consent = null;
};
const agentHost = (uri) => {
  try {
    const url = new URL(uri);
    return url.host || `${url.protocol}//`;
  } catch {
    return uri;
  }
};
const agentWhen = (secs) => (secs ? new Date(secs * 1000).toLocaleString(APP_LOCALE) : "never");

/** Every project of the account's online machines, as the consent card offers them. */
async function agentProjectChoices() {
  const listed = [];
  for (const [id, m] of machines.entries()) {
    if (!m.view.online || m.view.upstash_box) continue;
    const machine = hubIdOf(id);
    if (!machine) continue;
    try {
      const projects = await m.transport.call({ cmd: "list_projects" });
      for (const p of projects) listed.push({ machine, machineName: machineName(id), path: p.path, name: p.name });
    } catch (e) {
      console.warn("list_projects", id, e);
    }
  }
  return listed;
}

/** Opens the Connectors page on the pending request, once the hub socket is up. */
async function showAgentConsent() {
  const request = agentRequestId();
  if (!request || !hub) return;
  try {
    const view = await hub.hub({ cmd: "agent_request", request });
    agentsUi.consent = { request, view, projects: await agentProjectChoices(), error: "" };
  } catch (e) {
    agentsUi.consent = { request, view: null, projects: [], error: String(e) };
  }
  page = "integrations";
  save("settings-page", page);
  if (!settingsOpen()) await toggleSettings();
  else showPage("integrations");
}

function agentConsentHtml() {
  const c = agentsUi.consent;
  if (!c) return "";
  if (!c.view) {
    return `<div class="card agent-consent" id="agent-consent">
      <div class="card-head"><span class="logo agents">⇄</span><div><b>Connect an external agent</b></div></div>
      <p class="error">${esc(c.error)}</p>
      <div class="actions"><button class="secondary" data-action="dismiss-agent">Dismiss</button></div></div>`;
  }
  const framed = window.top !== window.self;
  const { view } = c;
  const scopes = Object.entries(AGENT_SCOPES)
    .map(([scope, text]) => `<label class="check"><input type="checkbox" name="agent-scope" value="${esc(scope)}"${view.scopes.includes(scope) ? " checked" : ""}> <b>${esc(scope)}</b> <span class="hint">${esc(text)}</span></label>`)
    .join("");
  const projects = c.projects.length
    ? c.projects
        .map((p, i) => `<label class="check"><input type="checkbox" name="agent-project" value="${i}"> <b>${esc(p.name)}</b> <span class="hint">${esc(p.machineName)} · ${esc(p.path)}</span></label>`)
        .join("")
    : `<p class="hint">No online machine lists a project. Bring the machine with the project online, then reload this page.</p>`;
  const plain = !/^https:/i.test(view.redirect_uri) && !/^http:\/\/(127\.0\.0\.1|localhost|\[::1\])[:/]/i.test(view.redirect_uri);
  return `<div class="card agent-consent" id="agent-consent">
    <div class="card-head"><span class="logo agents">⇄</span><div><b>Connect “${esc(view.client)}”?</b>
      <div class="hint">An external agent asks to work in this account through Pinned's MCP server.</div></div>
      <span class="badge">Waiting for you</span></div>
    <p>After you answer, your browser goes back to <code>${esc(view.redirect_uri)}</code>${plain ? " (an app on this device)" : ""}. Approve only if you just started connecting from that agent. The name above is the one the agent gave itself.</p>
    <h4>What it may do</h4>
    <div class="agent-checks">${scopes}</div>
    <h4>Where</h4>
    <div class="agent-checks">${projects}</div>
    <p class="hint">Its sessions run with your permission settings and ask you before risky actions, as your own do; it can't answer those prompts. It never gets your password or sign-in. Expires ${esc(agentWhen(view.expires_at))}.</p>
    ${framed ? `<p class="error">This page is inside another page, so it can't approve. Open Pinned in its own tab.</p>` : ""}
    <div class="actions"><button class="secondary" data-action="deny-agent">Deny</button><button class="primary" data-action="approve-agent"${framed ? " disabled" : ""}>Approve</button></div>
    <p class="error" id="agent-consent-error">${esc(c.error)}</p>
  </div>`;
}

function externalAgentsCardHtml() {
  const count = agentsUi.list?.length ?? 0;
  let body;
  if (!account || !hub) {
    body = `<p class="hint">Sign in to pinned to let external agents in.</p>`;
  } else {
    const endpoint = agentsUi.endpoint ?? "";
    const local = endpoint && !/^https:\/\//i.test(endpoint);
    const rows = (agentsUi.list ?? [])
      .map((a) => `<li class="agent-row">
          <div><b>${esc(a.name)}</b> <span class="hint">via ${esc(agentHost(a.redirect))}</span>
            <div class="hint">${esc(a.scopes.join(", "))} · ${esc(a.projects.map((p) => `${p.name} on ${machineName(localIdOf(p.machine))}`).join(", "))}</div>
            <div class="hint">Connected ${esc(agentWhen(a.created_at))} · last used ${esc(agentWhen(a.last_used))}</div></div>
          <button class="secondary" data-action="revoke-agent" data-agent="${esc(a.id)}">Revoke</button></li>`)
      .join("");
    body = `<ol class="steps">
        <li>In the agent's connector settings, add a custom MCP server with this URL.
          <div class="row"><input id="mcp-endpoint" readonly value="${esc(endpoint)}"><button class="secondary" data-action="copy-mcp-endpoint">Copy</button></div></li>
        <li>The agent opens Pinned in your browser. Sign in if asked, then pick what it may do and in which projects.</li>
        <li>Revoke it here at any time. Its access stops at once.</li>
      </ol>
      ${local ? `<p class="hint">This hub's address is not public HTTPS, so a hosted agent can't reach it. An agent on this network can.</p>` : ""}
      ${rows ? `<ul class="agent-list">${rows}</ul>` : ""}
      <p class="error" id="agents-error">${esc(agentsUi.error)}</p>`;
  }
  return `<div class="card" id="external-agents">
      <div class="card-head"><span class="logo agents">⇄</span><div><b>External agents (MCP)</b><div class="hint">Let an agent such as Grok start coding sessions or talk to a project's Captain.</div></div>
        <span class="badge" data-on="${count > 0}">${count ? `${count} connected` : "Not connected"}</span></div>
      ${body}
    </div>`;
}

function redrawAgentCards() {
  if (!settingsOpen() || page !== "integrations") return;
  const card = document.getElementById("external-agents");
  if (card) card.outerHTML = externalAgentsCardHtml();
  const consent = document.getElementById("agent-consent");
  if (consent) consent.outerHTML = agentConsentHtml();
}

async function loadExternalAgents() {
  if (!account || !hub) return;
  try {
    const found = await hub.hub({ cmd: "external_agents" });
    agentsUi.list = found.agents;
    agentsUi.endpoint = found.endpoint;
    agentsUi.error = "";
  } catch (e) {
    agentsUi.error = String(e);
  }
  redrawAgentCards();
}

function agentConsentError(e) {
  agentsUi.consent.error = String(e);
  const line = document.getElementById("agent-consent-error");
  if (line) line.textContent = agentsUi.consent.error;
}

Object.assign(actions, {
  async "copy-mcp-endpoint"() {
    const input = document.getElementById("mcp-endpoint");
    try {
      await navigator.clipboard.writeText(input.value);
    } catch {
      input.select();
    }
  },
  async "revoke-agent"(button) {
    button.disabled = true;
    try {
      await hub.hub({ cmd: "revoke_agent", id: button.dataset.agent });
    } catch (e) {
      agentsUi.error = String(e);
    }
    await loadExternalAgents();
  },
  async "approve-agent"(button) {
    const c = agentsUi.consent;
    if (window.top !== window.self) return agentConsentError("Open Pinned in its own tab to approve.");
    const scopes = [...settingsPage.querySelectorAll('input[name="agent-scope"]:checked')].map((i) => i.value);
    const projects = [...settingsPage.querySelectorAll('input[name="agent-project"]:checked')].map((i) => c.projects[Number(i.value)]).map(({ machine, path, name }) => ({ machine, path, name }));
    if (!scopes.length) return agentConsentError("Pick what the agent may do.");
    if (!projects.length) return agentConsentError("Pick at least one project.");
    button.disabled = true;
    try {
      const { redirect } = await hub.hub({ cmd: "approve_agent", request: c.request, scopes, projects });
      forgetAgentRequest();
      location.assign(redirect);
    } catch (e) {
      button.disabled = false;
      agentConsentError(e);
    }
  },
  async "deny-agent"() {
    try {
      const { redirect } = await hub.hub({ cmd: "deny_agent", request: agentsUi.consent.request });
      forgetAgentRequest();
      location.assign(redirect);
    } catch (e) {
      forgetAgentRequest();
      agentsUi.error = String(e);
      showPage("integrations");
    }
  },
  "dismiss-agent"() {
    forgetAgentRequest();
    showPage("integrations");
  },
});
