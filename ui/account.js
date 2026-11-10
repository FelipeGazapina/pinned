const DEFAULT_HUB = "https://pinned-hub-production.up.railway.app";

function showAccount() {
  if (!account) {
    settingsPage.innerHTML = `<h2>Account</h2>
      <p class="hint">Sign in to see your other machines and their sessions here, open sessions on them, and run workflows on whichever machine is online.</p>
      <form class="card sign-in">
        <label>Hub <input name="hub" value="${esc(load("hub-url") ?? DEFAULT_HUB)}" spellcheck="false" autocomplete="off"></label>
        <label>Email <input name="email" type="email" autocomplete="username"></label>
        <label>Password <input name="password" type="password" autocomplete="current-password"></label>
        <label hidden>Invite code, to create an account <input name="invite" autocomplete="off" spellcheck="false" autocapitalize="characters"></label>
        <p class="error" id="account-error"></p>
        <button type="button" class="retry" hidden>Retry</button>
        <div class="actions"><button type="button" class="secondary" data-action="create-account">Create account</button>
          <button class="primary">Sign in</button></div>
      </form>`;
    const form = settingsPage.querySelector(".sign-in");
    form.onsubmit = (e) => {
      e.preventDefault();
      signInApp(false);
    };
    const askInvite = async () => (form.invite.parentElement.hidden = (await signupMode(form.hub.value.trim().replace(/\/+$/, ""))) !== "invite");
    form.hub.onchange = askInvite;
    askInvite();
    return;
  }
  // Redrawing would take the password being typed away.
  const typing = settingsPage.querySelector(".new-password");
  if (typing && (typing.current.value || typing.password.value)) return;
  const state = document.body.dataset.hub;
  const badge = state === "online" ? "Connected" : state === "refused" ? "Refused by the hub" : "Connecting…";
  const name = machines.get(LOCAL)?.hubView?.name;
  settingsPage.innerHTML = `<h2>Account</h2>
    <p class="hint">Your machines, their sessions and your workflows belong to this account.</p>
    <div class="card">
      <div class="card-head"><span class="logo account">@</span>
        <div><b>${esc(account.email || "…")}</b><div class="hint">${esc(account.hub)}</div></div>
        <span class="badge" data-on="${state === "online"}">${badge}</span></div>
      ${TAURI ? `<p>This ${COMPUTER} is <b>${esc(name ?? "…")}</b> in the account. It stays signed in, with its sessions open to your other devices, until you sign out.</p>` : ""}
      ${state === "refused" ? `<p class="error">The hub no longer accepts this sign-in. Sign out, then sign in again.</p>` : ""}
      <div class="actions"><button class="secondary" data-action="sign-out">Sign out</button></div>
    </div>
    ${TAURI ? "" : notificationsHtml()}
    <form class="card new-password">
      <div class="card-head"><div><b>Password</b></div></div>
      <input name="username" type="email" value="${esc(account.email)}" autocomplete="username" hidden>
      <label>Current password <input name="current" type="password" autocomplete="current-password"></label>
      <label>New password <input name="password" type="password" autocomplete="new-password" minlength="8"></label>
      <p class="error" id="password-error"></p><p class="hint" id="password-done"></p>
      <div class="actions"><button class="primary">Change password</button></div>
    </form>
    ${invitesHtml()}`;
  settingsPage.querySelector(".new-password").onsubmit = (e) => {
    e.preventDefault();
    changePassword(e.target);
  };
  if (invitesUi.of !== account) invitesUi = { of: account, signup: null, pending: [], made: null, error: "", loading: false };
  if (state === "online" && invitesUi.signup === null && !invitesUi.loading) loadInvites();
}

// Whether the hub at `hubUrl` asks a new account for an invite code; null when it can't say.
async function signupMode(hubUrl) {
  try {
    const res = await fetch(`${hubUrl}/api/signup`);
    return res.ok ? (await res.json()).signup : null;
  } catch {
    return null;
  }
}

let invitesUi = { of: null, signup: null, pending: [], made: null, error: "", loading: false };

async function loadInvites() {
  invitesUi.loading = true;
  try {
    const { signup, pending } = await hub.hub({ cmd: "invites" });
    Object.assign(invitesUi, { signup, pending });
  } catch {
    // An older hub has no invites.
    invitesUi.signup = "none";
  }
  invitesUi.loading = false;
  if (settingsOpen() && page === "account") showAccount();
}

const inviteDate = (unix) => new Date(unix * 1000).toLocaleDateString(APP_LOCALE, { month: "short", day: "numeric" });
const clockTime = (unix) => new Date(unix * 1000).toLocaleTimeString(APP_LOCALE, { hour: "2-digit", minute: "2-digit" });

function invitesHtml() {
  if (invitesUi.signup !== "invite") return "";
  const { made, pending, error } = invitesUi;
  const rows = pending
    .map(
      (invite) => `<div class="machine-row" data-invite="${esc(invite.id)}">
        <div class="who"><b>${esc(invite.hint)}-…</b><span class="hint">Created ${esc(inviteDate(invite.created_at))}</span></div>
        <button class="secondary danger" data-action="revoke-invite">Revoke</button></div>`,
    )
    .join("");
  return `<div class="card invites">
      <div class="card-head"><div><b>Invite someone</b>
        <div class="hint">A code lets one person create their own account on this hub, with their own machines and sessions. It works once and doesn't expire.</div></div></div>
      ${
        made
          ? `<p>Send this code to the person you invite. It is shown only now.</p>
             <div class="row command"><code>${esc(made.code)}</code><button class="secondary" data-action="copy-invite">Copy</button></div>`
          : ""
      }
      ${rows ? `<p class="hint">Codes not used yet</p><div class="machine-list">${rows}</div>` : `<p class="hint">No codes waiting to be used.</p>`}
      <p class="error">${esc(error)}</p>
      <div class="actions start"><button class="${made ? "secondary" : "primary"}" data-action="create-invite">Create an invite code</button></div>
    </div>`;
}

async function inviteAction(call) {
  invitesUi.error = "";
  try {
    return await hub.hub(call);
  } catch (e) {
    invitesUi.error = String(e);
  } finally {
    await loadInvites();
  }
}

Object.assign(actions, {
  async "create-invite"() {
    invitesUi.made = (await inviteAction({ cmd: "create_invite" })) ?? invitesUi.made;
    showAccount();
  },
  async "revoke-invite"(button) {
    const id = button.closest("[data-invite]").dataset.invite;
    if (invitesUi.made?.id === id) invitesUi.made = null;
    await inviteAction({ cmd: "revoke_invite", id });
  },
  async "copy-invite"(button) {
    try {
      await navigator.clipboard.writeText(invitesUi.made.code);
      button.textContent = "Copied";
    } catch {
      getSelection().selectAllChildren(button.previousElementSibling);
    }
  },
});

const NOTIFICATIONS = {
  default: ["Off", "Hear when a session finishes or needs you while pinned is in the background or on another tab."],
  granted: ["Enabled", "A session that finishes or needs you while pinned is in the background or on another tab shows a notification."],
  denied: ["Blocked", "This browser blocks notifications from pinned. Allow them in the site's settings, then reload."],
  unsupported: ["Not available", "This browser can't show notifications. On an iPhone, add pinned to the Home Screen first."],
};

// The browser remembers the answer, so the card reads it from there each time.
function notificationsHtml() {
  const permission = window.Notification && navigator.serviceWorker ? Notification.permission : "unsupported";
  const [badge, hint] = NOTIFICATIONS[permission];
  return `<div class="card">
      <div class="card-head"><div><b>Notifications on this device</b><div class="hint">${hint}</div></div>
        <span class="badge" data-on="${permission === "granted"}">${badge}</span></div>
      ${permission === "default" ? `<div class="actions"><button class="primary" data-action="enable-notifications">Enable</button></div>` : ""}
    </div>`;
}

actions["enable-notifications"] = async () => {
  await Notification.requestPermission();
  showAccount();
};

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

/** Words a failed sign-in: plainly when the hub can't be reached, with a Retry that sends it again. */
function showSignInError(error, retry, e, hub, again) {
  error.textContent = signInProblem(e, hub);
  retry.hidden = !hubUnreachable(e);
  retry.onclick = again;
}

async function signInApp(create) {
  const form = settingsPage.querySelector(".sign-in");
  const error = document.getElementById("account-error");
  const retry = form.querySelector(".retry");
  const hubUrl = form.hub.value.trim().replace(/\/+$/, "");
  const email = form.email.value.trim();
  const password = form.password.value;
  if (!hubUrl || !email || !password) return (error.textContent = "Fill in the hub, your email and a password.");
  error.textContent = "";
  retry.hidden = true;
  for (const b of form.querySelectorAll("button")) b.disabled = true;
  try {
    const signedIn = await invoke("sign_in", { hub: hubUrl, email, password, create, invite: form.invite.value.trim() || null });
    save("hub-url", hubUrl);
    connectHub(signedIn);
    showAccount();
  } catch (e) {
    showSignInError(error, retry, e, hubUrl, () => signInApp(create));
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
  return `seen ${new Date(unix * 1000).toLocaleDateString(APP_LOCALE)}`;
}

function showMachinesAndRecheckSync() {
  showMachinesPage();
  for (const [id, m] of machines) if (id === LOCAL ? m.hubView : m.view.online) refreshSyncStopped(id);
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
    <div class="actions start"><button class="secondary" data-action="update-all">Update every machine</button></div>
    <div class="machine-list">${rows}</div>
    <p class="error">${esc(machinesUi.error)}</p>
    <div class="card">
      <div class="card-head"><span class="logo server">▣</span>
        <div><b>Add a server</b><div class="hint">A Linux box or another Mac that runs sessions without the app</div></div></div>
      ${
        pair
          ? `<p>Run this on the server. The code works once, until ${esc(clockTime(pair.expires_at))}.</p>
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
  const where = [view.os, local && (TAURI ? `this ${COMPUTER}` : null), !view.online && lastSeen(view.last_seen)].filter(Boolean).join(" · ");
  const status = rowUpdate(view);
  const updateId = local ? account?.machine_id : view.id;
  const name =
    machinesUi.renaming === id
      ? `<input class="rename" value="${esc(view.name)}" spellcheck="false" autocomplete="off">`
      : view.upstash_box
        ? `<span><b>${esc(view.name)}</b><span class="badge box" title="Upstash Box ${esc(view.upstash_box)}">Box</span></span>`
        : `<b>${esc(view.name)}</b>`;
  const leaves = view.upstash_box ? "Its Upstash Box is deleted, and the session in it with it." : "It leaves the account and its sessions stop showing on your devices.";
  const confirm =
    machinesUi.removing === id
      ? `<div class="confirm" data-machine="${esc(id)}">Remove ${esc(view.name)}? ${leaves}
           <span class="actions start"><button class="secondary danger" data-action="confirm-remove">Remove</button>
           <button class="secondary" data-action="cancel-remove">Cancel</button></span></div>`
      : "";
  const stopped = machines.get(id)?.syncStopped;
  const sync = stopped
    ? `<div class="sync-stopped" data-machine="${esc(id)}"><b>Sync stopped.</b> ${esc(view.name)} can't read its record of the last sync, so it no longer syncs projects and plugins.
         <span class="hint">${esc(stopped)}</span>
         Start over to sync again, as a new machine does. A file changed on both sides keeps the newer copy, and a file deleted elsewhere since the last sync can come back.
         <span class="actions start"><button class="secondary" data-action="reset-sync">Start sync over</button></span></div>`
    : "";
  return `<div class="machine-row" data-machine="${esc(id)}">
      <span class="dot" data-online="${view.online}"></span>
      <div class="who">${name}<span class="hint">${esc(where)}</span>${status ? `<span class="hint">${esc(status)}</span>` : ""}</div>
      ${updateId && machinesUi.renaming !== id ? `<button class="secondary" data-action="update-machine">Update</button>` : ""}
      ${machinesUi.renaming === id ? "" : `<button class="secondary" data-action="rename-machine">Rename</button>`}
      ${local ? "" : `<button class="secondary danger" data-action="remove-machine">Remove</button>`}
    </div>${sync}${confirm}`;
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
  "update-all": () => {
    updateEveryMachine().catch((e) => {
      machinesUi.error = String(e);
      showMachinesPage();
    });
  },
  async "update-machine"(button) {
    const row = rowMachine(button);
    const id = row === LOCAL ? account?.machine_id : row;
    if (!id) return;
    machinesUi.error = "";
    try {
      await hub.call(id, { cmd: "update_self" });
    } catch (e) {
      machinesUi.error = String(e);
    }
    showMachinesPage();
  },
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
  async "reset-sync"(button) {
    const id = rowMachine(button);
    machinesUi.error = "";
    await callOn(id, { cmd: "reset_sync" }).catch((e) => (machinesUi.error = String(e)));
    await refreshSyncStopped(id);
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
  loginForm.querySelector(".retry").hidden = true;
  loginForm.email.focus();
  signupMode("").then((mode) => (loginForm.invite.hidden = mode !== "invite"));
}

async function loginWeb(create) {
  const error = loginForm.querySelector(".error");
  const email = loginForm.email.value.trim();
  const password = loginForm.password.value;
  const invite = create ? loginForm.invite.value.trim() : "";
  if (!email || !password) return (error.textContent = "Fill in your email and password.");
  error.textContent = "";
  loginForm.querySelector(".retry").hidden = true;
  try {
    const res = await fetch(`/api/${create ? "signup" : "login"}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(invite ? { email, password, invite } : { email, password }),
    });
    const text = await res.text();
    let body = {};
    try {
      body = JSON.parse(text);
    } catch {}
    if (!res.ok) throw body.error ?? (text || res.statusText);
    save("hub-token", body.token);
    loginForm.password.value = loginForm.invite.value = "";
    delete document.body.dataset.page;
    connectHub({ hub: location.origin, user_token: body.token, machine_id: null, email: body.account?.email ?? email });
  } catch (e) {
    showSignInError(error, loginForm.querySelector(".retry"), e, location.origin, () => loginWeb(create));
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
