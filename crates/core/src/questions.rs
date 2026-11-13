//! Questionnaires an agent puts to the user, and the user's replies. Each belongs to a tab: while
//! it is open, the tab's view carries it in `TabView::questions` and reads as a question mid-turn,
//! so every surface that shows a session waiting on the user shows it. Saved to `questions.json`
//! in the config dir with each change, so an open questionnaire outlives a restart.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
    /// When the `question_user` call that waits on each questionnaire last asked how it stands.
    /// Not saved: after a restart no call waits.
    polled: HashMap<String, Instant>,
}

/// A `question_user` call that has not asked for this long is gone: its answer goes as a notice.
const CALL_GONE: Duration = Duration::from_secs(5);

impl Questions {
    /// The questionnaires saved in `dir`, none without one.
    pub fn load(dir: Option<&Path>) -> Self {
        let file = dir.map(|d| d.join(FILE));
        let asked = file.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        Questions { file, asked, polled: HashMap::new() }
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
        // A call that still waits hears it; a gone one gets no notice.
        asked.notified = true;
        self.closed();
        true
    }

    /// How the questionnaire `id` stands, for the `question_user` call that waits on it:
    /// `{"outcome":"open"}` while it waits, else the call's result, which then counts as delivered.
    pub fn status(&mut self, id: &str, now: Instant) -> Result<serde_json::Value, String> {
        let asked = self.asked.iter_mut().find(|a| a.questionnaire.id == id).ok_or_else(|| format!("no questions {id}"))?;
        if asked.is_open() {
            self.polled.insert(id.to_owned(), now);
            return Ok(serde_json::json!({ "outcome": "open", "id": id }));
        }
        let result = result(asked);
        if !asked.notified {
            asked.notified = true;
            self.save();
        }
        self.polled.remove(id);
        Ok(result)
    }

    /// Moves the questionnaires of Claude session `session` that still wait on the user, or whose
    /// outcome has not reached it, to `tab`, the tab that resumed that session. Whether any moved.
    pub fn rekey(&mut self, session: &str, tab: &str) -> bool {
        let open_here = self.open_on(tab).is_some();
        let mut moved = false;
        for asked in self.asked.iter_mut().filter(|a| a.session.as_deref() == Some(session) && a.tab != tab) {
            let waits = (asked.is_open() && !open_here) || (!asked.is_open() && !asked.notified);
            if waits {
                asked.tab = tab.to_owned();
                moved = true;
            }
        }
        if moved {
            self.save();
        }
        moved
    }

    /// The notice that carries a closed questionnaire's outcome to `tab`, when no `question_user`
    /// call waits on it any more and the session has not had it: its id and its text.
    pub fn late_notice(&self, tab: &str, now: Instant) -> Option<(String, String)> {
        self.asked
            .iter()
            .filter(|a| a.tab == tab && !a.notified)
            .filter(|a| self.polled.get(&a.questionnaire.id).is_none_or(|at| now.saturating_duration_since(*at) >= CALL_GONE))
            .find_map(|a| Some((a.questionnaire.id.clone(), notice(a)?)))
    }

    /// The outcome of `id` reached its session.
    pub fn notified(&mut self, id: &str) {
        if let Some(asked) = self.asked.iter_mut().find(|a| a.questionnaire.id == id) {
            asked.notified = true;
            self.save();
        }
    }

    /// Closes as skipped each open questionnaire whose wait limit passed by `now`, in Unix seconds.
    /// Whether any closed.
    pub fn expire(&mut self, now: u64) -> bool {
        let mut closed = false;
        for asked in self.asked.iter_mut().filter(|a| a.is_open()) {
            if asked.wait_limit_secs.is_some_and(|limit| asked.questionnaire.at.saturating_add(limit) <= now) {
                asked.outcome = Some(Outcome::Skipped);
                closed = true;
            }
        }
        if closed {
            self.closed();
        }
        closed
    }

    /// Whether any questionnaire waits on the user, or on delivering its outcome.
    pub fn unsettled(&self) -> bool {
        self.asked.iter().any(|a| a.is_open() || !a.notified)
    }

    #[cfg(all(test, unix))]
    pub(crate) fn set_wait_limit(&mut self, id: &str, secs: Option<u64>) {
        if let Some(asked) = self.asked.iter_mut().find(|a| a.questionnaire.id == id) {
            asked.wait_limit_secs = secs;
        }
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
/// (one unless it is multi-select) or typing something.
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
        if answer.chosen.is_empty() && answer.text.as_deref().is_none_or(|t| t.trim().is_empty()) {
            return Err(format!("question {n} has no answer: choose an option or type one"));
        }
    }
    Ok(())
}

/// The longest each text of a questionnaire is kept, in characters.
const HEADER_MAX: usize = 16;
const QUESTION_MAX: usize = 300;
const CHANGES_MAX: usize = 200;
const LABEL_MAX: usize = 60;
const DESCRIPTION_MAX: usize = 160;
const TITLE_MAX: usize = 80;

fn cut(text: &str, max: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max) {
        Some((at, _)) => text[..at].trim_end().to_owned(),
        None => text.to_owned(),
    }
}

/// `questions` as an agent may ask them: 1-4 questions of 2-4 options, each with something it
/// changes, its labels unique, and exactly one recommended option, the first. Texts are cut to
/// their lengths. The error says what to fix.
pub fn checked(title: Option<String>, questions: Vec<Question>) -> Result<(Option<String>, Vec<Question>), String> {
    if !(1..=4).contains(&questions.len()) {
        return Err(format!("ask 1 to 4 questions in one call, not {}", questions.len()));
    }
    let questions = questions
        .into_iter()
        .enumerate()
        .map(|(n, q)| {
            let n = n + 1;
            if q.question.trim().is_empty() {
                return Err(format!("question {n} has no text"));
            }
            if q.changes.trim().is_empty() {
                return Err(format!("question {n} does not say what its answer changes: a question whose answer changes nothing is not asked"));
            }
            if !(2..=4).contains(&q.options.len()) {
                return Err(format!("question {n} has {} options: give 2 to 4", q.options.len()));
            }
            let options: Vec<QOption> = q.options.into_iter().map(|o| QOption { label: cut(&o.label, LABEL_MAX), description: o.description.map(|d| cut(&d, DESCRIPTION_MAX)).filter(|d| !d.is_empty()), recommended: o.recommended }).collect();
            if options.iter().any(|o| o.label.is_empty()) {
                return Err(format!("question {n} has an option with no label"));
            }
            if options.iter().enumerate().any(|(i, o)| options[..i].iter().any(|p| p.label == o.label)) {
                return Err(format!("question {n} has two options with the same label"));
            }
            match options.iter().filter(|o| o.recommended).count() {
                0 => return Err(format!("question {n} has no recommended option: put your recommended option first and mark it recommended")),
                1 if options[0].recommended => {}
                1 => return Err(format!("question {n} recommends an option that is not first: put your recommended option first and mark it recommended")),
                _ => return Err(format!("question {n} recommends more than one option: mark only your recommendation, and put it first")),
            }
            Ok(Question {
                header: q.header.map(|h| cut(&h, HEADER_MAX)).filter(|h| !h.is_empty()),
                question: cut(&q.question, QUESTION_MAX),
                changes: cut(&q.changes, CHANGES_MAX),
                options,
                multi_select: q.multi_select,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok((title.map(|t| cut(&t, TITLE_MAX)).filter(|t| !t.is_empty()), questions))
}

/// What the `question_user` call answers once `asked` closed.
fn result(asked: &Asked) -> serde_json::Value {
    let id = &asked.questionnaire.id;
    match &asked.outcome {
        None => serde_json::json!({ "outcome": "open", "id": id }),
        Some(Outcome::Answered { answers }) => {
            let answers: Vec<serde_json::Value> = asked
                .questionnaire
                .questions
                .iter()
                .zip(answers)
                .map(|(q, a)| {
                    let mut answer = serde_json::json!({ "question": q.question, "chosen": a.chosen });
                    if let Some(header) = &q.header {
                        answer["header"] = header.clone().into();
                    }
                    if accepted(q, a) {
                        answer["recommended"] = true.into();
                    }
                    if let Some(text) = a.text.as_deref().filter(|t| !t.trim().is_empty()) {
                        answer["text"] = text.into();
                    }
                    answer
                })
                .collect();
            serde_json::json!({ "outcome": "answered", "id": id, "answers": answers, "next": "Go on with these answers." })
        }
        Some(Outcome::Skipped) => serde_json::json!({ "outcome": "skipped", "id": id, "next": "The user skipped these questions. Go on without them: take your recommended option for each and state every assumption you made where your work is read. Skipping can lead to wrong actions, so keep the assumptions easy to undo." }),
        Some(Outcome::Cancelled) => serde_json::json!({ "outcome": "cancelled", "id": id, "next": "The user stopped this turn. Do not go on with the work these questions were about." }),
    }
}

/// What the `question_user` call answers when its wait ends before the user does.
pub fn pending(id: &str) -> serde_json::Value {
    serde_json::json!({ "outcome": "pending", "id": id, "next": format!("The questions are still open. End your turn now; the answers arrive as a message that starts with [pinned] Answers to {id}.") })
}

/// The answer chose the recommended option, and only it.
fn accepted(question: &Question, answer: &Answer) -> bool {
    question.options.first().is_some_and(|o| o.recommended && answer.chosen == [o.label.clone()])
}

/// The notice typed into the session when its `question_user` call is gone. None for a
/// cancelled questionnaire, whose turn the user stopped.
fn notice(asked: &Asked) -> Option<String> {
    let q = &asked.questionnaire;
    let about = q.title.as_deref().map(|t| format!(" ({t})")).unwrap_or_default();
    match asked.outcome.as_ref()? {
        Outcome::Answered { answers } => {
            let lines: Vec<String> = q
                .questions
                .iter()
                .zip(answers)
                .map(|(question, answer)| {
                    let mut parts: Vec<String> = answer.chosen.clone();
                    if accepted(question, answer) {
                        parts[0] = format!("{} (your recommendation)", parts[0]);
                    }
                    if let Some(text) = answer.text.as_deref().filter(|t| !t.trim().is_empty()) {
                        parts.push(format!("{:?}", text.trim()));
                    }
                    format!("- {}: {}", question.header.as_deref().unwrap_or(&question.question), parts.join("; "))
                })
                .collect();
            Some(format!("[pinned] Answers to {}{about}:\n{}", q.id, lines.join("\n")))
        }
        Outcome::Skipped => Some(format!("[pinned] The user skipped {}{about}. Go on with your recommended options and state your assumptions.", q.id)),
        Outcome::Cancelled => None,
    }
}

/// `view` as it shows while `questionnaire` is open on its tab: a question mid-turn, as Claude
/// Code's own question shows, with the questionnaire beside it. Laid over the view, never written
/// into the tab, so the session's hooks reporting it at work do not hide the wait. It is laid over
/// only while the tab's process is alive, so the view reads as running, which a headless child at
/// work does not on its own: it has no session.
pub fn shown(mut view: TabView, questionnaire: &Questionnaire) -> TabView {
    view.running = true;
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
