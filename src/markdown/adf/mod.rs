mod blocks;
mod inline;
mod marks;

use crate::markdown::common::normalize_whitespace;
use serde_json::Value;

pub fn adf_to_markdown(adf: &Value) -> String {
    let Some(content) = adf.get("content").and_then(|c| c.as_array()) else {
        return String::new();
    };

    let blocks: Vec<String> = content
        .iter()
        .filter_map(|node| blocks::convert_block_node(node, 0))
        .collect();

    normalize_whitespace(&blocks.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_empty_doc() {
        let adf = json!({"type": "doc", "version": 1, "content": []});
        assert_eq!(adf_to_markdown(&adf), "");
    }

    #[test]
    fn test_simple_paragraph() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Hello"}]}]
        });
        assert_eq!(adf_to_markdown(&adf), "Hello");
    }

    #[test]
    fn test_multiple_paragraphs() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "First"}]},
                {"type": "paragraph", "content": [{"type": "text", "text": "Second"}]}
            ]
        });
        assert_eq!(adf_to_markdown(&adf), "First\n\nSecond");
    }

    #[test]
    fn test_formatted_text() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "paragraph",
                "content": [
                    {"type": "text", "text": "bold", "marks": [{"type": "strong"}]},
                    {"type": "text", "text": " and "},
                    {"type": "text", "text": "italic", "marks": [{"type": "em"}]}
                ]
            }]
        });
        assert_eq!(adf_to_markdown(&adf), "**bold** and *italic*");
    }

    #[test]
    fn test_heading_and_paragraph() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [
                {"type": "heading", "attrs": {"level": 1}, "content": [{"type": "text", "text": "Title"}]},
                {"type": "paragraph", "content": [{"type": "text", "text": "Content"}]}
            ]
        });
        assert_eq!(adf_to_markdown(&adf), "# Title\n\nContent");
    }

    #[test]
    fn test_code_block() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "codeBlock",
                "attrs": {"language": "rust"},
                "content": [{"type": "text", "text": "let x = 1;"}]
            }]
        });
        let result = adf_to_markdown(&adf);
        assert!(result.contains("```rust"));
        assert!(result.contains("let x = 1;"));
    }

    #[test]
    fn test_link() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "click here",
                    "marks": [{"type": "link", "attrs": {"href": "https://example.com"}}]
                }]
            }]
        });
        assert_eq!(adf_to_markdown(&adf), "[click here](https://example.com)");
    }

    #[test]
    fn test_table() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "table",
                "content": [
                    {"type": "tableRow", "content": [
                        {"type": "tableHeader", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Header"}]}]}
                    ]},
                    {"type": "tableRow", "content": [
                        {"type": "tableCell", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Cell"}]}]}
                    ]}
                ]
            }]
        });
        let result = adf_to_markdown(&adf);
        assert!(result.contains("| Header |"));
        assert!(result.contains("| --- |"));
        assert!(result.contains("| Cell |"));
    }
}
