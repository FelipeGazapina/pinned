//! Claude's models: the one list the composer's picker and the delegate catalog both read. The
//! table below is what pinned knows; claude's own /model menu on a machine updates it, so a model
//! Claude Code adds shows up without a pinned release.

use serde::{Deserialize, Serialize};

use crate::delegate::Cost;

/// A Claude model as the picker draws it and `delegate_task` runs it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct ClaudeModel {
    pub slug: String,
    /// "Claude Fable 5.2".
    pub name: String,
    /// The row of claude's /model menu that picks it for the session alone, by its first word. A
    /// model without one restarts claude with `--model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<String>,
    /// The composer's reasoning levels; `delegate_task` offers the ones `claude --effort` takes.
    pub efforts: Vec<String>,
    /// Context window, in tokens.
    pub context: u64,
    pub cost: Cost,
    /// Gone from claude's /model menu: listed under Legacy models, still accepted.
    #[serde(default)]
    pub legacy: bool,
    /// Drawn with a New badge.
    #[serde(default)]
    pub new: bool,
    /// Has a fast mode.
    #[serde(default)]
    pub fast: bool,
}

/// A row of claude's /model menu that names a model: "Fable" picks Fable 5.2.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub menu: String,
    pub version: String,
}

impl MenuRow {
    pub fn slug(&self) -> String {
        format!("claude-{}-{}", self.menu.to_ascii_lowercase(), self.version.replace('.', "-"))
    }

    pub fn name(&self) -> String {
        format!("Claude {} {}", self.menu, self.version)
    }
}

const ALL: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultracode", "ultrathink"];
const NO_ULTRACODE: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultrathink"];
const OLD: &[&str] = &["low", "medium", "high", "max", "ultrathink"];
const OLDEST: &[&str] = &["low", "medium", "high", "max"];

struct Known {
    slug: &'static str,
    name: &'static str,
    menu: Option<&'static str>,
    efforts: &'static [&'static str],
    context: u64,
    cost: Cost,
    new: bool,
    fast: bool,
}

const fn current(slug: &'static str, name: &'static str, menu: &'static str, efforts: &'static [&'static str], cost: Cost, fast: bool) -> Known {
    Known { slug, name, menu: Some(menu), efforts, context: 1_000_000, cost, new: false, fast }
}

const fn legacy(slug: &'static str, name: &'static str, efforts: &'static [&'static str], context: u64, cost: Cost, fast: bool) -> Known {
    Known { slug, name, menu: None, efforts, context, cost, new: false, fast }
}

/// Newest first, as T3 Code lists them: the ones claude's /model menu offers, then the legacy ones.
const KNOWN: &[Known] = &[
    current("claude-fable-5-2", "Claude Fable 5.2", "Fable", ALL, Cost::Mid, false),
    current("claude-opus-5-5", "Claude Opus 5.5", "Opus", ALL, Cost::High, true),
    current("claude-sonnet-5-5", "Claude Sonnet 5.5", "Sonnet", NO_ULTRACODE, Cost::Mid, false),
    Known { new: true, ..current("claude-haiku-5-5", "Claude Haiku 5.5", "Haiku", NO_ULTRACODE, Cost::Low, false) },
    legacy("claude-fable-5-1", "Claude Fable 5.1", ALL, 1_000_000, Cost::Mid, false),
    legacy("claude-fable-5", "Claude Fable 5", ALL, 1_000_000, Cost::Mid, false),
    legacy("claude-opus-5", "Claude Opus 5", ALL, 1_000_000, Cost::High, true),
    legacy("claude-opus-4-8", "Claude Opus 4.8", ALL, 1_000_000, Cost::High, true),
    legacy("claude-opus-4-7", "Claude Opus 4.7", NO_ULTRACODE, 1_000_000, Cost::High, false),
    legacy("claude-opus-4-6", "Claude Opus 4.6", OLD, 200_000, Cost::High, true),
    legacy("claude-opus-4-5", "Claude Opus 4.5", OLDEST, 200_000, Cost::High, true),
    legacy("claude-sonnet-5", "Claude Sonnet 5", NO_ULTRACODE, 1_000_000, Cost::Mid, false),
    legacy("claude-sonnet-4-6", "Claude Sonnet 4.6", OLD, 200_000, Cost::Mid, false),
    legacy("claude-haiku-4-5", "Claude Haiku 4.5", &[], 200_000, Cost::Low, false),
];

/// The models pinned knows, before any machine's menu updates them.
pub fn known() -> Vec<ClaudeModel> {
    KNOWN
        .iter()
        .map(|k| ClaudeModel {
            slug: k.slug.into(),
            name: k.name.into(),
            menu: k.menu.map(Into::into),
            efforts: k.efforts.iter().map(|e| (*e).into()).collect(),
            context: k.context,
            cost: k.cost,
            legacy: k.menu.is_none(),
            new: k.new,
            fast: k.fast,
        })
        .collect()
}

/// The models of `known` as a machine's claude lists them in `menu`: each row's model is current
/// and picked from that row, a model the table lacks takes its family's place with its traits,
/// and a model no row names becomes legacy. Current ones come first; an empty menu changes nothing.
pub fn merged(known: Vec<ClaudeModel>, menu: &[MenuRow]) -> Vec<ClaudeModel> {
    if menu.is_empty() {
        return known;
    }
    let mut list = known;
    for row in menu {
        let slug = row.slug();
        if let Some(model) = list.iter_mut().find(|m| m.slug == slug) {
            model.menu = Some(row.menu.clone());
            model.legacy = false;
            continue;
        }
        let family = format!("claude-{}-", row.menu.to_ascii_lowercase());
        let like = list
            .iter()
            .position(|m| m.menu.as_deref() == Some(row.menu.as_str()))
            .or_else(|| list.iter().position(|m| m.slug.starts_with(&family)));
        let model = match like.map(|at| &list[at]) {
            Some(like) => ClaudeModel { slug, name: row.name(), menu: Some(row.menu.clone()), legacy: false, new: true, ..like.clone() },
            None => ClaudeModel {
                slug: slug.clone(),
                name: row.name(),
                menu: Some(row.menu.clone()),
                efforts: NO_ULTRACODE.iter().map(|e| (*e).into()).collect(),
                context: 200_000,
                cost: crate::delegate::cost_of(&slug),
                legacy: false,
                new: true,
                fast: false,
            },
        };
        list.insert(like.unwrap_or(0), model);
    }
    let listed: Vec<String> = menu.iter().map(MenuRow::slug).collect();
    for model in &mut list {
        if !listed.contains(&model.slug) {
            model.menu = None;
            model.legacy = true;
            model.new = false;
        }
    }
    let (current, legacy): (Vec<_>, Vec<_>) = list.into_iter().partition(|m| !m.legacy);
    current.into_iter().chain(legacy).collect()
}

/// The model rows of claude's /model menu on a screen: the numbered rows under "Select model",
/// each named by its first word and the version after that word further along the row
/// ("Fable 5.2   For your toughest…" in 2.1.321, "Fable   Fable 5.1 · Most capable…" in 2.1.310).
/// The Default row and rows with no version are left out. `None` when the menu isn't up.
pub fn menu_rows(lines: &[String]) -> Option<Vec<MenuRow>> {
    let last = lines.iter().rev().find(|l| !l.trim().is_empty())?;
    if !last.contains("Esc to cancel") {
        return None;
    }
    let top = lines.iter().rposition(|l| l.trim() == "Select model")?;
    let rows: Vec<&str> = lines[top + 1..].iter().filter_map(|l| numbered(l)).collect();
    if rows.is_empty() {
        return None;
    }
    Some(rows.into_iter().filter_map(model_row).collect())
}

/// The text after "N. " on a numbered menu row, with the cursor mark before it allowed.
fn numbered(line: &str) -> Option<&str> {
    let line = line.trim_start().trim_start_matches('❯').trim_start();
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let rest = line[digits..].strip_prefix('.')?;
    (digits > 0 && rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

fn model_row(head: &str) -> Option<MenuRow> {
    let words: Vec<&str> = head.split_whitespace().collect();
    let menu = *words.first()?;
    if menu == "Default" || !menu.chars().next()?.is_ascii_uppercase() {
        return None;
    }
    let version = words.windows(2).find(|w| w[0] == menu && is_version(w[1]))?[1];
    Some(MenuRow { menu: menu.into(), version: version.into() })
}

fn is_version(word: &str) -> bool {
    !word.is_empty() && word.split('.').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(name: &str) -> Vec<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/web-sweep/screens").join(name);
        std::fs::read_to_string(path).unwrap().lines().map(String::from).collect()
    }

    fn rows(lines: &[String]) -> Vec<String> {
        menu_rows(lines).unwrap().iter().map(|r| format!("{} {}", r.menu, r.version)).collect()
    }

    fn shown(models: &[ClaudeModel]) -> Vec<String> {
        models
            .iter()
            .map(|m| format!("{}{}{}", m.slug, m.menu.as_ref().map(|r| format!(" [{r}]")).unwrap_or_default(), if m.legacy { " legacy" } else { "" }))
            .collect()
    }

    #[test]
    fn reads_the_model_rows_of_both_menu_layouts() {
        assert_eq!(rows(&screen("claude-2.1.321-model-menu.txt")), ["Opus 5.5", "Fable 5.2", "Sonnet 5.5", "Haiku 5.5"]);
        assert_eq!(rows(&screen("claude-2.1.310-model-menu.txt")), ["Sonnet 5.5", "Fable 5.1", "Opus 5.5", "Haiku 5.5"]);
        assert_eq!(menu_rows(&screen("claude-2.1.310-effort-menu.txt")), None);
    }

    #[test]
    fn the_menu_claude_shows_today_changes_nothing() {
        let menu = menu_rows(&screen("claude-2.1.321-model-menu.txt")).unwrap();
        assert_eq!(merged(known(), &menu), known());
    }

    #[test]
    fn a_model_the_menu_adds_takes_its_familys_place_and_the_old_one_turns_legacy() {
        let capture = screen("claude-2.1.321-model-menu.txt").join("\n").replace("Fable 5.2", "Fable 5.3");
        let menu = menu_rows(&capture.lines().map(String::from).collect::<Vec<_>>()).unwrap();
        let models = merged(known(), &menu);
        assert_eq!(
            shown(&models)[..7],
            [
                "claude-fable-5-3 [Fable]",
                "claude-opus-5-5 [Opus]",
                "claude-sonnet-5-5 [Sonnet]",
                "claude-haiku-5-5 [Haiku]",
                "claude-fable-5-2 legacy",
                "claude-fable-5-1 legacy",
                "claude-fable-5 legacy",
            ]
        );
        let fable = &models[0];
        assert_eq!((fable.name.as_str(), fable.efforts.len(), fable.context, fable.cost, fable.new), ("Claude Fable 5.3", 7, 1_000_000, Cost::Mid, true));
        assert_eq!(models.len(), known().len() + 1);
    }

    #[test]
    fn a_family_pinned_never_saw_is_listed_first() {
        let menu = [MenuRow { menu: "Opus".into(), version: "5.5".into() }, MenuRow { menu: "Mythos".into(), version: "1".into() }];
        let models = merged(known(), &menu);
        assert_eq!(shown(&models)[..2], ["claude-mythos-1 [Mythos]", "claude-opus-5-5 [Opus]"]);
        assert_eq!(models[0].name, "Claude Mythos 1");
        assert!(models[2..].iter().all(|m| m.legacy));
    }
}
