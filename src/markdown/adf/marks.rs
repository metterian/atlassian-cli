use serde_json::Value;

pub fn apply_marks(text: String, marks: Option<&Vec<Value>>) -> String {
    let Some(marks) = marks else {
        return text;
    };

    let mut result = text;
    for mark in marks {
        let mark_type = mark.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let attrs = mark.get("attrs");

        result = match mark_type {
            "strong" => format!("**{}**", result),
            "em" => format!("*{}*", result),
            "code" => format!("`{}`", result),
            "strike" => format!("~~{}~~", result),
            "underline" => format!("<u>{}</u>", result),
            "link" => format_link(&result, attrs),
            "subsup" => format_subsup(&result, attrs),
            "textColor" => format_text_color(&result, attrs),
            "backgroundColor" => format_background_color(&result, attrs),
            _ => result,
        };
    }
    result
}

fn format_link(text: &str, attrs: Option<&Value>) -> String {
    let href = attrs
        .and_then(|a| a.get("href"))
        .and_then(|h| h.as_str())
        .unwrap_or("");

    if href.is_empty() {
        return text.to_string();
    }

    // Sanitize dangerous URL schemes
    let href_lower = href.to_lowercase();
    if href_lower.starts_with("javascript:")
        || href_lower.starts_with("vbscript:")
        || href_lower.starts_with("data:")
    {
        return text.to_string();
    }

    match attrs.and_then(|a| a.get("title")).and_then(|t| t.as_str()) {
        Some(title) => format!("[{}]({} \"{}\")", text, href, title),
        None => format!("[{}]({})", text, href),
    }
}

fn format_subsup(text: &str, attrs: Option<&Value>) -> String {
    let sub_type = attrs
        .and_then(|a| a.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("sub");

    match sub_type {
        "sup" => format!("<sup>{}</sup>", text),
        _ => format!("<sub>{}</sub>", text),
    }
}

fn format_text_color(text: &str, attrs: Option<&Value>) -> String {
    let color = attrs
        .and_then(|a| a.get("color"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    if color.is_empty() {
        text.to_string()
    } else {
        format!("<span style=\"color:{}\">{}</span>", color, text)
    }
}

fn format_background_color(text: &str, attrs: Option<&Value>) -> String {
    let color = attrs
        .and_then(|a| a.get("color"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    if color.is_empty() {
        text.to_string()
    } else {
        format!("<mark style=\"background:{}\">{}</mark>", color, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_no_marks() {
        assert_eq!(apply_marks("text".into(), None), "text");
    }

    #[test]
    fn test_strong() {
        let marks = vec![json!({"type": "strong"})];
        assert_eq!(apply_marks("bold".into(), Some(&marks)), "**bold**");
    }

    #[test]
    fn test_em() {
        let marks = vec![json!({"type": "em"})];
        assert_eq!(apply_marks("italic".into(), Some(&marks)), "*italic*");
    }

    #[test]
    fn test_code() {
        let marks = vec![json!({"type": "code"})];
        assert_eq!(apply_marks("code".into(), Some(&marks)), "`code`");
    }

    #[test]
    fn test_strike() {
        let marks = vec![json!({"type": "strike"})];
        assert_eq!(apply_marks("strike".into(), Some(&marks)), "~~strike~~");
    }

    #[test]
    fn test_underline() {
        let marks = vec![json!({"type": "underline"})];
        assert_eq!(apply_marks("under".into(), Some(&marks)), "<u>under</u>");
    }

    #[test]
    fn test_link() {
        let marks = vec![json!({"type": "link", "attrs": {"href": "https://example.com"}})];
        assert_eq!(
            apply_marks("click".into(), Some(&marks)),
            "[click](https://example.com)"
        );
    }

    #[test]
    fn test_link_with_title() {
        let marks = vec![
            json!({"type": "link", "attrs": {"href": "https://example.com", "title": "Example"}}),
        ];
        assert_eq!(
            apply_marks("click".into(), Some(&marks)),
            "[click](https://example.com \"Example\")"
        );
    }

    #[test]
    fn test_subsup_sub() {
        let marks = vec![json!({"type": "subsup", "attrs": {"type": "sub"}})];
        assert_eq!(apply_marks("2".into(), Some(&marks)), "<sub>2</sub>");
    }

    #[test]
    fn test_subsup_sup() {
        let marks = vec![json!({"type": "subsup", "attrs": {"type": "sup"}})];
        assert_eq!(apply_marks("2".into(), Some(&marks)), "<sup>2</sup>");
    }

    #[test]
    fn test_text_color() {
        let marks = vec![json!({"type": "textColor", "attrs": {"color": "#ff0000"}})];
        assert_eq!(
            apply_marks("red".into(), Some(&marks)),
            "<span style=\"color:#ff0000\">red</span>"
        );
    }

    #[test]
    fn test_background_color() {
        let marks = vec![json!({"type": "backgroundColor", "attrs": {"color": "#ffff00"}})];
        assert_eq!(
            apply_marks("highlight".into(), Some(&marks)),
            "<mark style=\"background:#ffff00\">highlight</mark>"
        );
    }

    #[test]
    fn test_multiple_marks() {
        let marks = vec![json!({"type": "strong"}), json!({"type": "em"})];
        assert_eq!(apply_marks("text".into(), Some(&marks)), "***text***");
    }
}
