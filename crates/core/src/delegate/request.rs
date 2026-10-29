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
    let account = optional_str(&target["providerInstanceId"]);
    let model = optional_str(&target["model"]);
    if let Some(model) = &model {
        if !token_ok(model) {
            return Err(format!("{model:?} is not a model id"));
        }
    }
    let effort = optional_str(&target["options"]["effort"])
        .or(optional_str(&target["options"]["reasoning_effort"]));
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
        None => None,
        Some("auto") => Some(RuntimeMode::Auto),
        Some("full-access") => Some(RuntimeMode::FullAccess),
        Some(other) => {
            return Err(format!(
                "runtimeMode must be auto or full-access, not {other:?}"
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
        },
        isolation,
        mode,
        timeout_ms,
        interaction,
        runtime,
        role,
        client_request_id,
    })
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
