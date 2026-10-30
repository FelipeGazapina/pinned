/** The Accounts page: the agent accounts of one machine, each signed in to its CLI in a home of its own. */
const PROVIDERS = { claude: "Claude Code", codex: "Codex", cursor: "Cursor" };

let accountsUi = { machine: LOCAL, removing: null, error: "" };

function showAccountsPage() {
  if (!machines.get(accountsUi.machine)?.view.online) accountsUi.machine = onlineMachines()[0];
  settingsPage.innerHTML = `<h2>Accounts</h2>
    ${machineChipsHtml(accountsUi.machine, "accounts-machine")}
    <p class="hint">Each project runs and pays for its sessions with the account you pick for it under Projects. A long-lived credential saved on this Mac is used on your other machines. Browser sign-in and a Codex auth file stay on the machine where you signed in. A machine without that CLI still cannot run it.</p>
    <div id="account-list"></div>
    <form class="card add-account">
      <div class="card-head"><div><b>Add an account</b><div class="hint">Another subscription or login, kept apart from the others on ${esc(machineName(accountsUi.machine))}</div></div></div>
      <div class="row"><select name="provider">${options(Object.keys(PROVIDERS), (p) => PROVIDERS[p])}</select>
        <input name="name" placeholder="Work" autocomplete="off" spellcheck="false">
        <button class="primary">Add account</button></div>
      <p class="error" id="add-account-error"></p>
    </form>
    ${accountsUi.machine === LOCAL ? `<form class="card share-login">
      <div class="card-head"><div><b>Use a login on every machine</b><div class="hint">Paste a Claude <code>setup-token</code>, a Cursor API key, or a Codex API key. It is saved on this Mac, then sent only to your other machines.</div></div></div>
      <div class="row"><select id="share-login-account" name="account"></select>
        <input name="token" type="password" placeholder="setup-token or API key" autocomplete="off" spellcheck="false">
        <button class="primary">Save login</button></div>
      <p class="error" id="share-login-error"></p>
    </form>` : ""}`;
  settingsPage.querySelector(".add-account").onsubmit = (e) => {
    e.preventDefault();
    addAccount(e.target);
  };
  const share = settingsPage.querySelector(".share-login");
  if (share) share.onsubmit = (e) => {
    e.preventDefault();
    saveSharedLogin(e.target);
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

function fillShareAccounts() {
  const select = document.getElementById("share-login-account");
  if (!select) return;
  const current = select.value;
  const accounts = machines.get(LOCAL)?.accounts ?? [];
  select.innerHTML = accounts.map((account) => `<option value="${esc(account.id)}">${esc(account.name)} · ${esc(PROVIDERS[account.provider] ?? account.provider)}</option>`).join("");
  if (accounts.some((account) => account.id === current)) select.value = current;
}

async function saveSharedLogin(form) {
  const error = document.getElementById("share-login-error");
  const token = form.token.value.trim();
  if (!token) return (error.textContent = "Paste a setup-token or an API key.");
  error.textContent = "";
  form.querySelector("button").disabled = true;
  try {
    const saved = await callOn(LOCAL, { cmd: "share_login", account: form.account.value, token });
    form.token.value = "";
    setAccounts(LOCAL, [...(machines.get(LOCAL).accounts ?? []).filter((account) => account.id !== saved.id), saved]);
    drawAccountList();
  } catch (e) {
    error.textContent = String(e);
  }
  form.querySelector("button").disabled = false;
}

function drawAccountList() {
  const list = document.getElementById("account-list");
  if (!list) return;
  const accounts = machines.get(accountsUi.machine)?.accounts ?? [];
  list.innerHTML = `${accounts.map(accountCardHtml).join("")}<p class="error">${esc(accountsUi.error)}</p>`;
  fillShareAccounts();
}

function accountCardHtml(a) {
  const status = !a.signed_in ? "Not signed in" : a.email ? `Signed in as ${[a.email, a.org, a.plan].filter(Boolean).join(" · ")}` : "Signed in";
  const confirm =
    accountsUi.removing === a.id
      ? `<div class="confirm" data-account="${esc(a.id)}">Remove ${esc(a.name)}? Projects that run under it go back to ${esc(accountName(accountsUi.machine, DEFAULT_ACCOUNT))}. Its folder stays on this machine. A login shared from it is forgotten on your other machines.
           <span class="actions start"><button class="secondary danger" data-action="confirm-remove-account">Remove</button>
           <button class="secondary" data-action="cancel-remove-account">Cancel</button></span></div>`
      : "";
  return `<div class="card" data-account="${esc(a.id)}">
      <div class="card-head"><span class="logo ${esc(a.provider)}">${a.provider === "codex" ? "◎" : a.provider === "cursor" ? "▸" : "✳"}</span>
        <div><b>${esc(a.name)}</b><div class="hint">${esc(a.provider_name)}${a.builtin ? " · built in" : ""}</div></div></div>
      <p>${esc(status)}</p>
      <div class="actions start"><button class="${a.signed_in ? "secondary" : "primary"}" data-action="sign-in-account">${a.signed_in ? "Sign in again" : "Sign in"}</button>
        ${a.builtin ? "" : `<button class="secondary danger" data-action="remove-account">Remove</button>`}</div>
    </div>${confirm}`;
}

async function addAccount(form) {
  const error = document.getElementById("add-account-error");
  const name = form.name.value.trim();
  if (!name) return (error.textContent = "Name the account.");
  error.textContent = "";
  form.querySelector("button").disabled = true;
  const { machine } = accountsUi;
  try {
    const added = await callOn(machine, { cmd: "add_account", provider: form.provider.value, name });
    setAccounts(machine, [...(machines.get(machine).accounts ?? []).filter((a) => a.id !== added.id), added]);
    form.name.value = "";
    drawAccountList();
  } catch (e) {
    error.textContent = String(e);
  }
  form.querySelector("button").disabled = false;
}

const cardAccount = (button) => button.closest("[data-account]").dataset.account;

Object.assign(actions, {
  "accounts-machine"(button) {
    accountsUi = { machine: button.dataset.machine, removing: null, error: "" };
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
  "remove-account"(button) {
    accountsUi.removing = cardAccount(button);
    drawAccountList();
  },
  "cancel-remove-account"() {
    accountsUi.removing = null;
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
settingsSection({ page: "accounts", label: "Accounts", icon: "users", show: () => showAccountsPage(), search: [{ title: "Accounts", terms: ["claude code codex cursor sign in login add remove agent account home setup-token share"] }] });
