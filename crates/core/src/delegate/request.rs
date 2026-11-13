use super::*;

/// T3's `delegate_task` arguments, accepted under those names.
pub fn parse_request(args: &Value) -> Result<Request, String> {
    let task = args["task"]
        .as_str()
        .map(str::trim)
        .filter(|task| !task.is_empty())
        .ok_or("task is required")?
        .to_owned();
    // One line from here on: the title goes into notices, a box's machine name and listings.
    let title = args["title"]
        .as_str()
        .map(|title| crate::pinned_lines::flat(title, 80))
        .filter(|title| !title.is_empty())
        .or_else(|| {
            task.lines()
                .map(|line| crate::pinned_lines::flat(line, 80))
                .find(|line| !line.is_empty())
        })
        .unwrap_or_else(|| "task".to_owned());
    let target = &args["target"];
    let (account, driver_kind) = account_from(target)?;
    let model = optional_str(&target["model"]);
    if let Some(model) = &model {
        if !token_ok(model) {
            return Err(format!("{model:?} is not a model id"));
        }
    }
    let effort = effort_from(&target["options"]);
    if let Some(effort) = &effort {
        if !token_ok(effort) {
            return Err(format!("{effort:?} is not an effort"));
        }
    }
    let machine = optional_str(&target["machine"]);
    let boxed = box_from(&target["box"])?;
    if machine.is_some() && boxed.is_some() {
        return Err("pass target.machine or target.box, not both".into());
    }
    let isolation = match args["isolation"].as_str() {
        None | Some("shared") => Isolation::Shared,
        Some("worktree") => Isolation::Worktree,
        Some(other) => {
            return Err(format!(
                "isolation must be shared or worktree, not {other:?}"
            ))
        }
    };
    let mode = match args["mode"].as_str() {
        None | Some("async") => Mode::Async,
        Some("wait") => Mode::Wait,
        Some(other) => return Err(format!("mode must be async or wait, not {other:?}")),
    };
    let timeout_ms = match args.get("timeoutMs") {
        None | Some(Value::Null) => DEFAULT_TIMEOUT_MS,
        Some(value) => value
            .as_u64()
            .filter(|ms| *ms > 0)
            .ok_or("timeoutMs must be a number of milliseconds above 0")?,
    };
    let millis = |key: &str| match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().filter(|ms| *ms > 0).map(Some).ok_or(format!("{key} must be a number of milliseconds above 0")),
    };
    let limit_ms = millis("limitMs")?;
    let budget_ms = millis("budgetMs")?;
    if let (Some(budget), Some(limit)) = (budget_ms, limit_ms) {
        if budget >= limit {
            return Err(format!("budgetMs must be below limitMs, which is {limit} ms; pass a larger limitMs"));
        }
    }
    let interaction = match args["interactionMode"].as_str() {
        None | Some("inherit") => Interaction::Inherit,
        Some("default") => Interaction::Default,
        Some("plan") => Interaction::Plan,
        Some(other) => {
            return Err(format!(
                "interactionMode must be inherit, default, or plan, not {other:?}"
            ))
        }
    };
    let runtime = match args["runtimeMode"].as_str() {
        None | Some("inherit") => None,
        Some("auto") => Some(RuntimeMode::Auto),
        Some("full-access") => Some(RuntimeMode::FullAccess),
        Some(other) => {
            return Err(format!(
                "runtimeMode must be inherit, auto, or full-access, not {other:?}. A child runs headless and cannot pause for approval"
            ))
        }
    };
    let role = optional_str(&args["role"]);
    let tools = args
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| tools.iter().filter_map(Value::as_str).map(str::trim).filter(|tool| !tool.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default();
    let client_request_id = optional_str(&args["clientRequestId"]);
    let heavy = match args.get("heavy") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(heavy)) => Some(*heavy),
        Some(other) => return Err(format!("heavy must be true or false, not {other}")),
    };
    Ok(Request {
        task,
        title,
        target: Target {
            account,
            model,
            effort,
            driver_kind,
            machine,
            boxed,
        },
        isolation,
        mode,
        timeout_ms,
        budget_ms,
        limit_ms,
        interaction,
        runtime,
        role,
        tools,
        client_request_id,
        heavy,
    })
}

/// The budget notice and the run limit, in ms, under a time limit of `ceiling_ms`: the limit asked
/// for, at most the ceiling, which is also the default; the notice asked for, or else two hours
/// and at most half the limit.
pub fn run_limits(budget_ms: Option<u64>, limit_ms: Option<u64>, ceiling_ms: u64) -> Result<(u64, u64), String> {
    let limit = limit_ms.map_or(ceiling_ms, |ms| ms.min(ceiling_ms));
    match budget_ms {
        Some(ms) if ms >= limit => Err(format!("budgetMs must be below the run limit, which is {limit} ms; ask for less, or raise Time limit in Settings › Way of working")),
        Some(ms) => Ok((ms, limit)),
        None => Ok((DEFAULT_BUDGET_MS.min(limit / 2), limit)),
    }
}

fn account_from(target: &Value) -> Result<(Option<String>, Option<String>), String> {
    let id = optional_str(&target["providerInstanceId"]);
    let kind = optional_str(&target["driverKind"]);
    if let (Some(id), Some(kind)) = (&id, &kind) {
        if let Some(provider) = Provider::ALL.into_iter().find(|provider| provider.spec().builtin == id) {
            require_driver(id, provider, kind)?;
        }
    }
    if id.is_some() {
        return Ok((id, kind));
    }
    let Some(kind) = kind else { return Ok((None, None)) };
    let provider = driver_provider(&kind)?;
    Ok((Some(crate::accounts::Account::builtin(provider).id), Some(kind)))
}

/// Both were named, and the account's provider is not `driverKind`.
pub fn require_driver(account_id: &str, provider: Provider, kind: &str) -> Result<(), String> {
    let expected = driver_provider(kind)?;
    if provider != expected {
        return Err(format!(
            "providerInstanceId \"{account_id}\" is {} and does not match driverKind \"{kind}\"",
            provider.spec().name
        ));
    }
    Ok(())
}

fn driver_provider(kind: &str) -> Result<Provider, String> {
    match kind {
        "claude" => Ok(Provider::Claude),
        "codex" => Ok(Provider::Codex),
        "cursor" => Ok(Provider::Cursor),
        other => Err(format!("driverKind must be claude, codex, or cursor, not {other:?}")),
    }
}

/// A `box` field that reads `true` or `{ size?, repo? }`, as `send_to_captain` takes it.
pub fn box_or_true<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<BoxTarget>, D::Error> {
    box_from(&Value::deserialize(de)?).map_err(serde::de::Error::custom)
}

/// `target.box`: `true`, or `{ size?, repo? }`.
fn box_from(value: &Value) -> Result<Option<BoxTarget>, String> {
    match value {
        Value::Null | Value::Bool(false) => Ok(None),
        Value::Bool(true) => Ok(Some(BoxTarget::default())),
        Value::Object(_) => serde_json::from_value(value.clone()).map(Some).map_err(|e| format!("target.box: {e}")),
        other => Err(format!("target.box must be true or {{ size, repo }}, not {other}")),
    }
}

fn effort_from(options: &Value) -> Option<String> {
    optional_str(&options["effort"])
        .or_else(|| optional_str(&options["reasoning_effort"]))
        .or_else(|| option_value(options, "effort"))
        .or_else(|| option_value(options, "reasoning_effort"))
}

fn option_value(options: &Value, id: &str) -> Option<String> {
    options.as_array()?.iter().find_map(|item| (item["id"].as_str() == Some(id)).then(|| optional_str(&item["value"])).flatten())
}

fn optional_str(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn token_ok(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-.[]_".contains(&byte))
}
