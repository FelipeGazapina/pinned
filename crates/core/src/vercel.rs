//! Vercel's AI Gateway. A workflow can ask one of its models, chat or decision, about what a
//! trigger found before the Claude Code session starts. The HTTP JSON is parsed here and never leaves the module.

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

/// `language` on the gateway is chat. `evaluation` is a decision model.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Chat,
    Decision,
}

/// A gateway model id, and whether it answers chat or a decision.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub id: String,
    pub kind: Kind,
}

/// A canned completion body for tests, so a workflow test can run the ask step without the network.
#[cfg(test)]
static TEST_CONTENT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Makes `ask` parse `content` as a chat completion and skip the gateway. `None` calls it.
#[cfg(test)]
pub(crate) fn set_test_content(content: Option<String>) {
    *TEST_CONTENT.lock().unwrap() = content;
}

async fn exchange(
    method: reqwest::Method,
    path: &str,
    key: Option<&str>,
    body: Option<&Value>,
) -> Result<(reqwest::StatusCode, Value), String> {
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
    Ok((status, body))
}

fn accepted(status: reqwest::StatusCode, body: Value) -> Result<Value, String> {
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

async fn call(
    method: reqwest::Method,
    path: &str,
    key: Option<&str>,
    body: Option<&Value>,
) -> Result<Value, String> {
    let (status, body) = exchange(method, path, key, body).await?;
    accepted(status, body)
}

/// Checks the key against the gateway before pinned keeps it.
pub async fn account(key: &str) -> Result<Account, String> {
    call(reqwest::Method::GET, "/credits", Some(key.trim()), None).await?;
    Ok(Account {
        key_hint: crate::upstash::hint(key.trim()),
    })
}

/// Chat models tagged `structured-output`, and every decision model, sorted by id. The list needs no key.
pub async fn models() -> Result<Vec<Model>, String> {
    models_from(&call(reqwest::Method::GET, "/models", None, None).await?)
}

fn models_from(body: &Value) -> Result<Vec<Model>, String> {
    let data = body["data"]
        .as_array()
        .ok_or_else(|| "Vercel AI Gateway: sent a model list that isn't one".to_owned())?;
    let mut models: Vec<Model> = data.iter().filter_map(listed).collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

fn kind_of_type(type_name: &str) -> Option<Kind> {
    match type_name {
        "language" => Some(Kind::Chat),
        "evaluation" => Some(Kind::Decision),
        _ => None,
    }
}

fn listed(model: &Value) -> Option<Model> {
    let id = model["id"].as_str()?.to_owned();
    let kind = kind_of_type(model["type"].as_str()?)?;
    let structured = model["tags"].as_array().is_some_and(|tags| tags.iter().any(|tag| tag.as_str() == Some("structured-output")));
    (kind == Kind::Decision || structured).then_some(Model { id, kind })
}

fn not_a_model(model: &str) -> String {
    format!("{model} is not a chat or decision model on the gateway")
}

fn parse_kind(model: &str, type_name: Option<&str>) -> Result<Kind, String> {
    type_name.and_then(kind_of_type).ok_or_else(|| not_a_model(model))
}

async fn kind_of(model: &str) -> Result<Kind, String> {
    let (status, body) = exchange(reqwest::Method::GET, &format!("/models/{model}"), None, None).await?;
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(not_a_model(model));
    }
    let body = accepted(status, body)?;
    parse_kind(model, body["type"].as_str())
}

/// One gateway call. Answers come back in question order, a choice in that choice's spelling.
pub async fn ask(key: &str, ask: &Ask, input: &str) -> Result<Vec<(String, String)>, String> {
    #[cfg(test)]
    if let Some(content) = TEST_CONTENT.lock().unwrap().clone() {
        return parse_completion(
            ask,
            &json!({ "choices": [{ "message": { "content": content } }] }),
        );
    }
    match kind_of(&ask.model).await? {
        Kind::Chat => {
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
        Kind::Decision => {
            let body = decision_body(ask, input)?;
            parse_decision(
                ask,
                &call(reqwest::Method::POST, "/decisions", Some(key), Some(&body)).await?,
            )
        }
    }
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

fn decision_body(ask: &Ask, input: &str) -> Result<Value, String> {
    let mut questions = Vec::with_capacity(ask.questions.len());
    for question in &ask.questions {
        if question.choices.is_empty() {
            return Err(format!(
                "{} is a decision model and answers only questions with choices: give {} choices",
                ask.model, question.name
            ));
        }
        let choices: Vec<Value> = question
            .choices
            .iter()
            .map(|value| json!({ "value": value }))
            .collect();
        questions.push(json!({
            "type": "choice",
            "name": question.name,
            "instructions": question.ask,
            "choices": choices,
        }));
    }
    Ok(json!({
        "model": ask.model,
        "input": format!("{}\n\n{}", ask.instructions.trim_end(), input),
        "questions": questions,
    }))
}

enum DecisionReply {
    Choice(String),
    Refusal,
}

fn decision_reply(value: &Value) -> Option<DecisionReply> {
    match value["type"].as_str()? {
        "choice" => Some(DecisionReply::Choice(value["choice"].as_str()?.to_owned())),
        "refusal" => Some(DecisionReply::Refusal),
        _ => None,
    }
}

fn parse_decision(ask: &Ask, body: &Value) -> Result<Vec<(String, String)>, String> {
    if let Some(message) = body["error"]["message"].as_str() {
        return Err(format!("Vercel AI Gateway: {message}"));
    }
    let answers = body["answers"].as_array().ok_or_else(|| {
        format!("Vercel AI Gateway: {} sent no answer", ask.model)
    })?;
    ask.questions
        .iter()
        .enumerate()
        .map(|(i, question)| decision_answer(ask, question, answers.get(i)))
        .collect()
}

fn decision_answer(
    ask: &Ask,
    question: &Question,
    value: Option<&Value>,
) -> Result<(String, String), String> {
    let missing = || format!("Vercel AI Gateway: {} did not answer {}", ask.model, question.name);
    let Some(reply) = value.and_then(decision_reply) else {
        return Err(missing());
    };
    match reply {
        DecisionReply::Refusal => Err(format!(
            "Vercel AI Gateway: {} refused {}",
            ask.model, question.name
        )),
        DecisionReply::Choice(raw) => {
            let answer = choice_spelling(question, &raw).ok_or_else(|| {
                format!(
                    "Vercel AI Gateway: {} answered {raw:?} for {}, which is not one of its choices",
                    ask.model, question.name
                )
            })?;
            Ok((question.name.clone(), answer))
        }
    }
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
        choice_spelling(question, raw).ok_or_else(|| {
            format!(
                "Vercel AI Gateway: {} answered {raw:?} for {}, which is not one of its choices",
                ask.model, question.name
            )
        })?
    };
    Ok((question.name.clone(), answer))
}

fn choice_spelling(question: &Question, raw: &str) -> Option<String> {
    question
        .choices
        .iter()
        .find(|choice| choice.eq_ignore_ascii_case(raw))
        .cloned()
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

    fn decision_ask() -> Ask {
        Ask {
            model: "typesafe-ai/jev".into(),
            instructions: "Classify.\n".into(),
            questions: vec![
                Question {
                    name: "kind".into(),
                    ask: "What does it ask for?".into(),
                    choices: vec!["bug".into(), "question".into()],
                    stop_on: vec![],
                },
                Question {
                    name: "agent".into(),
                    ask: "Can a coding agent handle it?".into(),
                    choices: vec!["yes".into(), "no".into()],
                    stop_on: vec!["no".into()],
                },
            ],
        }
    }

    #[test]
    fn a_decision_request_is_one_choice_question_per_question() {
        assert_eq!(
            decision_body(&decision_ask(), "ENG-7: Broken button"),
            Ok(json!({
                "model": "typesafe-ai/jev",
                "input": "Classify.\n\nENG-7: Broken button",
                "questions": [
                    {
                        "type": "choice",
                        "name": "kind",
                        "instructions": "What does it ask for?",
                        "choices": [{ "value": "bug" }, { "value": "question" }],
                    },
                    {
                        "type": "choice",
                        "name": "agent",
                        "instructions": "Can a coding agent handle it?",
                        "choices": [{ "value": "yes" }, { "value": "no" }],
                    },
                ],
            }))
        );
    }

    #[test]
    fn a_decision_reply_uses_the_choices_spelling() {
        let body = json!({
            "answers": [
                { "type": "choice", "name": "kind", "choice": "Question", "confidence": 0.81 },
                { "type": "choice", "name": "agent", "choice": "YES", "probabilities": [0.1, 0.9] },
            ]
        });
        assert_eq!(
            parse_decision(&decision_ask(), &body),
            Ok(vec![
                ("kind".into(), "question".into()),
                ("agent".into(), "yes".into()),
            ])
        );
    }

    #[test]
    fn a_refusal_names_the_model_and_the_question() {
        let body = json!({
            "answers": [
                { "type": "choice", "name": "kind", "choice": "bug" },
                { "type": "refusal", "name": "agent" },
            ]
        });
        assert_eq!(
            parse_decision(&decision_ask(), &body),
            Err("Vercel AI Gateway: typesafe-ai/jev refused agent".into())
        );
    }

    #[test]
    fn a_missing_decision_answer_names_the_model_and_the_question() {
        let body = json!({
            "answers": [{ "type": "choice", "name": "kind", "choice": "bug" }]
        });
        assert_eq!(
            parse_decision(&decision_ask(), &body),
            Err("Vercel AI Gateway: typesafe-ai/jev did not answer agent".into())
        );
    }

    #[test]
    fn a_free_text_question_on_a_decision_model_is_an_error() {
        let mut ask = two_questions();
        ask.model = "typesafe-ai/jev".into();
        assert_eq!(
            decision_body(&ask, "ENG-7: Broken button"),
            Err("typesafe-ai/jev is a decision model and answers only questions with choices: give note choices".into())
        );
    }

    #[test]
    fn gateway_types_parse_as_chat_or_decision() {
        assert_eq!(parse_kind("google/gemini-3.5-flash", Some("language")), Ok(Kind::Chat));
        assert_eq!(parse_kind("typesafe-ai/jev", Some("evaluation")), Ok(Kind::Decision));
        assert_eq!(
            parse_kind("vendor/embed", Some("embedding")),
            Err("vendor/embed is not a chat or decision model on the gateway".into())
        );
        assert_eq!(
            parse_kind("vendor/embed", None),
            Err("vendor/embed is not a chat or decision model on the gateway".into())
        );
    }

    #[test]
    fn the_list_keeps_structured_chat_models_and_every_decision_model() {
        let body = json!({
            "data": [
                { "id": "openai/gpt", "type": "language", "tags": ["tool-use"] },
                { "id": "typesafe-ai/jev", "type": "evaluation", "tags": ["tool-use"] },
                { "id": "google/gemini-3.5-flash", "type": "language", "tags": ["structured-output"] },
                { "id": "vendor/embed", "type": "embedding" },
                { "id": "microsoft/microsoft-decision-1", "type": "evaluation" },
            ]
        });
        assert_eq!(
            serde_json::to_value(models_from(&body).unwrap()).unwrap(),
            json!([
                { "id": "google/gemini-3.5-flash", "kind": "chat" },
                { "id": "microsoft/microsoft-decision-1", "kind": "decision" },
                { "id": "typesafe-ai/jev", "kind": "decision" },
            ])
        );
    }
}
