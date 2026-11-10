/** The Accounts page: the agent accounts of one machine, each signed in to its CLI in a home of its own. */
const PROVIDERS = { claude: "Claude Code", codex: "Codex", cursor: "Cursor", "opencode-go": "OpenCode Go" };

let accountsUi = { machine: LOCAL, removing: null, replacing: null, error: "" };

function showAccountsPage() {
  if (!machines.get(accountsUi.machine)?.view.online) accountsUi.machine = onlineMachines()[0];
  const asked = agentAsking("add_account")?.args;
  const choice = !asked ? "claude" : asked.auth === "opencode-go" ? "opencode-go" : asked.provider;
  settingsPage.innerHTML = `<h2>Accounts</h2>
    ${machineChipsHtml(accountsUi.machine, "accounts-machine")}
    <p class="hint">Each project runs and pays for its sessions with the account you pick for it under Projects. Accounts are kept per machine.</p>
    <div id="account-list"></div>
    <form class="card add-account">
      <div class="card-head"><div><b>Add an account</b><div class="hint">Another subscription or login, kept apart from the others on ${esc(machineName(accountsUi.machine))}</div></div></div>
      ${agentAskingHtml("add_account")}
      <div class="row"><select name="provider"${asked ? " disabled" : ""}>${options(Object.keys(PROVIDERS), (p) => PROVIDERS[p], choice)}</select>
        <input name="name" placeholder="Work" autocomplete="off" spellcheck="false"${asked ? ` value="${esc(asked.name)}" readonly` : ""}>
        <input name="key" type="password" placeholder="OpenCode Go API key (oc_sk_...)" autocomplete="off" spellcheck="false" hidden>
        <button class="primary">Add account</button></div>
      <p class="error" id="add-account-error"></p>
    </form>`;
  const form = settingsPage.querySelector(".add-account");
  const toggleKey = () => {
    const go = form.provider.value === "opencode-go";
    form.key.hidden = !go;
    form.key.required = go;
    if (go) form.key.focus();
  };
  form.provider.onchange = toggleKey;
  if (asked) toggleKey();
  form.onsubmit = (e) => {
    e.preventDefault();
    addAccount(form);
  };
  loadAccountList();
}

async function loadAccountList() {
  const { machine } = accountsUi;
  const list = document.getElementById("account-list");
  if (!machine) return (list.innerHTML = `<p class="empty">No machine is online.</p>`);
  list.innerHTML = `<p class="hint">Asking each account's CLI who is signed in…</p>`;
  try {
    await accountsOf(machine, true);
  } catch (e) {
    accountsUi.error = String(e);
  }
  if (page === "accounts" && accountsUi.machine === machine) drawAccountList();
}

function drawAccountList() {
  const list = document.getElementById("account-list");
  if (!list) return;
  const accounts = machines.get(accountsUi.machine)?.accounts ?? [];
  list.innerHTML = `${accounts.map(accountCardHtml).join("")}<p class="error">${esc(accountsUi.error)}</p>`;
  const input = list.querySelector(`[data-account="${accountsUi.replacing}"] [name=key]`);
  input?.focus();
}

/** What stopped a limited account, and when it lifts when the provider said. */
function limitText(a) {
  const when = a.limit_resume_at ? new Date(a.limit_resume_at * 1000).toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }) : "";
  return `Limited: ${a.limit_reason || "a usage limit stopped this account"}${when ? `. Resumes ${when}.` : ". pinned tries it again soon."} Delegated tasks go to the next model on the ladder.`;
}

function accountCardHtml(a) {
  const go = a.auth === "opencode-go";
  const status = !a.signed_in ? "Not signed in" : a.email ? `Signed in as ${[a.email, a.org, a.plan].filter(Boolean).join(" · ")}` : "Signed in";
  const kept = go ? "Its API key is deleted from the machine." : "Its login stays on the machine, so adding it again finds it signed in.";
  const confirm =
    accountsUi.removing === a.id
      ? `<div class="confirm" data-account="${esc(a.id)}">Remove ${esc(a.name)}? Projects that run under it go back to ${esc(accountName(accountsUi.machine, DEFAULT_ACCOUNT))}. ${kept}
           <span class="actions start"><button class="secondary danger" data-action="confirm-remove-account">Remove</button>
           <button class="secondary" data-action="cancel-remove-account">Cancel</button></span></div>`
      : "";
  const replace =
    go && accountsUi.replacing === a.id
      ? `<div class="replace-key">${agentAsking("set_account_key")?.args.account === a.id ? agentAskingHtml("set_account_key") : ""}<input name="key" type="password" placeholder="oc_sk_..." autocomplete="off" spellcheck="false">
           <button class="primary" data-action="save-account-key">Save key</button>
           <button class="secondary" data-action="cancel-account-key">Cancel</button>
           <p class="error" data-key-error></p></div>`
      : "";
  const action = go
    ? `<button class="secondary" data-action="replace-account-key">Replace key</button>`
    : `<button class="${a.signed_in ? "secondary" : "primary"}" data-action="sign-in-account">${a.signed_in ? "Sign in again" : "Sign in"}</button>`;
  const limit = a.limited ? `<p class="limit-note" data-limit>${esc(limitText(a))}</p>` : "";
  return `<div class="card" data-account="${esc(a.id)}">
      <div class="card-head"><span class="logo ${go ? "opencode" : esc(a.provider)}">${go ? icon("opencode") : a.provider === "codex" ? "◎" : a.provider === "cursor" ? "▸" : "✳"}</span>
        <div><b>${esc(a.name)}</b><div class="hint">${esc(a.provider_name)}${a.builtin ? " · built in" : ""}</div></div>${a.limited ? `<span class="pill limited">Limited</span>` : ""}</div>
      <p>${esc(status)}</p>
      ${limit}
      <div class="actions start">${action}
        ${a.builtin ? "" : `<button class="secondary danger" data-action="remove-account">Remove</button>`}</div>
      ${replace}
    </div>${confirm}`;
}

async function addAccount(form) {
  const error = document.getElementById("add-account-error");
  const choice = form.provider.value;
  const go = choice === "opencode-go";
  const name = form.name.value.trim();
  const key = form.key.value.trim();
  if (!name) return (error.textContent = "Name the account.");
  if (go && !key) return (error.textContent = "Paste the OpenCode Go API key.");
  error.textContent = "";
  form.querySelector("button").disabled = true;
  const { machine } = accountsUi;
  try {
    const call = { cmd: "add_account", provider: go ? "claude" : choice, name, auth: go ? "opencode-go" : undefined, key: go ? key : undefined };
    const added = await (agentApproval(call) ?? callOn(machine, call));
    setAccounts(machine, [...(machines.get(machine).accounts ?? []).filter((a) => a.id !== added.id), added]);
    form.name.value = "";
    form.key.value = "";
    form.provider.value = "claude";
    form.key.hidden = true;
    form.key.required = false;
    drawAccountList();
  } catch (e) {
    error.textContent = String(e);
  }
  form.querySelector("button").disabled = false;
}

const cardAccount = (button) => button.closest("[data-account]").dataset.account;

Object.assign(actions, {
  "accounts-machine"(button) {
    accountsUi = { machine: button.dataset.machine, removing: null, replacing: null, error: "" };
    showAccountsPage();
  },
  async "sign-in-account"(button) {
    button.disabled = true;
    accountsUi.error = "";
    try {
      await signIn(accountsUi.machine, cardAccount(button));
    } catch (e) {
      accountsUi.error = String(e);
      drawAccountList();
    }
  },
  "replace-account-key"(button) {
    accountsUi = { ...accountsUi, replacing: cardAccount(button), removing: null, error: "" };
    drawAccountList();
  },
  "cancel-account-key"() {
    if (agentAsking("set_account_key")) forgetAgentApproval();
    accountsUi = { ...accountsUi, replacing: null };
    drawAccountList();
  },
  async "save-account-key"(button) {
    const card = button.closest("[data-account]");
    const account = card.dataset.account;
    const input = card.querySelector("[name=key]");
    const error = card.querySelector("[data-key-error]");
    const key = input.value.trim();
    if (!key) return (error.textContent = "Paste the OpenCode Go API key.");
    button.disabled = true;
    error.textContent = "";
    try {
      const call = { cmd: "set_account_key", account, key };
      const saved = await (agentApproval(call) ?? callOn(accountsUi.machine, call));
      setAccounts(accountsUi.machine, (machines.get(accountsUi.machine).accounts ?? []).map((a) => (a.id === account ? saved : a)));
      accountsUi.replacing = null;
      drawAccountList();
    } catch (e) {
      error.textContent = String(e);
      button.disabled = false;
    }
  },
  "remove-account"(button) {
    accountsUi = { ...accountsUi, removing: cardAccount(button), replacing: null };
    drawAccountList();
  },
  "cancel-remove-account"() {
    accountsUi = { ...accountsUi, removing: null };
    drawAccountList();
  },
  async "confirm-remove-account"() {
    const { machine, removing } = accountsUi;
    accountsUi = { ...accountsUi, removing: null, error: "" };
    try {
      await callOn(machine, { cmd: "remove_account", account: removing });
      setAccounts(machine, machines.get(machine).accounts.filter((a) => a.id !== removing));
    } catch (e) {
      accountsUi.error = String(e);
    }
    drawAccountList();
  },
});
settingsSection({ page: "accounts", label: "Accounts", icon: "users", show: () => showAccountsPage(), search: [{ title: "Accounts", terms: ["claude code codex cursor opencode go api key sign in login add remove agent account home"] }] });
