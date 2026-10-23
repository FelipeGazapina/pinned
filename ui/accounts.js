/** The Accounts page: the agent accounts of one machine, each signed in to its CLI in a home of its own. */
const PROVIDERS = { claude: "Claude Code", codex: "Codex" };

let accountsUi = { machine: LOCAL, removing: null, error: "" };

function showAccountsPage() {
  if (!machines.get(accountsUi.machine)?.view.online) accountsUi.machine = onlineMachines()[0];
  settingsPage.innerHTML = `<h2>Accounts</h2>
    ${machineChipsHtml(accountsUi.machine, "accounts-machine")}
    <p class="hint">Each project runs and pays for its sessions with the account you pick for it under Projects. Accounts are kept per machine.</p>
    <div id="account-list"></div>
    <form class="card add-account">
      <div class="card-head"><div><b>Add an account</b><div class="hint">Another subscription or login, kept apart from the others on ${esc(machineName(accountsUi.machine))}</div></div></div>
      <div class="row"><select name="provider">${options(Object.keys(PROVIDERS), (p) => PROVIDERS[p])}</select>
        <input name="name" placeholder="Work" autocomplete="off" spellcheck="false">
        <button class="primary">Add account</button></div>
      <p class="error" id="add-account-error"></p>
    </form>`;
  settingsPage.querySelector(".add-account").onsubmit = (e) => {
    e.preventDefault();
    addAccount(e.target);
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
}

function accountCardHtml(a) {
  const status = !a.signed_in ? "Not signed in" : a.email ? `Signed in as ${[a.email, a.org, a.plan].filter(Boolean).join(" · ")}` : "Signed in";
  const confirm =
    accountsUi.removing === a.id
      ? `<div class="confirm" data-account="${esc(a.id)}">Remove ${esc(a.name)}? Projects that run under it go back to ${esc(accountName(accountsUi.machine, DEFAULT_ACCOUNT))}. Its login stays on the machine, so adding it again finds it signed in.
           <span class="actions start"><button class="secondary danger" data-action="confirm-remove-account">Remove</button>
           <button class="secondary" data-action="cancel-remove-account">Cancel</button></span></div>`
      : "";
  return `<div class="card" data-account="${esc(a.id)}">
      <div class="card-head"><span class="logo ${esc(a.provider)}">${a.provider === "codex" ? "◎" : "✳"}</span>
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
    setAccounts(machine, [...(machines.get(machine).accounts ?? []), added]);
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
