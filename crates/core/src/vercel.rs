use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

const API: &str = "https://ai-gateway.vercel.sh/v1";
const QUESTION: &str = "category";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// The team's credit left, in dollars, as the gateway writes it.
    pub balance: String,
}

#[derive(Deserialize)]
struct Models {
    data: Vec<Model>,
}

#[derive(Deserialize)]
struct Model {
    id: String,
    #[serde(rename = "type", default)]
    kind: String,
}

#[derive(Deserialize)]
struct Evaluation {
    answers: HashMap<String, Answer>,
}

#[derive(Deserialize)]
struct Answer {
    choice: Option<String>,
    #[serde(default)]
    probabilities: HashMap<String, f64>,
}

#[derive(Debug, PartialEq)]
pub struct Chosen {
    /// The position of the choice it picked.
    pub index: usize,
    /// How likely it found each choice, by position. A choice it gave no probability gets 0.
    pub probabilities: Vec<f64>,
}

async fn send(request: reqwest::RequestBuilder) -> Result<Value, String> {
    let response = request.send().await.map_err(|e| format!("Vercel AI Gateway unreachable: {e}"))?;
    let status = response.status();
    let body: Value = response.json().await.map_err(|e| format!("Vercel AI Gateway sent a bad response: {e}"))?;
    match status.as_u16() {
        200..=299 => Ok(body),
        401 => Err("Vercel AI Gateway refused the key: create one under AI Gateway, API Keys in the Vercel dashboard".into()),
        _ => Err(format!("Vercel AI Gateway answered {status}: {}", body["error"]["message"].as_str().unwrap_or_default())),
    }
}

/// Checks the key and reads the team's balance.
pub async fn account(key: &str) -> Result<Account, String> {
    let credits = send(reqwest::Client::new().get(format!("{API}/credits")).bearer_auth(key)).await?;
    let balance = match &credits["balance"] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err("Vercel AI Gateway sent no balance".into()),
    };
    Ok(Account { balance })
}

/// The ids of the models `choose` can ask, sorted. The list is public, so it needs no key.
pub async fn decision_models() -> Result<Vec<String>, String> {
    deciding(send(reqwest::Client::new().get(format!("{API}/models"))).await?)
}

/// The gateway types them `evaluation`; its docs now say `decision`.
fn deciding(models: Value) -> Result<Vec<String>, String> {
    let models: Models = serde_json::from_value(models).map_err(|e| format!("Vercel AI Gateway sent a bad model list: {e}"))?;
    let mut ids: Vec<String> = models.data.into_iter().filter(|m| m.kind == "evaluation" || m.kind == "decision").map(|m| m.id).collect();
    ids.sort_unstable();
    Ok(ids)
}

/// Asks `model` which of `choices`, as (name, description), fits `state`. Answers positions, so
/// the caller takes the choice without looking the name up again.
pub async fn choose(key: &str, model: &str, instructions: &str, choices: &[(&str, &str)], state: &impl Serialize) -> Result<Chosen, String> {
    let criteria: serde_json::Map<String, Value> = choices.iter().map(|(name, description)| (name.to_string(), json!(description))).collect();
    let body = json!({
        "model": model,
        "state": state,
        "questions": { QUESTION: { "type": "choice", "instructions": instructions, "criteria": criteria } },
    });
    chosen(send(reqwest::Client::new().post(format!("{API}/evaluate")).bearer_auth(key).json(&body)).await?, choices)
}

fn chosen(answer: Value, choices: &[(&str, &str)]) -> Result<Chosen, String> {
    let evaluation: Evaluation = serde_json::from_value(answer).map_err(|e| format!("Vercel AI Gateway sent a bad answer: {e}"))?;
    let answer = evaluation.answers.get(QUESTION).ok_or("the decision model gave no answer")?;
    let choice = answer.choice.as_deref().ok_or("the decision model gave no answer")?;
    let index = choices
        .iter()
        .position(|(name, _)| *name == choice)
        .ok_or_else(|| format!("the decision model answered {choice:?}, which is not one of the categories"))?;
    let probabilities = choices.iter().map(|(name, _)| answer.probabilities.get(*name).copied().unwrap_or(0.0)).collect();
    Ok(Chosen { index, probabilities })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHOICES: [(&str, &str); 3] = [("error", "a bug report"), ("doubt", "a question"), ("ignore", "anything else")];

    #[test]
    fn the_answer_is_the_position_of_the_category_the_model_chose() {
        let answer = json!({
            "model": "typesafe-ai/jev",
            "answers": { "category": { "type": "choice", "choice": "doubt", "probabilities": { "error": 0.02, "doubt": 0.97, "ignore": 0.01 } } },
            "usage": { "input_tokens": 80 },
        });
        assert_eq!(chosen(answer, &CHOICES), Ok(Chosen { index: 1, probabilities: vec![0.02, 0.97, 0.01] }));
        assert_eq!(
            chosen(json!({ "answers": { "category": { "type": "choice", "choice": "error", "probabilities": { "error": 0.6 } } } }), &CHOICES),
            Ok(Chosen { index: 0, probabilities: vec![0.6, 0.0, 0.0] }),
            "a choice the model gave no probability is unlikely"
        );
        assert_eq!(chosen(json!({ "answers": {} }), &CHOICES), Err("the decision model gave no answer".into()));
        assert_eq!(
            chosen(json!({ "answers": { "category": { "type": "choice", "choice": null } } }), &CHOICES),
            Err("the decision model gave no answer".into())
        );
        assert_eq!(
            chosen(json!({ "answers": { "category": { "type": "choice", "choice": "spam" } } }), &CHOICES),
            Err("the decision model answered \"spam\", which is not one of the categories".into())
        );
        assert!(chosen(json!({ "error": { "message": "nope" } }), &CHOICES).unwrap_err().starts_with("Vercel AI Gateway sent a bad answer"));
    }

    #[test]
    fn only_evaluation_and_decision_models_are_listed() {
        let models = json!({ "object": "list", "data": [
            { "id": "typesafe-ai/jev", "type": "evaluation" },
            { "id": "openai/gpt-6", "type": "language" },
            { "id": "baseten/mammoth-1", "type": "decision" },
            { "id": "cohere/rerank-v4-fast", "type": "reranking" },
            { "id": "untyped/model" },
        ] });
        assert_eq!(deciding(models), Ok(vec!["baseten/mammoth-1".to_owned(), "typesafe-ai/jev".to_owned()]));
    }

    #[test]
    #[ignore]
    fn decision_models_live() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let models = runtime.block_on(decision_models()).unwrap();
        println!("{models:?}");
        assert!(models.contains(&"typesafe-ai/jev".to_owned()), "{models:?}");
    }
}
