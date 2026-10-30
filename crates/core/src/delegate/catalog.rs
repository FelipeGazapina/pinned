use super::*;

const CLAUDE_EFFORT: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const CLAUDE_MODELS: &[(&str, &str)] = &[
    ("claude-opus-5-5", "Opus 5.5"),
    ("claude-sonnet-5-5", "Sonnet 5.5"),
    ("claude-fable-5-1", "Fable 5.1"),
    ("claude-haiku-5-5", "Haiku 5.5"),
];
const CURSOR_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "fast"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    pub id: String,
    pub label: String,
    pub efforts: Vec<String>,
}

pub fn provider_models(provider: Provider) -> Vec<ModelChoice> {
    match provider {
        Provider::Claude => CLAUDE_MODELS
            .iter()
            .map(|(id, label)| ModelChoice {
                id: (*id).into(),
                label: (*label).into(),
                efforts: CLAUDE_EFFORT.iter().map(|e| (*e).into()).collect(),
            })
            .collect(),
        Provider::Codex => vec![],
        Provider::Cursor => vec![],
    }
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
            ModelChoice { id, label, efforts }
        })
        .collect()
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
    pub models: Vec<ModelChoice>,
    pub constraints: Vec<String>,
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
        "constraints": account.constraints,
    }))
}

fn model_json(model: &ModelChoice) -> Value {
    let options = if model.efforts.is_empty() {
        vec![]
    } else {
        vec![json!({"id": "effort", "values": model.efforts, "default": null})]
    };
    json!({"id": model.id, "label": model.label, "options": options})
}

pub fn codex_constraints() -> Vec<String> {
    vec!["Any model id works. Pass it as target.model.".into()]
}

pub fn cursor_constraints() -> Vec<String> {
    vec![
        "Cursor supports only its built-in account.".into(),
        "Cursor sessions can't delegate further.".into(),
    ]
}
