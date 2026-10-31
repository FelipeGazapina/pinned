use super::*;

/// T3's `delegate_task` arguments, accepted under those names.
pub fn parse_request(args: &Value) -> Result<Request, String> {
    let task = args["task"]
        .as_str()
        .map(str::trim)
        .filter(|task| !task.is_empty())
        .ok_or("task is required")?
        .to_owned();
    let title = match args["title"]
        .as_str()
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        Some(title) => thread::clip(title, 80),
        None => thread::clip(
            task.lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("task")
                .trim(),
            80,
        ),
    };
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
    let limit_ms = millis("limitMs")?.unwrap_or(DEFAULT_LIMIT_MS);
    let budget_ms = match millis("budgetMs")? {
        Some(ms) if ms >= limit_ms => return Err(format!("budgetMs must be below limitMs, which is {limit_ms} ms; pass a larger limitMs")),
        Some(ms) => ms,
        None => DEFAULT_BUDGET_MS.min(limit_ms),
    };
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
    let client_request_id = optional_str(&args["clientRequestId"]);
    Ok(Request {
        task,
        title,
        target: Target {
            account,
            model,
            effort,
            driver_kind,
        },
        isolation,
        mode,
        timeout_ms,
        budget_ms,
        limit_ms,
        interaction,
        runtime,
        role,
        client_request_id,
    })
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
