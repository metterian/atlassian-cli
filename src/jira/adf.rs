use anyhow::Result;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde_json::{Value, json};

/// Validates that a Value is a valid ADF (Atlassian Document Format) document.
///
/// A valid ADF document must have:
/// - type: must be exactly "doc"
/// - version: must be integer 1
/// - content: must be an array (can be empty)
///
/// This function performs minimal validation - only checking the top-level document structure.
/// Full content node validation is delegated to the Jira API for performance.
pub fn validate_adf(value: &Value) -> Result<()> {
    // Check if value is an object
    let obj = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Invalid ADF: must be an object"))?;

    // Check required field: type
    let doc_type = obj
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid ADF: missing required field 'type'"))?;

    if doc_type != "doc" {
        anyhow::bail!("Invalid ADF: type must be 'doc', got '{}'", doc_type);
    }

    // Check required field: version
    let version = obj
        .get("version")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("Invalid ADF: missing required field 'version'"))?;

    if version != 1 {
        anyhow::bail!("Invalid ADF: version must be 1, got {}", version);
    }

    // Check required field: content
    let content = obj
        .get("content")
        .ok_or_else(|| anyhow::anyhow!("Invalid ADF: missing required field 'content'"))?;

    if !content.is_array() {
        anyhow::bail!("Invalid ADF: content must be array");
    }

    Ok(())
}

/// Converts text (with Markdown support) to an ADF document.
///
/// Parses Markdown syntax using pulldown-cmark and builds ADF nodes.
/// Supports headings, bold, italic, code, strikethrough, links, lists,
/// code blocks, blockquotes, rules, and tables.
///
/// Plain text without Markdown syntax produces a single paragraph, preserving
/// backward compatibility.
pub fn text_to_adf(text: &str) -> Value {
    markdown_to_adf(text)
}

/// Returns true if an ADF node is a block-level element.
fn is_block_node(node: &Value) -> bool {
    matches!(
        node.get("type").and_then(|t| t.as_str()),
        Some(
            "paragraph"
                | "heading"
                | "bulletList"
                | "orderedList"
                | "codeBlock"
                | "blockquote"
                | "rule"
                | "table"
        )
    )
}

/// Groups consecutive inline children into paragraphs, leaving block-level nodes as-is.
/// Used for listItem and tableCell which require block-level children in ADF.
fn wrap_inline_in_paragraphs(children: Vec<Value>) -> Vec<Value> {
    let mut result = Vec::new();
    let mut inline_buf: Vec<Value> = Vec::new();

    for child in children {
        if is_block_node(&child) {
            if !inline_buf.is_empty() {
                result.push(json!({"type": "paragraph", "content": inline_buf}));
                inline_buf = Vec::new();
            }
            result.push(child);
        } else {
            inline_buf.push(child);
        }
    }

    if !inline_buf.is_empty() {
        result.push(json!({"type": "paragraph", "content": inline_buf}));
    }

    result
}

/// Converts Markdown text to an ADF (Atlassian Document Format) document.
///
/// Uses pulldown-cmark to parse Markdown and builds a corresponding ADF node tree.
/// GFM extensions (tables, strikethrough, tasklists) are enabled.
fn markdown_to_adf(text: &str) -> Value {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(text, opts);

    // Stack-based builder: each frame is (node_json, children_vec)
    // Top of stack is the current parent node being built.
    let mut stack: Vec<(Value, Vec<Value>)> = Vec::new();
    // Top-level content nodes (direct children of the doc)
    let mut doc_content: Vec<Value> = Vec::new();
    // Active inline marks (strong, em, code, strike, link)
    let mut marks: Vec<Value> = Vec::new();
    // Track if we're inside a table head for tableHeader vs tableCell
    let mut in_table_head = false;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    stack.push((json!({"type": "paragraph"}), Vec::new()));
                }
                Tag::Heading { level, .. } => {
                    stack.push((
                        json!({"type": "heading", "attrs": {"level": level as u8}}),
                        Vec::new(),
                    ));
                }
                Tag::BlockQuote(_) => {
                    stack.push((json!({"type": "blockquote"}), Vec::new()));
                }
                Tag::CodeBlock(kind) => {
                    let node = match kind {
                        CodeBlockKind::Fenced(lang) => {
                            let lang_str = lang.as_ref();
                            if lang_str.is_empty() {
                                json!({"type": "codeBlock"})
                            } else {
                                json!({"type": "codeBlock", "attrs": {"language": lang_str}})
                            }
                        }
                        CodeBlockKind::Indented => json!({"type": "codeBlock"}),
                    };
                    stack.push((node, Vec::new()));
                }
                Tag::List(first_item) => {
                    if first_item.is_some() {
                        stack.push((json!({"type": "orderedList"}), Vec::new()));
                    } else {
                        stack.push((json!({"type": "bulletList"}), Vec::new()));
                    }
                }
                Tag::Item => {
                    stack.push((json!({"type": "listItem"}), Vec::new()));
                }
                Tag::Table(_alignments) => {
                    stack.push((json!({"type": "table"}), Vec::new()));
                }
                Tag::TableHead => {
                    in_table_head = true;
                    stack.push((json!({"type": "tableRow"}), Vec::new()));
                }
                Tag::TableRow => {
                    stack.push((json!({"type": "tableRow"}), Vec::new()));
                }
                Tag::TableCell => {
                    let cell_type = if in_table_head {
                        "tableHeader"
                    } else {
                        "tableCell"
                    };
                    stack.push((json!({"type": cell_type}), Vec::new()));
                }
                Tag::Emphasis => {
                    marks.push(json!({"type": "em"}));
                }
                Tag::Strong => {
                    marks.push(json!({"type": "strong"}));
                }
                Tag::Strikethrough => {
                    marks.push(json!({"type": "strike"}));
                }
                Tag::Link { dest_url, .. } => {
                    marks.push(
                        json!({"type": "link", "attrs": {"href": dest_url.as_ref()}}),
                    );
                }
                Tag::Image { dest_url, .. } => {
                    // ADF doesn't have a direct inline image in text. Represent as link.
                    marks.push(
                        json!({"type": "link", "attrs": {"href": dest_url.as_ref()}}),
                    );
                }
                _ => {}
            },

            Event::End(tag_end) => match tag_end {
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::CodeBlock
                | TagEnd::List(_)
                | TagEnd::Item
                | TagEnd::Table
                | TagEnd::TableHead
                | TagEnd::TableRow
                | TagEnd::TableCell => {
                    if tag_end == TagEnd::TableHead {
                        in_table_head = false;
                    }
                    if let Some((mut node, children)) = stack.pop() {
                        // listItem and tableCell in ADF require block-level children.
                        // Tight lists in pulldown-cmark produce inline content directly
                        // under Item without a Paragraph wrapper — we add one.
                        let needs_block_wrap =
                            matches!(tag_end, TagEnd::Item | TagEnd::TableCell);
                        let final_children = if needs_block_wrap && !children.is_empty() {
                            wrap_inline_in_paragraphs(children)
                        } else {
                            children
                        };

                        if !final_children.is_empty() {
                            node["content"] = Value::Array(final_children);
                        }

                        // Push completed node to parent or doc_content
                        if let Some(parent) = stack.last_mut() {
                            parent.1.push(node);
                        } else {
                            doc_content.push(node);
                        }
                    }
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                    // Pop the corresponding mark
                    let mark_type = match tag_end {
                        TagEnd::Emphasis => "em",
                        TagEnd::Strong => "strong",
                        TagEnd::Strikethrough => "strike",
                        _ => unreachable!(),
                    };
                    if let Some(pos) = marks.iter().rposition(|m| {
                        m.get("type").and_then(|t| t.as_str()) == Some(mark_type)
                    }) {
                        marks.remove(pos);
                    }
                }
                TagEnd::Link | TagEnd::Image => {
                    // Pop the link mark
                    if let Some(pos) = marks.iter().rposition(|m| {
                        m.get("type").and_then(|t| t.as_str()) == Some("link")
                    }) {
                        marks.remove(pos);
                    }
                }
                _ => {}
            },

            Event::Text(text) => {
                let mut node = json!({"type": "text", "text": text.as_ref()});
                if !marks.is_empty() {
                    node["marks"] = Value::Array(marks.clone());
                }
                if let Some(parent) = stack.last_mut() {
                    parent.1.push(node);
                } else {
                    // Text outside any block — wrap in paragraph
                    doc_content.push(json!({
                        "type": "paragraph",
                        "content": [node]
                    }));
                }
            }

            Event::Code(code) => {
                let mut code_marks = marks.clone();
                code_marks.push(json!({"type": "code"}));
                let node = json!({
                    "type": "text",
                    "text": code.as_ref(),
                    "marks": code_marks
                });
                if let Some(parent) = stack.last_mut() {
                    parent.1.push(node);
                } else {
                    doc_content.push(json!({
                        "type": "paragraph",
                        "content": [node]
                    }));
                }
            }

            Event::SoftBreak => {
                // Treat soft breaks as a space in inline content
                let node = json!({"type": "text", "text": " "});
                if let Some(parent) = stack.last_mut() {
                    parent.1.push(node);
                }
            }

            Event::HardBreak => {
                let node = json!({"type": "hardBreak"});
                if let Some(parent) = stack.last_mut() {
                    parent.1.push(node);
                }
            }

            Event::Rule => {
                let node = json!({"type": "rule"});
                if let Some(parent) = stack.last_mut() {
                    parent.1.push(node);
                } else {
                    doc_content.push(node);
                }
            }

            Event::TaskListMarker(checked) => {
                // Represent as text prefix in the list item
                let marker = if checked { "[x] " } else { "[ ] " };
                let node = json!({"type": "text", "text": marker});
                if let Some(parent) = stack.last_mut() {
                    parent.1.push(node);
                }
            }

            // Ignore HTML, footnotes, math, metadata
            _ => {}
        }
    }

    // Build the ADF doc
    json!({
        "type": "doc",
        "version": 1,
        "content": doc_content
    })
}

/// Processes ADF input for any text field (description, comment, etc.)
///
/// This is the core processing function that handles conversion and validation
/// of various input types to ADF format. All field-specific functions delegate
/// to this function.
///
/// Handles three input types:
/// - String: Converts to simple paragraph ADF using text_to_adf
/// - Object: Validates as ADF and returns it (zero-copy via move semantics)
/// - Null: Returns empty paragraph ADF
///
/// # Arguments
/// * `value` - The input value to process (consumed)
/// * `field_name` - Name of the field for error messages (e.g., "description", "comment")
///
/// # Errors
/// Returns error if:
/// - Input is not string, object, or null (e.g., number, boolean, array)
/// - Object fails ADF validation
///
/// # Examples
/// ```
/// use serde_json::{json, Value};
/// use anyhow::Result;
///
/// fn process_adf_input(value: Value, field_name: &str) -> Result<Value> {
///     match value {
///         Value::String(text) => Ok(json!({
///             "type": "doc", "version": 1,
///             "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}]
///         })),
///         Value::Object(_) => Ok(value),
///         Value::Null => Ok(json!({"type": "doc", "version": 1, "content": []})),
///         _ => anyhow::bail!("{} must be string or ADF object", field_name)
///     }
/// }
///
/// let adf = process_adf_input(json!("Hello"), "description").unwrap();
/// assert_eq!(adf["type"], "doc");
/// ```
pub fn process_adf_input(value: Value, field_name: &str) -> Result<Value> {
    match value {
        Value::String(text) => {
            // Plain text: convert to simple ADF
            Ok(text_to_adf(&text))
        }
        Value::Object(_) => {
            // ADF object: validate and return (zero-copy via move)
            validate_adf(&value)?;
            Ok(value)
        }
        Value::Null => {
            // Null/missing: return empty paragraph ADF
            Ok(text_to_adf(""))
        }
        _ => {
            // Invalid type (number, boolean, array, etc.)
            anyhow::bail!(
                "{} must be string or ADF object, got {:?}",
                field_name,
                value
            )
        }
    }
}

/// Processes description input for create/update issue operations.
///
/// Convenience wrapper around process_adf_input with field name "description".
/// Consumes the input value for zero-copy processing.
///
/// # Errors
/// Returns error if input is invalid (see process_adf_input for details)
#[inline]
pub fn process_description_input(value: Value) -> Result<Value> {
    process_adf_input(value, "description")
}

/// Processes comment input for add/update comment operations.
///
/// Convenience wrapper around process_adf_input with field name "comment".
/// Consumes the input value for zero-copy processing.
///
/// # Errors
/// Returns error if input is invalid (see process_adf_input for details)
#[inline]
pub fn process_comment_input(value: Value) -> Result<Value> {
    process_adf_input(value, "comment")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests for validate_adf function

    #[test]
    fn test_validate_adf_valid_document() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": []
        });

        assert!(validate_adf(&adf).is_ok());
    }

    #[test]
    fn test_validate_adf_valid_document_with_content() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": "Hello"}]
            }]
        });

        assert!(validate_adf(&adf).is_ok());
    }

    #[test]
    fn test_validate_adf_missing_type() {
        let adf = json!({
            "version": 1,
            "content": []
        });

        let result = validate_adf(&adf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing required field 'type'")
        );
    }

    #[test]
    fn test_validate_adf_wrong_type() {
        let adf = json!({
            "type": "paragraph",
            "version": 1,
            "content": []
        });

        let result = validate_adf(&adf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("type must be 'doc'")
        );
    }

    #[test]
    fn test_validate_adf_wrong_version() {
        let adf = json!({
            "type": "doc",
            "version": 2,
            "content": []
        });

        let result = validate_adf(&adf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("version must be 1")
        );
    }

    #[test]
    fn test_validate_adf_missing_version() {
        let adf = json!({
            "type": "doc",
            "content": []
        });

        let result = validate_adf(&adf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing required field 'version'")
        );
    }

    #[test]
    fn test_validate_adf_invalid_content() {
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": "invalid"
        });

        let result = validate_adf(&adf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("content must be array")
        );
    }

    #[test]
    fn test_validate_adf_missing_content() {
        let adf = json!({
            "type": "doc",
            "version": 1
        });

        let result = validate_adf(&adf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing required field 'content'")
        );
    }

    #[test]
    fn test_validate_adf_not_object() {
        let adf = json!("not an object");

        let result = validate_adf(&adf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must be an object")
        );
    }

    // Tests for text_to_adf function

    #[test]
    fn test_text_to_adf_simple() {
        let text = "Hello, world!";
        let adf = text_to_adf(text);

        assert_eq!(adf["type"], "doc");
        assert_eq!(adf["version"], 1);
        assert_eq!(adf["content"][0]["type"], "paragraph");
        assert_eq!(adf["content"][0]["content"][0]["type"], "text");
        assert_eq!(adf["content"][0]["content"][0]["text"], "Hello, world!");
    }

    #[test]
    fn test_text_to_adf_empty() {
        let adf = text_to_adf("");

        assert_eq!(adf["type"], "doc");
        assert_eq!(adf["version"], 1);
        // Empty string produces empty content array
        assert_eq!(adf["content"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_text_to_adf_special_characters() {
        // Quotes and apostrophes are preserved; angle brackets are parsed as HTML by pulldown-cmark
        let text = "Test with \"quotes\" and 'apostrophes'";
        let adf = text_to_adf(text);

        assert_eq!(adf["content"][0]["content"][0]["text"], text);
    }

    #[test]
    fn test_text_to_adf_newlines() {
        // Newlines within a paragraph become soft breaks (spaces) in markdown
        let adf = text_to_adf("Line 1\nLine 2\nLine 3");

        assert_eq!(adf["content"][0]["type"], "paragraph");
        // Should have text nodes with space separators (soft breaks)
        let content = adf["content"][0]["content"].as_array().unwrap();
        assert!(content.len() >= 3); // "Line 1", " ", "Line 2", " ", "Line 3"
        assert_eq!(content[0]["text"], "Line 1");
    }

    #[test]
    fn test_text_to_adf_unicode() {
        let text = "Unicode: 你好 🎉 café";
        let adf = text_to_adf(text);

        assert_eq!(adf["content"][0]["content"][0]["text"], text);
    }

    // Tests for process_adf_input function (core logic)

    #[test]
    fn test_process_adf_input_string() {
        let input = json!("Plain text");
        let result = process_adf_input(input, "test_field").unwrap();

        assert_eq!(result["type"], "doc");
        assert_eq!(result["version"], 1);
        assert_eq!(result["content"][0]["content"][0]["text"], "Plain text");
    }

    #[test]
    fn test_process_adf_input_empty_string() {
        let input = json!("");
        let result = process_adf_input(input, "test_field").unwrap();

        assert_eq!(result["type"], "doc");
        // Empty string produces empty content
        assert_eq!(result["content"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_process_adf_input_valid_adf_object() {
        let input = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "heading",
                "attrs": {"level": 2},
                "content": [{"type": "text", "text": "Title"}]
            }]
        });

        let result = process_adf_input(input, "test_field").unwrap();
        assert_eq!(result["type"], "doc");
        assert_eq!(result["content"][0]["type"], "heading");
        assert_eq!(result["content"][0]["attrs"]["level"], 2);
    }

    #[test]
    fn test_process_adf_input_complex_adf() {
        let input = json!({
            "type": "doc",
            "version": 1,
            "content": [
                {
                    "type": "heading",
                    "attrs": {"level": 2},
                    "content": [{"type": "text", "text": "Problem"}]
                },
                {
                    "type": "paragraph",
                    "content": [
                        {"type": "text", "text": "The "},
                        {"type": "text", "text": "API", "marks": [{"type": "code"}]},
                        {"type": "text", "text": " is broken"}
                    ]
                },
                {
                    "type": "codeBlock",
                    "attrs": {"language": "rust"},
                    "content": [{"type": "text", "text": "fn main() {}"}]
                }
            ]
        });

        let result = process_adf_input(input, "test_field").unwrap();
        assert_eq!(result["type"], "doc");
        assert_eq!(result["content"].as_array().unwrap().len(), 3);
        assert_eq!(result["content"][0]["type"], "heading");
        assert_eq!(result["content"][1]["type"], "paragraph");
        assert_eq!(result["content"][2]["type"], "codeBlock");
    }

    #[test]
    fn test_process_adf_input_null() {
        let input = json!(null);
        let result = process_adf_input(input, "test_field").unwrap();

        assert_eq!(result["type"], "doc");
        // Null produces empty content (same as empty string)
        assert_eq!(result["content"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_process_adf_input_invalid_type_number() {
        let input = json!(123);
        let result = process_adf_input(input, "test_field");

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("test_field must be string or ADF object"));
    }

    #[test]
    fn test_process_adf_input_invalid_type_boolean() {
        let input = json!(true);
        let result = process_adf_input(input, "my_field");

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("my_field must be string or ADF object"));
    }

    #[test]
    fn test_process_adf_input_invalid_type_array() {
        let input = json!(["not", "an", "adf"]);
        let result = process_adf_input(input, "custom_field");

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("custom_field must be string or ADF object"));
    }

    #[test]
    fn test_process_adf_input_invalid_adf_object() {
        let input = json!({
            "type": "paragraph",  // Wrong type
            "version": 1,
            "content": []
        });

        let result = process_adf_input(input, "test_field");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("type must be 'doc'")
        );
    }

    #[test]
    fn test_process_adf_input_field_name_in_error() {
        let input = json!(123);
        let result = process_adf_input(input, "my_custom_field");

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("my_custom_field"),
            "Error message should include field name, got: {}",
            error_msg
        );
    }

    // Tests for wrapper functions

    #[test]
    fn test_process_description_input_delegates_correctly() {
        let input = json!("Test description");
        let result = process_description_input(input).unwrap();

        assert_eq!(result["type"], "doc");
        assert_eq!(
            result["content"][0]["content"][0]["text"],
            "Test description"
        );
    }

    #[test]
    fn test_process_description_input_error_includes_field_name() {
        let input = json!(123);
        let result = process_description_input(input);

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("description"),
            "Error message should include 'description', got: {}",
            error_msg
        );
    }

    #[test]
    fn test_process_comment_input_delegates_correctly() {
        let input = json!("Test comment");
        let result = process_comment_input(input).unwrap();

        assert_eq!(result["type"], "doc");
        assert_eq!(result["content"][0]["content"][0]["text"], "Test comment");
    }

    #[test]
    fn test_process_comment_input_error_includes_field_name() {
        let input = json!(true);
        let result = process_comment_input(input);

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("comment"),
            "Error message should include 'comment', got: {}",
            error_msg
        );
    }

    // Performance test

    #[test]
    fn test_adf_validation_performance() {
        use std::time::Instant;

        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": "test"}]
            }]
        });

        let start = Instant::now();
        for _ in 0..1000 {
            validate_adf(&adf).unwrap();
        }
        let duration = start.elapsed();

        let avg_ms = duration.as_micros() as f64 / 1000.0 / 1000.0;
        println!("Average validation time: {:.3}ms", avg_ms);

        // Should be < 10ms average (NFR3 requirement)
        assert!(avg_ms < 10.0, "Validation too slow: {}ms > 10ms", avg_ms);
    }

    // Edge case tests: version field type validation

    #[test]
    fn test_validate_adf_version_as_float() {
        // Test that version 1.0 (float) is rejected - must be integer
        let adf = json!({
            "type": "doc",
            "version": 1.0,
            "content": []
        });

        let result = validate_adf(&adf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing required field 'version'"),
            "Float version should be rejected (as_i64 fails)"
        );
    }

    #[test]
    fn test_validate_adf_version_as_string() {
        // Test that version "1" (string) is rejected
        let adf = json!({
            "type": "doc",
            "version": "1",
            "content": []
        });

        let result = validate_adf(&adf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing required field 'version'")
        );
    }

    #[test]
    fn test_validate_adf_version_zero() {
        // Test that version 0 is rejected
        let adf = json!({
            "type": "doc",
            "version": 0,
            "content": []
        });

        let result = validate_adf(&adf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("version must be 1")
        );
    }

    #[test]
    fn test_validate_adf_version_negative() {
        // Test that negative version is rejected
        let adf = json!({
            "type": "doc",
            "version": -1,
            "content": []
        });

        let result = validate_adf(&adf);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("version must be 1")
        );
    }

    // Edge case tests: large text handling

    #[test]
    fn test_text_to_adf_large_text() {
        // Test 50KB text conversion (typical large Jira description)
        let large_text = "x".repeat(50_000);
        let adf = text_to_adf(&large_text);

        assert_eq!(adf["type"], "doc");
        assert_eq!(adf["version"], 1);
        assert_eq!(adf["content"][0]["content"][0]["text"], large_text);
        assert_eq!(
            adf["content"][0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .len(),
            50_000
        );
    }

    #[test]
    fn test_process_adf_input_large_string() {
        // Test processing 100KB string doesn't panic
        let large_text = "Large description text. ".repeat(4_000); // ~100KB
        let input = json!(large_text);
        let result = process_adf_input(input, "description").unwrap();

        assert_eq!(result["type"], "doc");
        // Large text is parsed as a paragraph
        assert_eq!(result["content"][0]["type"], "paragraph");
    }

    #[test]
    fn test_text_to_adf_performance_large_text() {
        use std::time::Instant;

        // Test that 10KB text conversion with markdown parsing is fast (< 10ms)
        let text = "x".repeat(10_000);

        let start = Instant::now();
        for _ in 0..100 {
            text_to_adf(&text);
        }
        let duration = start.elapsed();

        let avg_ms = duration.as_micros() as f64 / 100.0 / 1000.0;
        println!("Average 10KB text conversion time: {:.3}ms", avg_ms);

        // Should be fast (< 10ms per conversion including markdown parsing)
        assert!(
            avg_ms < 10.0,
            "Text conversion too slow: {}ms > 10ms",
            avg_ms
        );
    }

    // Edge case tests: deeply nested structures

    #[test]
    fn test_validate_adf_deeply_nested_content() {
        // Test that deeply nested content passes validation
        // (we only validate top-level, so deep nesting is OK)
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{
                            "type": "text",
                            "text": "Nested",
                            "marks": [
                                {"type": "strong"},
                                {"type": "em"},
                                {"type": "code"}
                            ]
                        }]
                    }]
                }]
            }]
        });

        let result = validate_adf(&adf);
        assert!(result.is_ok(), "Deeply nested structure should be valid");
    }

    #[test]
    fn test_process_adf_input_complex_nested_lists() {
        // Test mixed list types with deep nesting
        let input = json!({
            "type": "doc",
            "version": 1,
            "content": [
                {
                    "type": "bulletList",
                    "content": [{
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [{"type": "text", "text": "Bullet 1"}]
                        }, {
                            "type": "orderedList",
                            "content": [{
                                "type": "listItem",
                                "content": [{
                                    "type": "paragraph",
                                    "content": [{"type": "text", "text": "Nested ordered"}]
                                }]
                            }]
                        }]
                    }]
                },
                {
                    "type": "orderedList",
                    "content": [{
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [{"type": "text", "text": "Ordered 1"}]
                        }]
                    }]
                }
            ]
        });

        let result = process_adf_input(input, "description").unwrap();
        assert_eq!(result["type"], "doc");
        assert_eq!(result["content"].as_array().unwrap().len(), 2);
        assert_eq!(result["content"][0]["type"], "bulletList");
        assert_eq!(result["content"][1]["type"], "orderedList");
    }

    // Edge case tests: multiple marks and formatting

    #[test]
    fn test_process_adf_input_multiple_text_marks() {
        // Test text with multiple marks (bold + italic + code)
        let input = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "Important API",
                    "marks": [
                        {"type": "strong"},
                        {"type": "em"},
                        {"type": "code"}
                    ]
                }]
            }]
        });

        let result = process_adf_input(input, "description").unwrap();
        assert_eq!(result["type"], "doc");
        assert_eq!(
            result["content"][0]["content"][0]["marks"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn test_validate_adf_mixed_content_types() {
        // Test document with all major block types
        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": [
                {
                    "type": "heading",
                    "attrs": {"level": 1},
                    "content": [{"type": "text", "text": "Title"}]
                },
                {
                    "type": "paragraph",
                    "content": [{"type": "text", "text": "Normal text"}]
                },
                {
                    "type": "bulletList",
                    "content": [{
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [{"type": "text", "text": "Item"}]
                        }]
                    }]
                },
                {
                    "type": "orderedList",
                    "content": [{
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [{"type": "text", "text": "Step 1"}]
                        }]
                    }]
                },
                {
                    "type": "codeBlock",
                    "attrs": {"language": "rust"},
                    "content": [{"type": "text", "text": "fn main() {}"}]
                },
                {
                    "type": "panel",
                    "attrs": {"panelType": "info"},
                    "content": [{
                        "type": "paragraph",
                        "content": [{"type": "text", "text": "Info panel"}]
                    }]
                }
            ]
        });

        let result = validate_adf(&adf);
        assert!(result.is_ok(), "Mixed content types should be valid");
    }

    // Edge case tests: empty and whitespace

    #[test]
    fn test_text_to_adf_only_whitespace() {
        let text = "   \n\t\r\n   ";
        let adf = text_to_adf(text);

        // Whitespace-only text produces empty content
        assert_eq!(adf["type"], "doc");
        assert_eq!(adf["content"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_process_adf_input_empty_content_array() {
        // Empty content array is valid
        let input = json!({
            "type": "doc",
            "version": 1,
            "content": []
        });

        let result = process_adf_input(input, "description").unwrap();
        assert_eq!(result["type"], "doc");
        assert_eq!(result["content"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_validate_adf_content_with_many_elements() {
        // Test document with many top-level elements (100+)
        let mut content = Vec::new();
        for i in 0..150 {
            content.push(json!({
                "type": "paragraph",
                "content": [{"type": "text", "text": format!("Paragraph {}", i)}]
            }));
        }

        let adf = json!({
            "type": "doc",
            "version": 1,
            "content": content
        });

        let result = validate_adf(&adf);
        assert!(
            result.is_ok(),
            "Document with 150 paragraphs should be valid"
        );
    }

    // ===== Markdown → ADF conversion tests =====

    #[test]
    fn test_markdown_heading_levels() {
        for level in 1..=6 {
            let md = format!("{} Heading {}", "#".repeat(level), level);
            let adf = text_to_adf(&md);
            let node = &adf["content"][0];
            assert_eq!(node["type"], "heading", "level {}", level);
            assert_eq!(node["attrs"]["level"], level as u64);
            assert_eq!(node["content"][0]["text"], format!("Heading {}", level));
        }
    }

    #[test]
    fn test_markdown_bold() {
        let adf = text_to_adf("**bold text**");
        let node = &adf["content"][0]["content"][0];
        assert_eq!(node["type"], "text");
        assert_eq!(node["text"], "bold text");
        assert_eq!(node["marks"][0]["type"], "strong");
    }

    #[test]
    fn test_markdown_italic() {
        let adf = text_to_adf("*italic text*");
        let node = &adf["content"][0]["content"][0];
        assert_eq!(node["type"], "text");
        assert_eq!(node["text"], "italic text");
        assert_eq!(node["marks"][0]["type"], "em");
    }

    #[test]
    fn test_markdown_inline_code() {
        let adf = text_to_adf("`inline code`");
        let node = &adf["content"][0]["content"][0];
        assert_eq!(node["type"], "text");
        assert_eq!(node["text"], "inline code");
        assert_eq!(node["marks"][0]["type"], "code");
    }

    #[test]
    fn test_markdown_strikethrough() {
        let adf = text_to_adf("~~deleted~~");
        let node = &adf["content"][0]["content"][0];
        assert_eq!(node["type"], "text");
        assert_eq!(node["text"], "deleted");
        assert_eq!(node["marks"][0]["type"], "strike");
    }

    #[test]
    fn test_markdown_link() {
        let adf = text_to_adf("[click here](https://example.com)");
        let node = &adf["content"][0]["content"][0];
        assert_eq!(node["type"], "text");
        assert_eq!(node["text"], "click here");
        assert_eq!(node["marks"][0]["type"], "link");
        assert_eq!(node["marks"][0]["attrs"]["href"], "https://example.com");
    }

    #[test]
    fn test_markdown_bold_italic_nested() {
        let adf = text_to_adf("***bold and italic***");
        let node = &adf["content"][0]["content"][0];
        assert_eq!(node["type"], "text");
        let marks: Vec<&str> = node["marks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["type"].as_str().unwrap())
            .collect();
        assert!(marks.contains(&"strong"));
        assert!(marks.contains(&"em"));
    }

    #[test]
    fn test_markdown_bullet_list() {
        let adf = text_to_adf("- item 1\n- item 2\n- item 3");
        let list = &adf["content"][0];
        assert_eq!(list["type"], "bulletList");
        let items = list["content"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["type"], "listItem");
        assert_eq!(items[0]["content"][0]["type"], "paragraph");
        assert_eq!(items[0]["content"][0]["content"][0]["text"], "item 1");
    }

    #[test]
    fn test_markdown_ordered_list() {
        let adf = text_to_adf("1. first\n2. second\n3. third");
        let list = &adf["content"][0];
        assert_eq!(list["type"], "orderedList");
        let items = list["content"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[2]["content"][0]["content"][0]["text"], "third");
    }

    #[test]
    fn test_markdown_nested_list() {
        let md = "- parent\n  - child\n  - child2\n- parent2";
        let adf = text_to_adf(md);
        let list = &adf["content"][0];
        assert_eq!(list["type"], "bulletList");
        // First item should contain a paragraph and a nested bulletList
        let first_item = &list["content"][0];
        assert_eq!(first_item["type"], "listItem");
        let item_content = first_item["content"].as_array().unwrap();
        assert!(item_content.len() >= 2); // paragraph + nested list
        assert_eq!(item_content[0]["type"], "paragraph");
        assert_eq!(item_content[1]["type"], "bulletList");
    }

    #[test]
    fn test_markdown_code_block() {
        let adf = text_to_adf("```rust\nfn main() {\n    println!(\"hello\");\n}\n```");
        let block = &adf["content"][0];
        assert_eq!(block["type"], "codeBlock");
        assert_eq!(block["attrs"]["language"], "rust");
        assert!(block["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("fn main()"));
    }

    #[test]
    fn test_markdown_code_block_no_language() {
        let adf = text_to_adf("```\nsome code\n```");
        let block = &adf["content"][0];
        assert_eq!(block["type"], "codeBlock");
        // No language attr
        assert!(block.get("attrs").is_none() || block["attrs"].is_null());
    }

    #[test]
    fn test_markdown_blockquote() {
        let adf = text_to_adf("> This is a quote");
        let block = &adf["content"][0];
        assert_eq!(block["type"], "blockquote");
        assert_eq!(block["content"][0]["type"], "paragraph");
        assert_eq!(block["content"][0]["content"][0]["text"], "This is a quote");
    }

    #[test]
    fn test_markdown_rule() {
        let adf = text_to_adf("---");
        let node = &adf["content"][0];
        assert_eq!(node["type"], "rule");
    }

    #[test]
    fn test_markdown_table() {
        let md = "| Header 1 | Header 2 |\n| --- | --- |\n| Cell 1 | Cell 2 |";
        let adf = text_to_adf(md);
        let table = &adf["content"][0];
        assert_eq!(table["type"], "table");
        let rows = table["content"].as_array().unwrap();
        assert_eq!(rows.len(), 2); // header row + data row

        // Header row
        let header_row = &rows[0];
        assert_eq!(header_row["type"], "tableRow");
        let headers = header_row["content"].as_array().unwrap();
        assert_eq!(headers[0]["type"], "tableHeader");
        assert_eq!(headers[1]["type"], "tableHeader");

        // Data row
        let data_row = &rows[1];
        assert_eq!(data_row["type"], "tableRow");
        let cells = data_row["content"].as_array().unwrap();
        assert_eq!(cells[0]["type"], "tableCell");
        assert_eq!(cells[1]["type"], "tableCell");
    }

    #[test]
    fn test_markdown_composite_document() {
        let md = "## Root Cause Analysis\n\nThe **API** returned `500`.\n\n- Check logs\n- Restart service\n\n```bash\ncurl -v https://api.example.com\n```";
        let adf = text_to_adf(md);
        let content = adf["content"].as_array().unwrap();

        assert_eq!(content[0]["type"], "heading");
        assert_eq!(content[0]["attrs"]["level"], 2);
        assert_eq!(content[1]["type"], "paragraph");
        assert_eq!(content[2]["type"], "bulletList");
        assert_eq!(content[3]["type"], "codeBlock");
        assert_eq!(content[3]["attrs"]["language"], "bash");
    }

    #[test]
    fn test_markdown_plain_text_paragraph() {
        // Plain text without any markdown should still produce a paragraph
        let adf = text_to_adf("Just plain text here");
        assert_eq!(adf["type"], "doc");
        assert_eq!(adf["version"], 1);
        assert_eq!(adf["content"][0]["type"], "paragraph");
        assert_eq!(
            adf["content"][0]["content"][0]["text"],
            "Just plain text here"
        );
    }

    #[test]
    fn test_markdown_multiple_paragraphs() {
        let adf = text_to_adf("First paragraph.\n\nSecond paragraph.");
        let content = adf["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "paragraph");
        assert_eq!(content[0]["content"][0]["text"], "First paragraph.");
        assert_eq!(content[1]["type"], "paragraph");
        assert_eq!(content[1]["content"][0]["text"], "Second paragraph.");
    }

    #[test]
    fn test_markdown_mixed_inline_formatting() {
        let adf = text_to_adf("Normal **bold** and *italic* and `code`");
        let para = &adf["content"][0];
        assert_eq!(para["type"], "paragraph");
        let nodes = para["content"].as_array().unwrap();
        // Should have multiple text nodes with different marks
        assert!(nodes.len() >= 5);
        // "Normal " - no marks
        assert!(nodes[0]["marks"].is_null());
        // "bold" - strong mark
        let bold_node = nodes.iter().find(|n| n["text"] == "bold").unwrap();
        assert_eq!(bold_node["marks"][0]["type"], "strong");
        // "italic" - em mark
        let italic_node = nodes.iter().find(|n| n["text"] == "italic").unwrap();
        assert_eq!(italic_node["marks"][0]["type"], "em");
        // "code" - code mark
        let code_node = nodes.iter().find(|n| n["text"] == "code").unwrap();
        assert!(code_node["marks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["type"] == "code"));
    }

    #[test]
    fn test_markdown_hard_break() {
        let adf = text_to_adf("Line 1  \nLine 2");
        let para = &adf["content"][0];
        let content = para["content"].as_array().unwrap();
        // Should have a hardBreak node between Line 1 and Line 2
        assert!(content.iter().any(|n| n["type"] == "hardBreak"));
    }

    #[test]
    fn test_markdown_blockquote_with_formatting() {
        let adf = text_to_adf("> **Important**: Check the *logs*");
        let bq = &adf["content"][0];
        assert_eq!(bq["type"], "blockquote");
        let para = &bq["content"][0];
        assert_eq!(para["type"], "paragraph");
        // Should contain bold and italic nodes
        let nodes = para["content"].as_array().unwrap();
        let bold = nodes.iter().find(|n| n["text"] == "Important").unwrap();
        assert_eq!(bold["marks"][0]["type"], "strong");
    }
}
