use super::*;

use crate::claude_models::ClaudeModel;

/// The levels `claude --effort` takes; the composer's ultracode and ultrathink are not among them.
const CLAUDE_EFFORT: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const CURSOR_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "fast"];

/// What a model costs to run, coarsely: routing by tier beats stale per-token prices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Cost {
    /// Haiku and flash models: sweeps, translations, first drafts.
    Low,
    /// The middle default when nothing is known.
    #[default]
    Mid,
    /// Opus and max efforts: architecture and hairy bugs.
    High,
}

impl Cost {
    pub fn name(self) -> &'static str {
        match self {
            Cost::Low => "low",
            Cost::Mid => "mid",
            Cost::High => "high",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    pub id: String,
    pub label: String,
    pub efforts: Vec<String>,
    pub cost: Cost,
}

pub fn provider_models(provider: Provider) -> Vec<ModelChoice> {
    match provider {
        Provider::Claude => claude_choices(&crate::claude_models::known()),
        Provider::Codex => vec![],
        Provider::Cursor => vec![],
    }
}

/// Claude's models as `delegate_task` offers them: a legacy one still runs, named as legacy.
pub fn claude_choices(models: &[ClaudeModel]) -> Vec<ModelChoice> {
    models
        .iter()
        .map(|m| {
            let name = m.name.strip_prefix("Claude ").unwrap_or(&m.name);
            ModelChoice {
                id: m.slug.clone(),
                label: if m.legacy { format!("{name} (legacy)") } else { name.into() },
                efforts: m.efforts.iter().filter(|e| CLAUDE_EFFORT.contains(&e.as_str())).cloned().collect(),
                cost: m.cost,
            }
        })
        .collect()
}

/// `{model, effort}` becomes `model-effort` when `--list-models` printed that id.
pub fn map_cursor_model(ids: &[String], model: &str, effort: Option<&str>) -> String {
    match effort.map(str::trim).filter(|effort| !effort.is_empty()) {
        Some(effort) if ids.iter().any(|id| id == &format!("{model}-{effort}")) => {
            format!("{model}-{effort}")
        }
        _ => model.to_owned(),
    }
}

pub fn cursor_models(list: &str) -> Vec<ModelChoice> {
    let listed = parse_model_list(list);
    let mut order = vec![];
    let mut groups: HashMap<String, (String, Vec<String>)> = HashMap::new();
    for (id, label) in &listed {
        let (base, effort) = split_cursor_id(id);
        let entry = groups.entry(base.clone()).or_insert_with(|| {
            order.push(base.clone());
            (String::new(), vec![])
        });
        let cleaned = clean_label(label);
        if entry.0.is_empty() || cleaned.len() < entry.0.len() {
            entry.0 = cleaned;
        }
        if let Some(effort) = effort {
            if !entry.1.iter().any(|have| have == &effort) {
                entry.1.push(effort);
            }
        }
    }
    order
        .into_iter()
        .map(|id| {
            let (label, mut efforts) = groups.remove(&id).unwrap_or_default();
            efforts.sort_by_key(|effort| {
                CURSOR_EFFORTS
                    .iter()
                    .position(|known| known == effort)
                    .unwrap_or(CURSOR_EFFORTS.len())
            });
            ModelChoice { id: id.clone(), label, efforts, cost: cost_of(&id) }
        })
        .collect()
}

/// A model's cost tier, with Claude's models as pinned knows them. A machine that learned its
/// claude's /model menu asks [`cost_in`] with its own list.
pub fn cost_of(id: &str) -> Cost {
    cost_in(&crate::claude_models::known(), id)
}

/// A model's cost tier: the one `claude` gives it when it lists the model, otherwise a rough one
/// from the words of its id. A costly word wins, so `opus-fast` is high, and `minimax` is no `mini`.
pub fn cost_in(claude: &[ClaudeModel], id: &str) -> Cost {
    if let Some(model) = claude.iter().find(|m| m.slug == id) {
        return model.cost;
    }
    let id = id.to_ascii_lowercase();
    let words: Vec<&str> = id.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    let says = |hints: &[&str]| words.iter().any(|word| hints.contains(word));
    if says(&["opus", "max", "pro", "ultra", "thinking"]) {
        Cost::High
    } else if says(&["haiku", "flash", "mini", "fast", "nano", "lite"]) {
        Cost::Low
    } else {
        Cost::Mid
    }
}

pub(crate) fn parse_model_list(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (id, label) = line.split_once(" - ")?;
            let id = id.trim();
            if id.is_empty() || id.contains(' ') {
                return None;
            }
            Some((id.to_owned(), label.trim().to_owned()))
        })
        .collect()
}

fn split_cursor_id(id: &str) -> (String, Option<String>) {
    let (body, fast) = match id.strip_suffix("-fast") {
        Some(body) if !body.is_empty() => (body, true),
        _ => (id, false),
    };
    for effort in ["xhigh", "medium", "high", "low", "max"] {
        if let Some(root) = body.strip_suffix(&format!("-{effort}")) {
            if !root.is_empty() {
                return (root.to_owned(), Some(effort.to_owned()));
            }
        }
    }
    if fast {
        (body.to_owned(), Some("fast".into()))
    } else {
        (id.to_owned(), None)
    }
}

fn clean_label(label: &str) -> String {
    let mut label = label
        .trim()
        .trim_end_matches(" (current)")
        .trim()
        .to_owned();
    let suffixes = [
        " Extra High Fast",
        " Extra High",
        " Low Fast",
        " Medium Fast",
        " High Fast",
        " Max Fast",
        " Low",
        " Medium",
        " High",
        " Max",
        " Fast",
    ];
    while let Some(shorter) = suffixes.iter().find_map(|suffix| label.strip_suffix(suffix).map(|rest| rest.trim().to_owned())) {
        label = shorter;
    }
    label.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub struct ListedAccount {
    pub id: String,
    pub provider: Provider,
    pub name: String,
    pub signed_in: bool,
    pub bin_found: bool,
    /// A usage limit stopped this account; route around it unless nothing else can run.
    pub limited: bool,
    /// Why and until when, while `limited` is true.
    pub limit: Option<LimitNote>,
    pub models: Vec<ModelChoice>,
    pub constraints: Vec<String>,
}

/// What stopped an account, as `orchestrator_capabilities` states it.
pub struct LimitNote {
    pub reason: String,
    /// Unix seconds when the provider says the limit lifts.
    pub resume_at: Option<u64>,
    /// Unix seconds when pinned lets a task try the account again, for a limit with no reset time.
    pub retry_at: Option<u64>,
}

pub struct Inherited {
    pub account: String,
    pub model: Option<String>,
    pub access: Access,
}

pub fn capabilities(parent: &Inherited, accounts: &[ListedAccount], search: Option<&str>) -> Value {
    let needle = search
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| text.to_ascii_lowercase());
    let accounts = accounts
        .iter()
        .filter_map(|account| account_json(account, needle.as_deref()))
        .collect::<Vec<_>>();
    json!({
        "inheritedProviderInstanceId": parent.account,
        "inheritedModel": parent.model,
        "runtimeMode": parent.access.runtime_mode(),
        "providers": accounts,
        "features": {
            "appOwnedSubagents": true,
            "asyncPolling": false,
            "cancellation": true,
        },
    })
}

fn account_json(account: &ListedAccount, needle: Option<&str>) -> Option<Value> {
    let driver_kind = serde_json::to_value(account.provider)
        .ok()?
        .as_str()?
        .to_owned();
    let account_hit = needle.is_none_or(|needle| {
        account.id.to_ascii_lowercase().contains(needle)
            || account.name.to_ascii_lowercase().contains(needle)
            || driver_kind.to_ascii_lowercase().contains(needle)
    });
    let models: Vec<&ModelChoice> = account
        .models
        .iter()
        .filter(|model| {
            account_hit
                || needle.is_none_or(|needle| {
                    model.id.to_ascii_lowercase().contains(needle)
                        || model.label.to_ascii_lowercase().contains(needle)
                })
        })
        .collect();
    if !account_hit && models.is_empty() {
        return None;
    }
    let ready = account.signed_in && account.bin_found;
    Some(json!({
        "providerInstanceId": account.id,
        "driverKind": driver_kind,
        "displayName": account.name,
        "models": models.iter().map(|model| model_json(model)).collect::<Vec<_>>(),
        "runtimeModes": ["auto", "full-access"],
        "canRunChildTask": ready,
        "canRunCrossProviderChildTask": ready && account.provider != Provider::Cursor,
        "limited": account.limited,
        "limit": account.limit.as_ref().map(|limit| json!({ "reason": limit.reason, "resumeAt": limit.resume_at, "retryAt": limit.retry_at })),
        "constraints": account.constraints,
    }))
}

fn model_json(model: &ModelChoice) -> Value {
    let options = if model.efforts.is_empty() {
        vec![]
    } else {
        vec![json!({"id": "effort", "values": model.efforts, "default": null})]
    };
    json!({"id": model.id, "label": model.label, "cost": model.cost.name(), "options": options})
}

pub fn codex_constraints() -> Vec<String> {
    vec!["Any model id works. Pass it as target.model.".into()]
}

pub fn cursor_constraints() -> Vec<String> {
    vec![
        "Cursor supports only its built-in account.".into(),
        "A delegated child cannot start another pinned worker. A Cursor session cannot delegate.".into(),
    ]
}
