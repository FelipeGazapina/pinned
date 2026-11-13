//! Questionnaires an agent puts to the user, and the user's replies. Each belongs to a tab: while
//! it is open, the tab's view carries it in `TabView::questions` and reads as a question mid-turn,
//! so every surface that shows a session waiting on the user shows it. Saved to `questions.json`
//! in the config dir with each change, so an open questionnaire outlives a restart.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agent::AgentStatus;
use crate::wire::{TabId, TabView};

const FILE: &str = "questions.json";

/// How many closed questionnaires the file keeps, oldest dropped first.
const CLOSED_KEEP: usize = 50;

/// What a tab with open questions says it asks, as Claude Code's own question does.
pub const MESSAGE: &str = "Claude has questions";

/// The time a questionnaire waits on the user before its wait ends, when its task sets none: no
/// limit. An unattended run waits until the user answers or skips; it never skips by itself.
pub const DEFAULT_WAIT_LIMIT_SECS: Option<u64> = None;

fn default_wait_limit() -> Option<u64> {
    DEFAULT_WAIT_LIMIT_SECS
}

/// The questions an agent asks the user in one go.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Questionnaire {
    /// `q-7`, unique on its machine.
    pub id: String,
    /// What the questions are about, in a few words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub questions: Vec<Question>,
    /// The project keys whose message from the global orchestrator waits on the answers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<String>,
    /// Unix seconds it was asked.
    pub at: u64,
}

/// One question of a questionnaire.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[schemars(rename = "QuestionnaireQuestion")]
pub struct Question {
    /// A short chip label, such as `Branch`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    pub question: String,
    /// What the agent does differently depending on the answer.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub changes: String,
    /// The agent's recommendation first.
    pub options: Vec<QOption>,
    /// The user may choose more than one option.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multi_select: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct QOption {
    pub label: String,
    /// One line: what choosing it leads to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recommended: bool,
}

/// How the user closed a questionnaire.
///
/// `{"kind":"answered","answers":[{"chosen":["main"]}]}`, `{"kind":"skipped"}`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuestionsReply {
    /// One answer per question, in the questions' order.
    Answered { answers: Vec<Answer> },
    /// The agent goes on with its recommendations and states its assumptions.
    Skipped,
}

/// The answer to one question: the labels of the options chosen, what the user typed, or both.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Answer {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chosen: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// How a questionnaire ended.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    Answered { answers: Vec<Answer> },
    Skipped,
    /// The user stopped the turn, or the tab closed.
    Cancelled,
    /// Its worker ended before anyone answered.
    Ended,
}

/// One questionnaire on this machine, open or closed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Asked {
    pub tab: TabId,
    /// The Claude session of the tab when it asked, so a tab that resumes it can be found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The delegated task whose worker asked, whose time limit pauses while it waits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub questionnaire: Questionnaire,
    /// None while open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// The outcome reached the session that asked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notified: bool,
    /// Seconds the wait may last, None for no limit. Off unless the task sets one.
    #[serde(default = "default_wait_limit", skip_serializing_if = "Option::is_none")]
    pub wait_limit_secs: Option<u64>,
}

impl Asked {
    pub fn is_open(&self) -> bool {
        self.outcome.is_none()
    }
}

/// Every questionnaire on this machine, oldest first.
#[derive(Default)]
pub struct Questions {
    file: Option<PathBuf>,
    asked: Vec<Asked>,
}

impl Questions {
    /// The questionnaires saved in `dir`, none without one.
    pub fn load(dir: Option<&Path>) -> Self {
        let file = dir.map(|d| d.join(FILE));
        let asked = file.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        Questions { file, asked }
    }

    /// Opens a questionnaire on `tab` and answers its id. A tab holds one open questionnaire at a time.
    pub fn open(&mut self, tab: &str, session: Option<String>, task: Option<String>, title: Option<String>, questions: Vec<Question>, projects: Vec<String>, at: u64) -> Result<String, String> {
        if let Some(open) = self.open_on(tab) {
            return Err(format!("questions {} are still open: wait for them", open.id));
        }
        let next = self.asked.iter().filter_map(|a| a.questionnaire.id.strip_prefix("q-")?.parse::<u64>().ok()).max().unwrap_or(0) + 1;
        let id = format!("q-{next}");
        let questionnaire = Questionnaire { id: id.clone(), title, questions, projects, at };
        self.asked.push(Asked { tab: tab.to_owned(), session, task, questionnaire, outcome: None, notified: false, wait_limit_secs: default_wait_limit() });
        self.save();
        Ok(id)
    }

    /// The questionnaire open on `tab`, which its view shows.
    pub fn open_on(&self, tab: &str) -> Option<&Questionnaire> {
        self.asked.iter().find(|a| a.tab == tab && a.is_open()).map(|a| &a.questionnaire)
    }

    pub fn get(&self, id: &str) -> Option<&Asked> {
        self.asked.iter().find(|a| a.questionnaire.id == id)
    }

    /// The tasks whose worker waits on open questions.
    pub fn waiting_tasks(&self) -> impl Iterator<Item = &str> {
        self.asked.iter().filter(|a| a.is_open()).filter_map(|a| a.task.as_deref())
    }

    /// Closes the questionnaire `id` open on `tab` with the user's reply, after checking the reply
    /// fits its questions.
    pub fn answer(&mut self, tab: &str, id: &str, reply: QuestionsReply) -> Result<(), String> {
        let asked = self.asked.iter_mut().find(|a| a.questionnaire.id == id && a.tab == tab).ok_or_else(|| format!("no questions {id} on {tab}"))?;
        if !asked.is_open() {
            return Err(format!("questions {id} are already closed"));
        }
        asked.outcome = Some(match reply {
            QuestionsReply::Skipped => Outcome::Skipped,
            QuestionsReply::Answered { answers } => {
                fits(&asked.questionnaire, &answers)?;
                Outcome::Answered { answers }
            }
        });
        self.closed();
        Ok(())
    }

    /// Closes the questionnaire open on `tab` as cancelled. Whether one was.
    pub fn cancel_tab(&mut self, tab: &str) -> bool {
        let Some(asked) = self.asked.iter_mut().find(|a| a.tab == tab && a.is_open()) else { return false };
        asked.outcome = Some(Outcome::Cancelled);
        self.closed();
        true
    }

    /// The open questionnaires with the task whose worker asked each: its `task`, else the one its
    /// tab runs, which `tab_task` answers. Each is (id, task, the second it was asked).
    pub fn open_tasks(&self, tab_task: impl Fn(&str) -> Option<String>) -> Vec<(String, String, u64)> {
        self.asked.iter().filter(|a| a.is_open()).filter_map(|a| Some((a.questionnaire.id.clone(), a.task.clone().or_else(|| tab_task(&a.tab))?, a.questionnaire.at))).collect()
    }

    /// Closes the open questionnaire `id` as ended: its worker's task ended with nobody having
    /// answered, so nobody is left to tell. Whether one was open.
    pub fn end(&mut self, id: &str) -> bool {
        let Some(asked) = self.asked.iter_mut().find(|a| a.questionnaire.id == id && a.is_open()) else { return false };
        asked.outcome = Some(Outcome::Ended);
        asked.notified = true;
        self.closed();
        true
    }

    fn closed(&mut self) {
        let closed = self.asked.iter().filter(|a| !a.is_open()).count();
        if closed > CLOSED_KEEP {
            let mut drop = closed - CLOSED_KEEP;
            self.asked.retain(|a| {
                let gone = drop > 0 && !a.is_open();
                drop -= usize::from(gone);
                !gone
            });
        }
        self.save();
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(bytes) = serde_json::to_vec_pretty(&self.asked) else { return };
        let tmp = file.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }
}

/// Whether `answers` answer `questionnaire`: one per question, each choosing only its options
/// (one unless it is multi-select, and none twice) or typing something.
fn fits(questionnaire: &Questionnaire, answers: &[Answer]) -> Result<(), String> {
    if answers.len() != questionnaire.questions.len() {
        return Err(format!("questions {} has {} questions, the reply answers {}", questionnaire.id, questionnaire.questions.len(), answers.len()));
    }
    for (n, (question, answer)) in questionnaire.questions.iter().zip(answers).enumerate() {
        let n = n + 1;
        if let Some(label) = answer.chosen.iter().find(|label| !question.options.iter().any(|o| &o.label == *label)) {
            return Err(format!("question {n} has no option {label:?}"));
        }
        if answer.chosen.len() > 1 && !question.multi_select {
            return Err(format!("question {n} takes one option"));
        }
        if let Some(label) = answer.chosen.iter().enumerate().find_map(|(i, label)| answer.chosen[..i].contains(label).then_some(label)) {
            return Err(format!("question {n} chooses {label:?} twice"));
        }
        if answer.chosen.is_empty() && answer.text.as_deref().is_none_or(|t| t.trim().is_empty()) {
            return Err(format!("question {n} has no answer: choose an option or type one"));
        }
    }
    Ok(())
}

/// `view` as it shows while `questionnaire` is open on its tab: a question mid-turn, as Claude
/// Code's own question shows, with the questionnaire beside it. Laid over the view, never written
/// into the tab, so the session's hooks reporting it at work do not hide the wait.
pub fn shown(mut view: TabView, questionnaire: &Questionnaire) -> TabView {
    view.status = Some(AgentStatus::NeedsInput);
    view.interrupted = true;
    view.message = Some(MESSAGE.into());
    view.ask_detail = questionnaire.questions.first().map(|q| q.question.clone());
    view.questions = Some(questionnaire.clone());
    view
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(text: &str, labels: &[&str]) -> Question {
        Question {
            header: None,
            question: text.into(),
            changes: "what gets built".into(),
            options: labels.iter().enumerate().map(|(i, l)| QOption { label: (*l).into(), description: None, recommended: i == 0 }).collect(),
            multi_select: false,
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pinned-questions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_tab_holds_one_open_questionnaire_and_ids_count_up() {
        let mut q = Questions::default();
        let id = q.open("tab-1", None, None, None, vec![question("Base on main?", &["main", "release"])], vec![], 1).unwrap();
        assert_eq!(id, "q-1");
        assert_eq!(q.open("tab-1", None, None, None, vec![question("Again?", &["yes", "no"])], vec![], 2), Err("questions q-1 are still open: wait for them".into()));
        assert_eq!(q.open("tab-2", None, None, None, vec![question("Other?", &["yes", "no"])], vec![], 2).unwrap(), "q-2");
        q.answer("tab-1", "q-1", QuestionsReply::Skipped).unwrap();
        assert_eq!(q.open("tab-1", None, None, None, vec![question("Next?", &["yes", "no"])], vec![], 3).unwrap(), "q-3");
    }

    #[test]
    fn a_reply_must_fit_the_questions_it_answers() {
        let mut q = Questions::default();
        let id = q.open("tab-1", None, None, None, vec![question("Base?", &["main", "release"]), question("Phone too?", &["yes", "no"])], vec![], 1).unwrap();
        let answered = |answers: Vec<Answer>| QuestionsReply::Answered { answers };
        let chose = |labels: &[&str]| Answer { chosen: labels.iter().map(|l| (*l).to_string()).collect(), text: None };
        assert_eq!(q.answer("tab-2", &id, QuestionsReply::Skipped), Err("no questions q-1 on tab-2".into()));
        assert_eq!(q.answer("tab-1", &id, answered(vec![chose(&["main"])])), Err("questions q-1 has 2 questions, the reply answers 1".into()));
        assert_eq!(q.answer("tab-1", &id, answered(vec![chose(&["main"]), chose(&["maybe"])])), Err("question 2 has no option \"maybe\"".into()));
        assert_eq!(q.answer("tab-1", &id, answered(vec![chose(&["main", "release"]), chose(&["yes"])])), Err("question 1 takes one option".into()));
        assert_eq!(q.answer("tab-1", &id, answered(vec![chose(&["main"]), Answer { chosen: vec![], text: Some(" ".into()) }])), Err("question 2 has no answer: choose an option or type one".into()));
        assert!(q.open_on("tab-1").is_some(), "a refused reply leaves it open");
        let both = q.open("tab-2", None, None, None, vec![Question { multi_select: true, ..question("Which apps?", &["desktop", "phone"]) }], vec![], 2).unwrap();
        assert_eq!(q.answer("tab-2", &both, answered(vec![chose(&["desktop", "desktop"])])), Err("question 1 chooses \"desktop\" twice".into()));
        q.answer("tab-2", &both, answered(vec![chose(&["desktop", "phone"])])).unwrap();
        let typed = Answer { chosen: vec![], text: Some("Only the desktop one".into()) };
        q.answer("tab-1", &id, answered(vec![chose(&["main"]), typed.clone()])).unwrap();
        assert_eq!(q.get(&id).unwrap().outcome, Some(Outcome::Answered { answers: vec![chose(&["main"]), typed] }));
        assert_eq!(q.open_on("tab-1"), None);
        assert_eq!(q.answer("tab-1", &id, QuestionsReply::Skipped), Err("questions q-1 are already closed".into()));
    }

    #[test]
    fn open_questionnaires_survive_a_restart_from_questions_json() {
        let dir = temp("reload");
        let mut q = Questions::load(Some(&dir));
        let id = q.open("tab-1", Some("sess-1".into()), Some("task-1".into()), Some("Checkout".into()), vec![question("Base?", &["main", "release"])], vec!["github.com/o/site".into()], 7).unwrap();
        q.open("tab-2", None, None, None, vec![question("Other?", &["yes", "no"])], vec![], 8).unwrap();
        q.cancel_tab("tab-2");
        let asked = q.get(&id).unwrap().clone();
        drop(q);

        let again = Questions::load(Some(&dir));
        assert_eq!(again.get(&id), Some(&asked));
        assert_eq!(again.open_on("tab-1").map(|q| q.title.as_deref()), Some(Some("Checkout")));
        assert_eq!(again.get("q-2").unwrap().outcome, Some(Outcome::Cancelled));
        assert_eq!(again.waiting_tasks().collect::<Vec<_>>(), ["task-1"]);
        assert_eq!(asked.wait_limit_secs, None, "no wait limit unless a task sets one");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_questionnaire_whose_worker_ended_closes_as_ended_and_survives_a_restart_closed() {
        let dir = temp("ended");
        let mut q = Questions::load(Some(&dir));
        let own = q.open("tab-1", None, Some("task-1".into()), None, vec![question("Which database?", &["postgres", "mysql"])], vec![], 1).unwrap();
        let by_tab = q.open("tab-2", None, None, None, vec![question("Which region?", &["eu", "us"])], vec![], 2).unwrap();
        q.open("tab-3", None, None, None, vec![question("No task?", &["yes", "no"])], vec![], 3).unwrap();
        let tab_task = |tab: &str| (tab == "tab-2").then(|| "task-2".to_owned());
        assert_eq!(q.open_tasks(tab_task), [(own.clone(), "task-1".to_owned(), 1), (by_tab.clone(), "task-2".to_owned(), 2)], "a questionnaire with no task of its own has its tab's");
        assert!(q.end(&own) && q.end(&by_tab));
        assert!(!q.end(&own), "already closed");
        assert_eq!(q.get(&own).unwrap().outcome, Some(Outcome::Ended));
        assert_eq!((q.open_on("tab-1"), q.waiting_tasks().count()), (None, 0));
        assert!(q.open_on("tab-3").is_some(), "another tab's questionnaire stays open");
        let again = Questions::load(Some(&dir));
        assert_eq!(again.get(&by_tab).unwrap().outcome, Some(Outcome::Ended), "the outcome is saved");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn closed_questionnaires_are_kept_up_to_a_limit_and_open_ones_always() {
        let mut q = Questions::default();
        q.open("tab-open", None, None, None, vec![question("Open?", &["yes", "no"])], vec![], 0).unwrap();
        for n in 0..CLOSED_KEEP + 5 {
            let tab = format!("tab-{n}");
            q.open(&tab, None, None, None, vec![question("Done?", &["yes", "no"])], vec![], 0).unwrap();
            q.cancel_tab(&tab);
        }
        assert_eq!(q.asked.len(), CLOSED_KEEP + 1);
        assert!(q.open_on("tab-open").is_some());
        assert_eq!(q.asked[1].tab, "tab-5", "the oldest closed ones went first");
    }
}
