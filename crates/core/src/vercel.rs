//! Vercel's AI Gateway. A workflow can ask one of its models about what a trigger found
//! before the Claude Code session starts. The HTTP JSON is parsed here and never leaves the module.

use crate::connectors::{Ask, Question};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

const API: &str = "https://ai-gateway.vercel.sh/v1";
const CALL_WAIT: Duration = Duration::from_secs(60);

/// What the Connectors card shows of the key pinned keeps.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Account {
    /// The key's last characters, as `…a1b2`.
    pub key_hint: String,
}

/// A canned completion body for tests, so a workflow test can run the ask step without the network.
#[cfg(test)]
static TEST_CONTENT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Makes `ask` parse `content` as the model's reply. `None` calls the gateway.
#[cfg(test)]
pub(crate) fn set_test_content(content: Option<String>) {
    *TEST_CONTENT.lock().unwrap() = content;
}

async fn call(
    method: reqwest::Method,
    path: &str,
    key: Option<&str>,
    body: Option<&Value>,
) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(CALL_WAIT)
        .build()
        .map_err(|e| format!("Vercel AI Gateway: {e}"))?;
    let mut request = client.request(method, format!("{API}{path}"));
    if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("Vercel AI Gateway: unreachable: {e}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| format!("Vercel AI Gateway: sent a bad response: {e}"))?;
    let body: Value = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&text)
            .map_err(|e| format!("Vercel AI Gateway: sent a bad response: {e}"))?
    };
    if status.is_success() {
        return Ok(body);
    }
    // The gateway's own 401 tells the reader to set an environment variable pinned never reads.
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("Vercel AI Gateway: the key was rejected. Create a new one in the AI Gateway's API Keys page".into());
    }
    let message = body["error"]["message"]
        .as_str()
        .unwrap_or("request failed");
    Err(format!("Vercel AI Gateway: {message}"))
}

/// Checks the key against the gateway before pinned keeps it.
pub async fn account(key: &str) -> Result<Account, String> {
    call(reqwest::Method::GET, "/credits", Some(key.trim()), None).await?;
    Ok(Account {
        key_hint: crate::upstash::hint(key.trim()),
    })
}

/// Language models that can answer a JSON schema, sorted by id. The list needs no key.
pub async fn models() -> Result<Vec<String>, String> {
    model_ids(&call(reqwest::Method::GET, "/models", None, None).await?)
}

fn model_ids(body: &Value) -> Result<Vec<String>, String> {
    let data = body["data"]
        .as_array()
        .ok_or_else(|| "Vercel AI Gateway: sent a model list that isn't one".to_owned())?;
    let mut ids: Vec<String> = data
        .iter()
        .filter(|m| {
            m["type"].as_str() == Some("language")
                && m["tags"].as_array().is_some_and(|tags| {
                    tags.iter().any(|t| t.as_str() == Some("structured-output"))
                })
        })
        .filter_map(|m| m["id"].as_str().map(str::to_owned))
        .collect();
    ids.sort();
    Ok(ids)
}

/// One chat completion. Answers come back in question order, a choice in that choice's spelling.
pub async fn ask(key: &str, ask: &Ask, input: &str) -> Result<Vec<(String, String)>, String> {
    #[cfg(test)]
    if let Some(content) = TEST_CONTENT.lock().unwrap().clone() {
        return parse_completion(
            ask,
            &json!({ "choices": [{ "message": { "content": content } }] }),
        );
    }
    let body = completion_body(ask, input);
    parse_completion(
        ask,
        &call(
            reqwest::Method::POST,
            "/chat/completions",
            Some(key),
            Some(&body),
        )
        .await?,
    )
}

fn system_prompt(ask: &Ask) -> String {
    format!("{}\nAnswer every question.", ask.instructions.trim_end())
}

fn schema(ask: &Ask) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for question in &ask.questions {
        let mut property = json!({ "type": "string", "description": question.ask });
        if !question.choices.is_empty() {
            property["enum"] = json!(question.choices);
        }
        properties.insert(question.name.clone(), property);
        required.push(question.name.clone());
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn completion_body(ask: &Ask, input: &str) -> Value {
    json!({
        "model": ask.model,
        "messages": [
            { "role": "system", "content": system_prompt(ask) },
            { "role": "user", "content": input },
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": { "name": "answers", "strict": true, "schema": schema(ask) },
        },
    })
}

fn strip_fence(content: &str) -> &str {
    let trimmed = content.trim();
    let Some(after) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let after = after.strip_prefix("json").unwrap_or(after).trim_start();
    after.strip_suffix("```").unwrap_or(after).trim()
}

fn parse_completion(ask: &Ask, body: &Value) -> Result<Vec<(String, String)>, String> {
    if let Some(message) = body["error"]["message"].as_str() {
        return Err(format!("Vercel AI Gateway: {message}"));
    }
    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| format!("Vercel AI Gateway: {} sent no answer", ask.model))?;
    let value: Value = serde_json::from_str(strip_fence(content)).map_err(|_| {
        format!(
            "Vercel AI Gateway: {} sent an answer that is not JSON",
            ask.model
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        format!(
            "Vercel AI Gateway: {} sent an answer that is not an object",
            ask.model
        )
    })?;
    ask.questions
        .iter()
        .map(|question| answer_of(ask, question, object.get(&question.name)))
        .collect()
}

fn answer_of(
    ask: &Ask,
    question: &Question,
    value: Option<&Value>,
) -> Result<(String, String), String> {
    let raw = value.and_then(Value::as_str).ok_or_else(|| {
        format!(
            "Vercel AI Gateway: {} did not answer {}",
            ask.model, question.name
        )
    })?;
    let answer = if question.choices.is_empty() {
        raw.to_owned()
    } else {
        question
            .choices
            .iter()
            .find(|choice| choice.eq_ignore_ascii_case(raw))
            .cloned()
            .ok_or_else(|| format!("Vercel AI Gateway: {} answered {raw:?} for {}, which is not one of its choices", ask.model, question.name))?
    };
    Ok((question.name.clone(), answer))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_questions() -> Ask {
        Ask {
            model: "google/gemini-3.5-flash".into(),
            instructions: "Classify.\n".into(),
            questions: vec![
                Question {
                    name: "kind".into(),
                    ask: "What does it ask for?".into(),
                    choices: vec!["bug".into(), "question".into()],
                    stop_on: vec![],
                },
                Question {
                    name: "note".into(),
                    ask: "Anything else?".into(),
                    choices: vec![],
                    stop_on: vec![],
                },
            ],
        }
    }

    #[test]
    fn the_request_schema_is_one_string_per_question() {
        assert_eq!(
            completion_body(&two_questions(), "ENG-7: Broken button"),
            json!({
                "model": "google/gemini-3.5-flash",
                "messages": [
                    { "role": "system", "content": "Classify.\nAnswer every question." },
                    { "role": "user", "content": "ENG-7: Broken button" },
                ],
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": "answers",
                        "strict": true,
                        "schema": {
                            "type": "object",
                            "properties": {
                                "kind": { "type": "string", "description": "What does it ask for?", "enum": ["bug", "question"] },
                                "note": { "type": "string", "description": "Anything else?" },
                            },
                            "required": ["kind", "note"],
                            "additionalProperties": false,
                        },
                    },
                },
            })
        );
    }

    #[test]
    fn a_fenced_reply_parses_in_question_order_with_the_choices_spelling() {
        let body = json!({
            "choices": [{ "message": { "content": "```json\n{\"kind\":\"Question\",\"note\":\"free text\"}\n```" } }]
        });
        assert_eq!(
            parse_completion(&two_questions(), &body),
            Ok(vec![
                ("kind".into(), "question".into()),
                ("note".into(), "free text".into())
            ])
        );
    }

    #[test]
    fn a_choice_outside_the_list_is_refused() {
        let body = json!({ "choices": [{ "message": { "content": "{\"kind\":\"enhancement\",\"note\":\"x\"}" } }] });
        assert_eq!(
            parse_completion(&two_questions(), &body),
            Err("Vercel AI Gateway: google/gemini-3.5-flash answered \"enhancement\" for kind, which is not one of its choices".into())
        );
    }
}
