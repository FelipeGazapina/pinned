//! The global orchestrator settings page's last save. The page writes each online machine itself,
//! then records the save here; the hub keeps it with the machines that did not take it, and sends
//! it to each of them when it connects, keeping that machine's own `host`.

use crate::hub::{Account, Hub};
use pinned_core::global_orchestrator::{AccountSave, Settings};
use pinned_core::wire::{Call, MachineId, MachineView};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// How long a machine has to answer each call of a resend.
const WAIT: Duration = Duration::from_secs(30);
/// The most saves one delivery sends, when newer ones land while it sends.
const SENDS: usize = 3;

/// Keeps `settings`, cleaned and with `host` false, as the account's last save, pending on every
/// machine of the account but a box and those in `took`, and sends it to each pending machine that
/// is online now, as one that connected while the page wrote does not wait for its next connect.
pub async fn record(hub: &Arc<Hub>, account: &Arc<Account>, settings: Settings, took: Vec<MachineId>) -> Result<AccountSave, String> {
    let settings = Settings { host: false, ..settings.cleaned() };
    let pending = pending(&hub.db.machines(account.id).await?, &took);
    let id = Uuid::new_v4();
    hub.db.save_global_settings(account.id, id, &settings, &pending).await?;
    eprintln!("pinned-server: account {} recorded global orchestrator save {id}, pending on {} machines", account.id, pending.len());
    let pending: Vec<MachineId> = pending.iter().map(Uuid::to_string).collect();
    for machine in pending.iter().filter(|machine| account.is_online(machine)) {
        tokio::spawn(deliver(hub.clone(), account.clone(), machine.clone()));
    }
    Ok(AccountSave { id: id.to_string(), settings, pending })
}

/// The machines a save is pending on: every machine of the account that is no box, but those that took it.
pub fn pending(machines: &[MachineView], took: &[MachineId]) -> Vec<Uuid> {
    machines.iter().filter(|m| m.upstash_box.is_none() && !took.contains(&m.id)).filter_map(|m| Uuid::parse_str(&m.id).ok()).collect()
}

/// What a resend writes: the save's shared fields, with the `host` the machine has now.
pub fn resend(saved: &Settings, current: &Settings) -> Settings {
    Settings { host: current.host, ..saved.clone() }
}

/// Sends the account's last save to `machine`, which just connected, when it is pending there. It
/// reads the machine's settings, writes the saved shared fields with the machine's own `host`, and
/// only then takes the machine off `pending`. A machine that fails, does not answer or is too old
/// stays pending for its next connect. Logs name ids only, never a setting.
pub async fn deliver(hub: Arc<Hub>, account: Arc<Account>, machine: String) {
    let Ok(machine_id) = Uuid::parse_str(&machine) else { return };
    let mut newer = false;
    for _ in 0..SENDS {
        let save = match hub.db.global_settings(account.id).await {
            Ok(Some(save)) => save,
            Ok(None) => return,
            Err(e) => return eprintln!("pinned-server: could not read account {}'s global orchestrator save: {e}", account.id),
        };
        // A save recorded while this one was sent is sent too, pending or not: the write before
        // it may have replaced what the machine took from the page.
        if !newer && !save.pending.contains(&machine) {
            return;
        }
        let Ok(save_id) = Uuid::parse_str(&save.id) else { return };
        if send(&hub, &account, &machine, &save.settings).await.is_err() {
            return eprintln!("pinned-server: machine {machine} did not take global orchestrator save {save_id} of account {}; it stays pending", account.id);
        }
        match hub.db.took_global_settings(account.id, save_id, machine_id).await {
            Ok(true) => return eprintln!("pinned-server: machine {machine} took global orchestrator save {save_id} of account {}", account.id),
            Ok(false) => newer = true,
            Err(e) => return eprintln!("pinned-server: could not mark global orchestrator save {save_id} taken by machine {machine}: {e}"),
        }
    }
}

/// Writes `saved` to `machine` with the `host` it answers it has now.
async fn send(hub: &Hub, account: &Account, machine: &str, saved: &Settings) -> Result<(), String> {
    let current = hub.call_machine(account, machine, Call::GlobalOrchestratorSettings, WAIT).await.map_err(String::from)?;
    let current: Settings = serde_json::from_value(current).map_err(|e| e.to_string())?;
    let settings = resend(saved, &current);
    hub.call_machine(account, machine, Call::SetGlobalOrchestratorSettings { settings }, WAIT).await.map_err(String::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{pending, resend};
    use pinned_core::global_orchestrator::{BusyPolicy, ReportPolicy, Settings};
    use pinned_core::wire::MachineView;
    use uuid::Uuid;

    fn machine(id: Uuid, upstash_box: Option<&str>) -> MachineView {
        MachineView { id: id.to_string(), name: "m".into(), os: "linux".into(), online: false, last_seen: 0, upstash_box: upstash_box.map(str::to_owned), version: String::new(), update: None, state: None, background: None }
    }

    #[test]
    fn a_save_is_pending_on_every_machine_but_a_box_and_those_that_took_it() {
        let (mac, mini, vps, boxed) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let machines = [machine(mac, None), machine(mini, None), machine(vps, None), machine(boxed, Some("box-1"))];
        assert_eq!(pending(&machines, &[mac.to_string()]), vec![mini, vps]);
        assert_eq!(pending(&machines, &[mac.to_string(), mini.to_string(), vps.to_string()]), Vec::<Uuid>::new());
        assert_eq!(pending(&machines, &["not-a-machine".into()]), vec![mac, mini, vps], "an unknown id in took changes nothing");
    }

    #[test]
    fn a_resend_takes_the_saved_shared_fields_and_keeps_the_machines_host() {
        let saved = Settings { name: "Admiral".into(), host: false, busy: BusyPolicy::Refuse, reports: ReportPolicy::ToolOnly, group_reports: false, read_aloud: true, auto_merge: false, approval_profile: "Merge docs only.".into() };
        let on_host = Settings { name: "Orchestrator".into(), host: true, auto_merge: true, ..Settings::default() };
        assert_eq!(resend(&saved, &on_host), Settings { host: true, ..saved.clone() });
        let elsewhere = Settings { host: false, auto_merge: true, ..Settings::default() };
        assert_eq!(resend(&saved, &elsewhere), saved);
    }
}
