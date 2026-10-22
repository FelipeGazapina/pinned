const DEFAULT_HUB = "https://pinned.187-127-36-149.sslip.io";

function showAccount() {
  if (!account) {
    settingsPage.innerHTML = `<h2>Account</h2>
      <p class="hint">Sign in to see your other machines and their sessions here, open sessions on them, and run connectors on whichever machine is online.</p>
      <form class="card sign-in">
        <label>Hub <input name="hub" value="${esc(load("hub-url") ?? DEFAULT_HUB)}" spellcheck="false" autocomplete="off"></label>
        <label>Email <input name="email" type="email" autocomplete="username"></label>
        <label>Password <input name="password" type="password" autocomplete="current-password"></label>
        <p class="error" id="account-error"></p>
        <div class="actions"><button type="button" class="secondary" data-action="create-account">Create account</button>
          <button class="primary">Sign in</button></div>
      </form>`;
    settingsPage.querySelector(".sign-in").onsubmit = (e) => {
      e.preventDefault();
      signInApp(false);
    };
    return;
  }
  // Redrawing would take the password being typed away.
  const typing = settingsPage.querySelector(".new-password");
  if (typing && (typing.current.value || typing.password.value)) return;
  const state = document.body.dataset.hub;
  const badge = state === "online" ? "Connected" : state === "refused" ? "Refused by the hub" : "Connecting…";
  const name = machines.get(LOCAL)?.hubView?.name;
  settingsPage.innerHTML = `<h2>Account</h2>
    <p class="hint">Your machines, their sessions and your connectors belong to this account.</p>
    <div class="card">
      <div class="card-head"><span class="logo account">@</span>
        <div><b>${esc(account.email || "…")}</b><div class="hint">${esc(account.hub)}</div></div>
        <span class="badge" data-on="${state === "online"}">${badge}</span></div>
      ${TAURI ? `<p>This Mac is <b>${esc(name ?? "…")}</b> in the account. It stays signed in, with its sessions open to your other devices, until you sign out.</p>` : ""}
      ${state === "refused" ? `<p class="error">The hub no longer accepts this sign-in. Sign out, then sign in again.</p>` : ""}
      <div class="actions"><button class="secondary" data-action="sign-out">Sign out</button></div>
    </div>
    <form class="card new-password">
      <div class="card-head"><div><b>Password</b></div></div>
      <input name="username" type="email" value="${esc(account.email)}" autocomplete="username" hidden>
      <label>Current password <input name="current" type="password" autocomplete="current-password"></label>
      <label>New password <input name="password" type="password" autocomplete="new-password" minlength="8"></label>
      <p class="error" id="password-error"></p><p class="hint" id="password-done"></p>
      <div class="actions"><button class="primary">Change password</button></div>
    </form>`;
  settingsPage.querySelector(".new-password").onsubmit = (e) => {
    e.preventDefault();
    changePassword(e.target);
  };
}

async function changePassword(form) {
  const error = document.getElementById("password-error");
  const done = document.getElementById("password-done");
  const current = form.current.value;
  const password = form.password.value;
  error.textContent = done.textContent = "";
  if (!current || !password) return (error.textContent = "Fill in your current and new password.");
  form.querySelector("button").disabled = true;
  try {
    if (TAURI) await invoke("change_password", { current, password });
    else {
      const res = await fetch("/api/password", {
        method: "POST",
        headers: { "content-type": "application/json", authorization: `Bearer ${account.user_token}` },
        body: JSON.stringify({ current, password }),
      });
      if (!res.ok) throw (await res.json().catch(() => ({}))).error ?? res.statusText;
    }
    form.reset();
    done.textContent = "Password changed. Sign in with the new one from now on.";
  } catch (e) {
    error.textContent = String(e);
  }
  form.querySelector("button").disabled = false;
}

async function signInApp(create) {
  const form = settingsPage.querySelector(".sign-in");
  const error = document.getElementById("account-error");
  const hubUrl = form.hub.value.trim().replace(/\/+$/, "");
  const email = form.email.value.trim();
  const password = form.password.value;
  if (!hubUrl || !email || !password) return (error.textContent = "Fill in the hub, your email and a password.");
  error.textContent = "";
  for (const b of form.querySelectorAll("button")) b.disabled = true;
  try {
    const signedIn = await invoke("sign_in", { hub: hubUrl, email, password, create });
    save("hub-url", hubUrl);
    connectHub(signedIn);
    showAccount();
  } catch (e) {
    error.textContent = String(e);
    for (const b of form.querySelectorAll("button")) b.disabled = false;
  }
}

actions["create-account"] = () => signInApp(true);
actions["sign-out"] = async () => {
  if (!TAURI) return signOutWeb();
  // A later sign-in registers this Mac anew, so the old entry would linger offline in the account.
  await hub?.hub({ cmd: "remove_machine", id: account.machine_id }).catch(console.warn);
  await invoke("sign_out");
  disconnectHub();
  settings = await invoke("get_settings");
  showAccount();
};

let machinesUi = { renaming: null, removing: null, pair: null, error: "" };

function lastSeen(unix) {
  if (!unix) return "never seen";
  const minutes = Math.round((Date.now() / 1000 - unix) / 60);
  if (minutes < 1) return "seen just now";
  if (minutes < 60) return `seen ${minutes} min ago`;
  if (minutes < 24 * 60) return `seen ${Math.round(minutes / 60)} h ago`;
  return `seen ${new Date(unix * 1000).toLocaleDateString()}`;
}

function showMachinesPage() {
  if (!account) {
    settingsPage.innerHTML = `<h2>Machines</h2>
      <p class="hint">Sign in under Account to see every machine of your account here.</p>
      <div class="actions start"><button class="primary" data-action="go-account">Go to Account</button></div>`;
    return;
  }
  const rows = [...machines.entries()].map(([id, m]) => machineEntryHtml(id, id === LOCAL ? m.hubView : m.view)).join("");
  const pair = machinesUi.pair;
  settingsPage.innerHTML = `<h2>Machines</h2>
    <p class="hint">Every machine signed into ${esc(account.email || "this account")}. Sessions on the ones online show up as tabs here.</p>
    <div class="machine-list">${rows}</div>
    <p class="error">${esc(machinesUi.error)}</p>
    <div class="card">
      <div class="card-head"><span class="logo server">▣</span>
        <div><b>Add a server</b><div class="hint">A Linux box or another Mac that runs sessions without the app</div></div></div>
      ${
        pair
          ? `<p>Run this on the server. The code works once, until ${esc(new Date(pair.expires_at * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }))}.</p>
             <div class="row command"><code>${esc(pair.command)}</code><button class="secondary" data-action="copy-pair">Copy</button></div>`
          : `<div class="actions start"><button class="primary" data-action="pair">Get a pairing command</button></div>`
      }
    </div>`;
  const field = settingsPage.querySelector(".rename");
  field?.focus();
  field?.select();
}

function machineEntryHtml(id, view) {
  if (!view) return "";
  const local = id === LOCAL;
  const where = [view.os, local && (TAURI ? "this Mac" : null), !view.online && lastSeen(view.last_seen)].filter(Boolean).join(" · ");
  const name =
    machinesUi.renaming === id
      ? `<input class="rename" value="${esc(view.name)}" spellcheck="false" autocomplete="off">`
      : `<b>${esc(view.name)}</b>`;
  const confirm =
    machinesUi.removing === id
      ? `<div class="confirm" data-machine="${esc(id)}">Remove ${esc(view.name)}? It leaves the account and its sessions stop showing on your devices.
           <span class="actions start"><button class="secondary danger" data-action="confirm-remove">Remove</button>
           <button class="secondary" data-action="cancel-remove">Cancel</button></span></div>`
      : "";
  return `<div class="machine-row" data-machine="${esc(id)}">
      <span class="dot" data-online="${view.online}"></span>
      <div class="who">${name}<span class="hint">${esc(where)}</span></div>
      ${machinesUi.renaming === id ? "" : `<button class="secondary" data-action="rename-machine">Rename</button>`}
      ${local ? "" : `<button class="secondary danger" data-action="remove-machine">Remove</button>`}
    </div>${confirm}`;
}

const rowMachine = (button) => button.closest("[data-machine]").dataset.machine;

async function hubAction(call) {
  machinesUi.error = "";
  try {
    return await hub.hub(call);
  } catch (e) {
    machinesUi.error = String(e);
  } finally {
    showMachinesPage();
  }
}

async function renameMachine(field) {
  const id = field.closest(".machine-row").dataset.machine;
  const name = field.value.trim();
  machinesUi.renaming = null;
  const m = machines.get(id);
  const current = id === LOCAL ? m.hubView : m.view;
  if (!name || name === current.name) return showMachinesPage();
  current.name = name;
  render();
  await hubAction({ cmd: "rename_machine", id: hubIdOf(id), name });
}

Object.assign(actions, {
  "go-account": () => showPage("account"),
  "rename-machine"(button) {
    machinesUi.renaming = rowMachine(button);
    showMachinesPage();
  },
  "remove-machine"(button) {
    machinesUi.removing = rowMachine(button);
    showMachinesPage();
  },
  "cancel-remove"() {
    machinesUi.removing = null;
    showMachinesPage();
  },
  async "confirm-remove"() {
    const id = machinesUi.removing;
    machinesUi.removing = null;
    await hubAction({ cmd: "remove_machine", id });
  },
  async pair() {
    machinesUi.pair = (await hubAction({ cmd: "pair_code" })) ?? null;
    showMachinesPage();
  },
  async "copy-pair"(button) {
    try {
      await navigator.clipboard.writeText(machinesUi.pair.command);
      button.textContent = "Copied";
    } catch {
      getSelection().selectAllChildren(button.previousElementSibling);
    }
  },
});

settingsPage.addEventListener("keydown", (e) => {
  if (!e.target.classList.contains("rename")) return;
  if (e.key === "Enter") renameMachine(e.target);
  if (e.key === "Escape") {
    machinesUi.renaming = null;
    showMachinesPage();
  }
});
settingsPage.addEventListener("focusout", (e) => {
  if (e.target.classList.contains("rename") && machinesUi.renaming) renameMachine(e.target);
});

const loginForm = document.getElementById("login");

function showLogin(reason = "") {
  document.body.dataset.page = "login";
  loginForm.querySelector(".error").textContent = reason;
  loginForm.email.focus();
}

async function loginWeb(create) {
  const error = loginForm.querySelector(".error");
  const email = loginForm.email.value.trim();
  const password = loginForm.password.value;
  if (!email || !password) return (error.textContent = "Fill in your email and password.");
  error.textContent = "";
  try {
    const res = await fetch(`/api/${create ? "signup" : "login"}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ email, password }),
    });
    const text = await res.text();
    let body = {};
    try {
      body = JSON.parse(text);
    } catch {}
    if (!res.ok) throw body.error ?? (text || res.statusText);
    save("hub-token", body.token);
    loginForm.password.value = "";
    delete document.body.dataset.page;
    connectHub({ hub: location.origin, user_token: body.token, machine_id: null, email: body.account?.email ?? email });
  } catch (e) {
    error.textContent = String(e);
  }
}

loginForm.onsubmit = (e) => {
  e.preventDefault();
  loginWeb(false);
};
loginForm.querySelector("[name=signup]").onclick = () => loginWeb(true);

async function signOutWeb(reason) {
  const token = account?.user_token;
  disconnectHub();
  save("hub-token", null);
  if (token) fetch("/api/logout", { method: "POST", headers: { authorization: `Bearer ${token}` } }).catch(() => {});
  showLogin(reason);
}

function bootWeb() {
  const token = load("hub-token");
  if (!token) return showLogin();
  connectHub({ hub: location.origin, user_token: token, machine_id: null, email: "" });
}

if (TAURI) bootApp();
else bootWeb();
