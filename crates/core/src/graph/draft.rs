use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Draft {
    pub title: String,
    pub description: String,
}

pub(crate) fn draft_from_turn(items: &[crate::thread::Item]) -> Result<Draft, String> {
    let Some(start) = items.iter().rposition(|item| matches!(item, crate::thread::Item::User { .. })).map(|index| index + 1) else {
        return Err("the turn has no ticket fence".into());
    };
    let mut text = String::new();
    let mut error = None;
    for item in &items[start..] {
        match item {
            crate::thread::Item::Text { text: body, .. } => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(body);
            }
            crate::thread::Item::Error { text: body, .. } => error = Some(body.clone()),
            _ => {}
        }
    }
    match parse_draft(&text) {
        Ok(draft) => Ok(draft),
        Err(why) => Err(error.unwrap_or(why)),
    }
}

pub(crate) fn parse_draft(text: &str) -> Result<Draft, String> {
    let mut fences = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```ticket\n") {
        let body = &rest[start + "```ticket\n".len()..];
        let Some(end) = body.find("\n```") else {
            return Err("the turn has no ticket fence".into());
        };
        fences.push(&body[..end]);
        rest = &body[end + "\n```".len()..];
    }
    let [inside] = fences.as_slice() else {
        return Err(if fences.is_empty() { "the turn has no ticket fence".into() } else { "the turn has more than one ticket fence".into() });
    };
    let title_line = inside.lines().next().unwrap_or("");
    let title = title_line.strip_prefix("Title:").map(str::trim).filter(|title| !title.is_empty()).ok_or("the ticket fence has no title")?;
    let description = inside[title_line.len()..].strip_prefix("\n\n").map(str::trim).filter(|body| !body.is_empty()).ok_or("the ticket fence has no description")?;
    Ok(Draft { title: title.to_owned(), description: description.to_owned() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread::Item;

    fn user(text: &str) -> Item {
        Item::User { id: "u".into(), at: "t".into(), text: text.into(), images: 0, pictures: vec![] }
    }

    fn text(id: &str, text: &str) -> Item {
        Item::Text { id: id.into(), at: "t".into(), text: text.into() }
    }

    #[test]
    fn draft_from_turn_keeps_the_fence_after_the_last_user_and_ignores_an_earlier_reply() {
        let earlier = "```ticket\nTitle: Old title\n\nThe earlier talk already answered.\n```";
        let later = "```ticket\nTitle: Export 500s\n\nThe worker returns 500 when the cursor is past the end.\n```";
        let items = vec![text("a", earlier), user("File the bug."), text("b", later)];
        assert_eq!(
            draft_from_turn(&items),
            Ok(Draft {
                title: "Export 500s".into(),
                description: "The worker returns 500 when the cursor is past the end.".into(),
            })
        );
    }
}
