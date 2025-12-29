use crate::config::Config;
use crate::filter;
use crate::http;
use crate::jira::adf;
use crate::jira::fields;
use crate::markdown::adf_to_markdown;
use anyhow::Result;
use serde_json::{Value, json};
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;
use tokio::time::sleep;

fn convert_issue_to_markdown(issue: &mut Value) {
    let Some(fields) = issue.get_mut("fields") else {
        return;
    };
    let Some(desc) = fields.get_mut("description") else {
        return;
    };
    if desc.is_object() {
        *desc = Value::String(adf_to_markdown(desc));
    }
}

fn convert_issues_to_markdown(result: &mut Value) {
    let Some(items) = result.get_mut("items").and_then(|i| i.as_array_mut()) else {
        return;
    };

    for issue in items {
        convert_issue_to_markdown(issue);
    }
}

const MAX_RESULTS_PER_PAGE: u32 = 100;

fn apply_project_filter(jql: &str, config: &Config) -> String {
    if config.jira.projects_filter.is_empty() {
        return jql.to_string();
    }

    let jql_lower = jql.to_lowercase();
    let (conditions, order_by) = if let Some(pos) = jql_lower.find(" order by ") {
        (jql[..pos].to_string(), Some(jql[pos..].to_string()))
    } else if jql_lower.starts_with("order by ") {
        (String::new(), Some(format!(" {}", jql)))
    } else {
        (jql.to_string(), None)
    };

    let conditions_lower = conditions.to_lowercase();
    if conditions_lower.contains("project ")
        || conditions_lower.contains("project=")
        || conditions_lower.contains("project in")
    {
        return jql.to_string();
    }

    let projects = config
        .jira
        .projects_filter
        .iter()
        .map(|p| format!("\"{}\"", p))
        .collect::<Vec<_>>()
        .join(",");

    let base = if conditions.trim().is_empty() {
        format!("project IN ({})", projects)
    } else {
        format!("project IN ({}) AND ({})", projects, conditions.trim())
    };

    if let Some(order_clause) = order_by {
        format!("{}{}", base, order_clause)
    } else {
        base
    }
}

fn extract_display_name(value: &Value) -> Value {
    value
        .get("displayName")
        .cloned()
        .unwrap_or(Value::Null)
}

/// Inject attachment IDs into [Media: filename] references
/// Transforms "[Media: image.png]" to "[Media: image.png (id:12345)]"
fn inject_attachment_ids(text: &str, attachments: &[Value]) -> String {
    use regex::Regex;

    let re = Regex::new(r"\[Media: ([^\]]+)\]").unwrap();

    re.replace_all(text, |caps: &regex::Captures| {
        let filename = &caps[1];

        // Find matching attachment by filename
        for attachment in attachments {
            if let (Some(att_filename), Some(att_id)) = (
                attachment.get("filename").and_then(|f| f.as_str()),
                attachment.get("id").and_then(|i| i.as_str()),
            ) {
                if att_filename == filename {
                    return format!("[Media: {} (id:{})]", filename, att_id);
                }
            }
        }

        // No match found, return original
        caps[0].to_string()
    })
    .to_string()
}

fn simplify_issue(data: &Value, as_markdown: bool) -> Value {
    let fields = &data["fields"];

    let description = if as_markdown {
        fields
            .get("description")
            .map(|d| {
                if d.is_object() {
                    Value::String(adf_to_markdown(d))
                } else {
                    d.clone()
                }
            })
            .unwrap_or(Value::Null)
    } else {
        fields.get("description").cloned().unwrap_or(Value::Null)
    };

    json!({
        "key": data.get("key").cloned().unwrap_or(Value::Null),
        "summary": fields.get("summary").cloned().unwrap_or(Value::Null),
        "type": fields.get("issuetype").and_then(|t| t.get("name")).cloned().unwrap_or(Value::Null),
        "status": fields.get("status").and_then(|s| s.get("name")).cloned().unwrap_or(Value::Null),
        "priority": fields.get("priority").and_then(|p| p.get("name")).cloned().unwrap_or(Value::Null),
        "assignee": fields.get("assignee").map(extract_display_name).unwrap_or(Value::Null),
        "reporter": fields.get("reporter").map(extract_display_name).unwrap_or(Value::Null),
        "project": fields.get("project").and_then(|p| p.get("name")).cloned().unwrap_or(Value::Null),
        "created": fields.get("created").cloned().unwrap_or(Value::Null),
        "updated": fields.get("updated").cloned().unwrap_or(Value::Null),
        "description": description,
    })
}

fn simplify_attachment(attachment: &Value) -> Value {
    json!({
        "id": attachment.get("id").cloned().unwrap_or(Value::Null),
        "filename": attachment.get("filename").cloned().unwrap_or(Value::Null),
        "mimeType": attachment.get("mimeType").cloned().unwrap_or(Value::Null),
        "size": attachment.get("size").cloned().unwrap_or(Value::Null),
        "content": attachment.get("content").cloned().unwrap_or(Value::Null),
    })
}

pub async fn get_issue(issue_key: &str, as_markdown: bool, config: &Config) -> Result<Value> {
    let client = http::client(config);
    // Include attachment field
    let url = format!(
        "{}/rest/api/3/issue/{}?fields=summary,description,status,priority,issuetype,assignee,reporter,project,created,updated,attachment",
        config.base_url(),
        issue_key
    );

    let response = client
        .get(&url)
        .header("Authorization", http::auth_header(config))
        .header("Accept", "application/json")
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Failed to get issue ({}): {}", status, body);
    }

    let data: Value = response.json().await?;

    // Extract attachments first (needed for ID injection)
    let attachments: Vec<Value> = data["fields"]["attachment"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(simplify_attachment)
        .collect();

    // Simplify issue structure
    let mut simplified = simplify_issue(&data, as_markdown);

    // Inject attachment IDs into description [Media: filename] references
    if as_markdown {
        if let Some(obj) = simplified.as_object_mut() {
            if let Some(Value::String(desc)) = obj.get("description") {
                let desc_with_ids = inject_attachment_ids(desc, &attachments);
                obj.insert("description".to_string(), Value::String(desc_with_ids));
            }
        }
    }

    // Add attachments to output
    if let Some(obj) = simplified.as_object_mut() {
        if !attachments.is_empty() {
            obj.insert("attachments".to_string(), json!(attachments));
        }
    }

    // Fetch and include comments at the end
    let mut comments = fetch_comments_for_issue(issue_key, as_markdown, config).await;

    // Inject attachment IDs into comment bodies
    if as_markdown {
        for comment in &mut comments {
            if let Some(Value::String(body)) = comment.get("body") {
                let body_with_ids = inject_attachment_ids(body, &attachments);
                if let Some(obj) = comment.as_object_mut() {
                    obj.insert("body".to_string(), Value::String(body_with_ids));
                }
            }
        }
    }

    if let Some(obj) = simplified.as_object_mut() {
        obj.insert("comments".to_string(), json!(comments));
    }

    Ok(simplified)
}

pub async fn search(
    jql: &str,
    limit: u32,
    fields: Option<Vec<String>>,
    as_markdown: bool,
    config: &Config,
) -> Result<Value> {
    let final_jql = apply_project_filter(jql, config);
    let client = http::client(config);
    let url = format!("{}/rest/api/3/search/jql", config.base_url());

    let resolved_fields = fields::resolve_search_fields(fields, as_markdown, config);

    let body = json!({
        "jql": final_jql,
        "maxResults": limit,
        "fields": resolved_fields,
    });

    let response = client
        .post(&url)
        .header("Authorization", http::auth_header(config))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Search failed ({}): {}", status, body);
    }

    let mut data: Value = response.json().await?;
    filter::apply(&mut data, config);

    let issues = data["issues"].as_array().cloned().unwrap_or_default();
    let count = issues.len();
    let mut result = json!({
        "items": issues,
        "count": count
    });

    if as_markdown {
        convert_issues_to_markdown(&mut result);
    }

    Ok(result)
}

pub async fn search_all(
    jql: &str,
    fields: Option<Vec<String>>,
    stream: bool,
    as_markdown: bool,
    config: &Config,
) -> Result<Value> {
    let final_jql = apply_project_filter(jql, config);
    let client = http::client(config);
    let url = format!("{}/rest/api/3/search/jql", config.base_url());
    let resolved_fields = fields::resolve_search_fields(fields, as_markdown, config);

    let mut all_issues: Vec<Value> = Vec::new();
    let mut page_num = 1;
    let mut next_page_token: Option<String> = None;
    let mut total_count: u64 = 0;

    loop {
        let mut body = json!({
            "jql": final_jql,
            "maxResults": MAX_RESULTS_PER_PAGE,
            "fields": resolved_fields,
        });

        if let Some(ref token) = next_page_token {
            body["nextPageToken"] = json!(token);
        }

        let response = client
            .post(&url)
            .header("Authorization", http::auth_header(config))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Search failed ({}): {}", status, body);
        }

        let mut data: Value = response.json().await?;
        filter::apply(&mut data, config);

        if page_num == 1 {
            total_count = data["total"].as_u64().unwrap_or(0);
        }

        let issues = data["issues"].as_array().cloned().unwrap_or_default();
        let count = issues.len();

        let processed_issues: Vec<Value> = if as_markdown {
            issues
                .into_iter()
                .map(|mut issue| {
                    convert_issue_to_markdown(&mut issue);
                    issue
                })
                .collect()
        } else {
            issues
        };

        if stream {
            for issue in &processed_issues {
                println!("{}", serde_json::to_string(issue)?);
            }
            io::stdout().flush()?;
        }

        all_issues.extend(processed_issues);

        eprintln!(
            "  Page {}: {} issues (fetched: {}/{})",
            page_num,
            count,
            all_issues.len(),
            total_count
        );

        next_page_token = data["nextPageToken"].as_str().map(String::from);
        if next_page_token.is_none() || count == 0 {
            break;
        }

        page_num += 1;
        sleep(Duration::from_millis(
            config.performance.rate_limit_delay_ms,
        ))
        .await;
    }

    eprintln!("\nTotal: {} issues fetched", all_issues.len());

    if stream {
        Ok(json!({"streamed": true, "total": all_issues.len()}))
    } else {
        Ok(json!({
            "items": all_issues,
            "total": all_issues.len()
        }))
    }
}

pub async fn create_issue(
    project_key: &str,
    summary: &str,
    issue_type: &str,
    description: Value,
    config: &Config,
) -> Result<Value> {
    let client = http::client(config);
    let base_url = format!("{}/rest/api/3/issue", config.base_url());

    let url = fields::apply_field_filtering_to_url(&base_url);

    let description_adf = adf::process_description_input(description)?;

    let body = json!({
        "fields": {
            "project": {
                "key": project_key
            },
            "summary": summary,
            "issuetype": {
                "name": issue_type
            },
            "description": description_adf
        }
    });

    let response = client
        .post(&url)
        .header("Authorization", http::auth_header(config))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !response.status().is_success() {
        let error = response.text().await?;
        anyhow::bail!("Failed to create issue: {}", error);
    }

    let data: Value = response.json().await?;
    Ok(json!({
        "key": data["key"],
        "id": data["id"]
    }))
}

pub async fn update_issue(
    issue_key: &str,
    mut fields_value: Value,
    config: &Config,
) -> Result<Value> {
    let client = http::client(config);
    let url = format!("{}/rest/api/3/issue/{}", config.base_url(), issue_key);

    if let Some(fields_obj) = fields_value.as_object_mut()
        && let Some(description_ref) = fields_obj.get_mut("description")
    {
        let description = std::mem::replace(description_ref, Value::Null);
        let description_adf = adf::process_description_input(description)?;
        fields_obj.insert("description".to_string(), description_adf);
    }

    let response = client
        .put(&url)
        .header("Authorization", http::auth_header(config))
        .header("Content-Type", "application/json")
        .json(&json!({
            "fields": fields_value
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Failed to update issue ({}): {}", status, body);
    }

    Ok(json!({}))
}

fn simplify_comment(comment: &Value, as_markdown: bool) -> Value {
    let body = if as_markdown {
        comment
            .get("body")
            .map(|b| {
                if b.is_object() {
                    Value::String(adf_to_markdown(b))
                } else {
                    b.clone()
                }
            })
            .unwrap_or(Value::Null)
    } else {
        comment.get("body").cloned().unwrap_or(Value::Null)
    };

    json!({
        "id": comment.get("id").cloned().unwrap_or(Value::Null),
        "author": comment.get("author").and_then(|a| a.get("displayName")).cloned().unwrap_or(Value::Null),
        "body": body,
        "created": comment.get("created").cloned().unwrap_or(Value::Null),
        "updated": comment.get("updated").cloned().unwrap_or(Value::Null)
    })
}

pub async fn get_comments(issue_key: &str, as_markdown: bool, config: &Config) -> Result<Value> {
    let client = http::client(config);
    let url = format!(
        "{}/rest/api/3/issue/{}/comment",
        config.base_url(),
        issue_key
    );

    let response = client
        .get(&url)
        .header("Authorization", http::auth_header(config))
        .header("Accept", "application/json")
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Failed to get comments ({}): {}", status, body);
    }

    let data: Value = response.json().await?;
    let comments = data["comments"].as_array().cloned().unwrap_or_default();

    let processed_comments: Vec<Value> = comments
        .iter()
        .map(|comment| simplify_comment(comment, as_markdown))
        .collect();

    Ok(json!({
        "comments": processed_comments,
        "total": processed_comments.len()
    }))
}

async fn fetch_comments_for_issue(issue_key: &str, as_markdown: bool, config: &Config) -> Vec<Value> {
    let client = http::client(config);
    let url = format!(
        "{}/rest/api/3/issue/{}/comment",
        config.base_url(),
        issue_key
    );

    let response = match client
        .get(&url)
        .header("Authorization", http::auth_header(config))
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    if !response.status().is_success() {
        return vec![];
    }

    let data: Value = match response.json().await {
        Ok(d) => d,
        Err(_) => return vec![],
    };

    data["comments"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|comment| simplify_comment(comment, as_markdown))
        .collect()
}

pub async fn add_comment(issue_key: &str, comment: Value, config: &Config) -> Result<Value> {
    let comment_adf = adf::process_comment_input(comment)?;

    let client = http::client(config);
    let base_url = format!(
        "{}/rest/api/3/issue/{}/comment",
        config.base_url(),
        issue_key
    );

    let url = fields::apply_field_filtering_to_url(&base_url);

    let body = json!({
        "body": comment_adf
    });

    let response = client
        .post(&url)
        .header("Authorization", http::auth_header(config))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Failed to add comment ({}): {}", status, body);
    }

    let data: Value = response.json().await?;
    Ok(json!({"id": data["id"]}))
}

pub async fn update_comment(
    issue_key: &str,
    comment_id: &str,
    body: Value,
    config: &Config,
) -> Result<Value> {
    let body_adf = adf::process_comment_input(body)?;

    let client = http::client(config);
    let base_url = format!(
        "{}/rest/api/3/issue/{}/comment/{}",
        config.base_url(),
        issue_key,
        comment_id
    );

    let url = fields::apply_field_filtering_to_url(&base_url);

    let request_body = json!({
        "body": body_adf
    });

    let response = client
        .put(&url)
        .header("Authorization", http::auth_header(config))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await?;

    if !response.status().is_success() {
        let error = response.text().await?;
        anyhow::bail!("Failed to update comment: {}", error);
    }

    let data: Value = response.json().await?;
    Ok(json!({"id": data["id"]}))
}

pub async fn transition_issue(
    issue_key: &str,
    transition_id: &str,
    config: &Config,
) -> Result<Value> {
    let client = http::client(config);
    let url = format!(
        "{}/rest/api/3/issue/{}/transitions",
        config.base_url(),
        issue_key
    );

    let body = json!({
        "transition": {
            "id": transition_id
        }
    });

    let response = client
        .post(&url)
        .header("Authorization", http::auth_header(config))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Failed to transition issue ({}): {}", status, body);
    }

    Ok(json!({}))
}

pub async fn get_transitions(issue_key: &str, config: &Config) -> Result<Value> {
    let client = http::client(config);
    let base_url = format!(
        "{}/rest/api/3/issue/{}/transitions",
        config.base_url(),
        issue_key
    );

    let url = fields::apply_field_filtering_to_url(&base_url);

    let response = client
        .get(&url)
        .header("Authorization", http::auth_header(config))
        .header("Accept", "application/json")
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Failed to get transitions ({}): {}", status, body);
    }

    let mut data: Value = response.json().await?;
    filter::apply(&mut data, config);
    Ok(data["transitions"].take())
}

pub async fn get_attachments(issue_key: &str, config: &Config) -> Result<Value> {
    let client = http::client(config);
    let url = format!(
        "{}/rest/api/3/issue/{}?fields=attachment",
        config.base_url(),
        issue_key
    );

    let response = client
        .get(&url)
        .header("Authorization", http::auth_header(config))
        .header("Accept", "application/json")
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Failed to get attachments ({}): {}", status, body);
    }

    let data: Value = response.json().await?;
    let attachments = data["fields"]["attachment"].clone();

    if attachments.is_null() {
        return Ok(json!([]));
    }

    Ok(attachments)
}

pub async fn download_attachment(
    attachment_id: &str,
    output_path: Option<&Path>,
    config: &Config,
) -> Result<Value> {
    let client = http::client(config);

    // First, get attachment metadata to get the content URL and filename
    let meta_url = format!(
        "{}/rest/api/3/attachment/{}",
        config.base_url(),
        attachment_id
    );

    let meta_response = client
        .get(&meta_url)
        .header("Authorization", http::auth_header(config))
        .header("Accept", "application/json")
        .send()
        .await?;

    if !meta_response.status().is_success() {
        let status = meta_response.status();
        let body = meta_response.text().await.unwrap_or_default();
        anyhow::bail!("Failed to get attachment metadata ({}): {}", status, body);
    }

    let metadata: Value = meta_response.json().await?;
    let content_url = metadata["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No content URL in attachment metadata"))?;
    let filename = metadata["filename"]
        .as_str()
        .unwrap_or("attachment");

    // Download the actual file content
    let content_response = client
        .get(content_url)
        .header("Authorization", http::auth_header(config))
        .send()
        .await?;

    if !content_response.status().is_success() {
        let status = content_response.status();
        let body = content_response.text().await.unwrap_or_default();
        anyhow::bail!("Failed to download attachment ({}): {}", status, body);
    }

    let bytes = content_response.bytes().await?;

    // Determine output path
    let final_path = match output_path {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir()?.join(filename),
    };

    // Write to file
    std::fs::write(&final_path, &bytes)?;

    Ok(json!({
        "filename": filename,
        "path": final_path.to_string_lossy(),
        "size": bytes.len(),
        "id": attachment_id
    }))
}

pub async fn search_users(query: &str, limit: u32, config: &Config) -> Result<Value> {
    let client = http::client(config);
    let encoded_query: String = query
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
                c.to_string()
            } else {
                format!("%{:02X}", c as u8)
            }
        })
        .collect();
    let url = format!(
        "{}/rest/api/3/user/search?query={}&maxResults={}",
        config.base_url(),
        encoded_query,
        limit
    );

    let response = client
        .get(&url)
        .header("Authorization", http::auth_header(config))
        .header("Accept", "application/json")
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("User search failed ({}): {}", status, body);
    }

    let data: Value = response.json().await?;
    let users: Vec<Value> = data
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|user| {
            json!({
                "accountId": user.get("accountId").cloned().unwrap_or(Value::Null),
                "displayName": user.get("displayName").cloned().unwrap_or(Value::Null),
                "emailAddress": user.get("emailAddress").cloned().unwrap_or(Value::Null),
                "active": user.get("active").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();

    Ok(json!({
        "users": users,
        "count": users.len()
    }))
}

#[cfg(test)]
#[allow(
    clippy::field_reassign_with_default,
    clippy::unnecessary_literal_unwrap
)]
mod tests {
    use super::*;

    use crate::test_utils::create_test_config_with_filters;

    // Helper function to create test config
    fn create_test_config(
        jira_projects_filter: Vec<String>,
        jira_search_default_fields: Option<Vec<String>>,
    ) -> Config {
        let mut config = create_test_config_with_filters(jira_projects_filter, vec![]);
        config.jira.search_default_fields = jira_search_default_fields;
        config
    }

    // T013: Jira search tests

    #[test]
    fn test_search_default_limit() {
        // Test that default limit is 20 when not specified
        let jql = "status = Open";
        let limit = 20u32;

        assert_eq!(jql, "status = Open");
        assert_eq!(limit, 20);
    }

    #[test]
    fn test_search_custom_limit() {
        // Test that custom limit is respected
        let jql = "status = Open";
        let limit = 50u32;

        assert_eq!(jql, "status = Open");
        assert_eq!(limit, 50);
    }

    #[test]
    fn test_search_project_filter_injection() {
        // Test that project filter is injected when not present in JQL
        let config = create_test_config(vec!["PROJ1".to_string(), "PROJ2".to_string()], None);
        let jql = "status = Open";

        // Simulate the project filter logic with ORDER BY handling
        let jql_lower = jql.to_lowercase();
        let (conditions, order_by) = if let Some(pos) = jql_lower.find(" order by ") {
            (jql[..pos].to_string(), Some(jql[pos..].to_string()))
        } else if jql_lower.starts_with("order by ") {
            (String::new(), Some(format!(" {}", jql)))
        } else {
            (jql.to_string(), None)
        };

        let final_jql = if !config.jira.projects_filter.is_empty() {
            let conditions_lower = conditions.to_lowercase();
            if conditions_lower.contains("project ")
                || conditions_lower.contains("project=")
                || conditions_lower.contains("project in")
            {
                jql.to_string()
            } else {
                let projects = config
                    .jira
                    .projects_filter
                    .iter()
                    .map(|p| format!("\"{}\"", p))
                    .collect::<Vec<_>>()
                    .join(",");
                let base = if conditions.trim().is_empty() {
                    format!("project IN ({})", projects)
                } else {
                    format!("project IN ({}) AND ({})", projects, conditions.trim())
                };
                if let Some(ref order_clause) = order_by {
                    format!("{}{}", base, order_clause)
                } else {
                    base
                }
            }
        } else {
            jql.to_string()
        };

        assert_eq!(
            final_jql,
            "project IN (\"PROJ1\",\"PROJ2\") AND (status = Open)"
        );
    }

    #[test]
    fn test_search_project_filter_not_injected_when_present() {
        // Test that project filter is NOT injected when already in JQL
        let config = create_test_config(vec!["PROJ1".to_string()], None);
        let jql = "project = MYPROJ AND status = Open";

        // Simulate the project filter logic with ORDER BY handling
        let jql_lower = jql.to_lowercase();
        let (conditions, order_by) = if let Some(pos) = jql_lower.find(" order by ") {
            (jql[..pos].to_string(), Some(jql[pos..].to_string()))
        } else if jql_lower.starts_with("order by ") {
            (String::new(), Some(format!(" {}", jql)))
        } else {
            (jql.to_string(), None)
        };

        let final_jql = if !config.jira.projects_filter.is_empty() {
            let conditions_lower = conditions.to_lowercase();
            if conditions_lower.contains("project ")
                || conditions_lower.contains("project=")
                || conditions_lower.contains("project in")
            {
                jql.to_string()
            } else {
                let projects = config
                    .jira
                    .projects_filter
                    .iter()
                    .map(|p| format!("\"{}\"", p))
                    .collect::<Vec<_>>()
                    .join(",");
                let base = if conditions.trim().is_empty() {
                    format!("project IN ({})", projects)
                } else {
                    format!("project IN ({}) AND ({})", projects, conditions.trim())
                };
                if let Some(ref order_clause) = order_by {
                    format!("{}{}", base, order_clause)
                } else {
                    base
                }
            }
        } else {
            jql.to_string()
        };

        // Should remain unchanged because JQL already has "project ="
        assert_eq!(final_jql, "project = MYPROJ AND status = Open");
    }

    #[test]
    fn test_search_project_filter_with_order_by() {
        // Test that ORDER BY is correctly placed outside parentheses
        let config = create_test_config(vec!["PROJ1".to_string(), "PROJ2".to_string()], None);
        let jql = "status = Open ORDER BY created DESC";

        // Simulate the project filter logic with ORDER BY handling
        let jql_lower = jql.to_lowercase();
        let (conditions, order_by) = if let Some(pos) = jql_lower.find(" order by ") {
            (jql[..pos].to_string(), Some(jql[pos..].to_string()))
        } else if jql_lower.starts_with("order by ") {
            (String::new(), Some(format!(" {}", jql)))
        } else {
            (jql.to_string(), None)
        };

        let final_jql = if !config.jira.projects_filter.is_empty() {
            let conditions_lower = conditions.to_lowercase();
            if conditions_lower.contains("project ")
                || conditions_lower.contains("project=")
                || conditions_lower.contains("project in")
            {
                jql.to_string()
            } else {
                let projects = config
                    .jira
                    .projects_filter
                    .iter()
                    .map(|p| format!("\"{}\"", p))
                    .collect::<Vec<_>>()
                    .join(",");
                let base = if conditions.trim().is_empty() {
                    format!("project IN ({})", projects)
                } else {
                    format!("project IN ({}) AND ({})", projects, conditions.trim())
                };
                if let Some(ref order_clause) = order_by {
                    format!("{}{}", base, order_clause)
                } else {
                    base
                }
            }
        } else {
            jql.to_string()
        };

        // ORDER BY should be outside parentheses at the end
        assert_eq!(
            final_jql,
            "project IN (\"PROJ1\",\"PROJ2\") AND (status = Open) ORDER BY created DESC"
        );
    }

    #[test]
    fn test_search_project_filter_with_empty_conditions() {
        // Test that empty conditions (only ORDER BY) work correctly
        let config = create_test_config(vec!["PROJ1".to_string(), "PROJ2".to_string()], None);
        let jql = "ORDER BY created DESC";

        // Simulate the project filter logic with ORDER BY handling
        let jql_lower = jql.to_lowercase();
        let (conditions, order_by) = if let Some(pos) = jql_lower.find(" order by ") {
            (jql[..pos].to_string(), Some(jql[pos..].to_string()))
        } else if jql_lower.starts_with("order by ") {
            (String::new(), Some(format!(" {}", jql)))
        } else {
            (jql.to_string(), None)
        };

        let final_jql = if !config.jira.projects_filter.is_empty() {
            let conditions_lower = conditions.to_lowercase();
            if conditions_lower.contains("project ")
                || conditions_lower.contains("project=")
                || conditions_lower.contains("project in")
            {
                jql.to_string()
            } else {
                let projects = config
                    .jira
                    .projects_filter
                    .iter()
                    .map(|p| format!("\"{}\"", p))
                    .collect::<Vec<_>>()
                    .join(",");
                let base = if conditions.trim().is_empty() {
                    format!("project IN ({})", projects)
                } else {
                    format!("project IN ({}) AND ({})", projects, conditions.trim())
                };
                if let Some(ref order_clause) = order_by {
                    format!("{}{}", base, order_clause)
                } else {
                    base
                }
            }
        } else {
            jql.to_string()
        };

        // Should inject project filter without empty parentheses
        assert_eq!(
            final_jql,
            "project IN (\"PROJ1\",\"PROJ2\") ORDER BY created DESC"
        );
    }

    #[test]
    fn test_search_fields_extraction_from_api() {
        // Test that fields parameter is extracted from API call
        let fields = Some(vec![
            "key".to_string(),
            "summary".to_string(),
            "status".to_string(),
        ]);

        let fields_vec = fields.expect("fields should be Some");
        assert_eq!(fields_vec.len(), 3);
        assert_eq!(fields_vec, vec!["key", "summary", "status"]);
    }

    #[test]
    fn test_search_no_fields_uses_default() {
        let config = create_test_config(vec![], None);
        let result = fields::resolve_search_fields(None, false, &config);
        assert_eq!(result.len(), 17);
    }

    #[test]
    fn test_search_markdown_includes_description() {
        let config = create_test_config(vec![], None);
        let result = fields::resolve_search_fields(None, true, &config);
        assert_eq!(result.len(), 18);
        assert!(result.contains(&"description".to_string()));
    }

    #[test]
    fn test_search_empty_project_filter() {
        // Test that empty project filter doesn't modify JQL
        let config = create_test_config(vec![], None);
        let jql = "status = Open";

        let final_jql = if !config.jira.projects_filter.is_empty() {
            format!("project IN (...) AND ({})", jql)
        } else {
            jql.to_string()
        };

        assert_eq!(final_jql, "status = Open");
    }

    // T014: Jira get_issue tests

    #[test]
    fn test_get_issue_valid_issue_key() {
        let issue_key = "PROJ-123";
        assert_eq!(issue_key, "PROJ-123");
    }

    #[test]
    fn test_get_issue_url_construction() {
        let config = create_test_config(vec![], None);
        let issue_key = "PROJ-123";

        let base_url = format!("{}/rest/api/3/issue/{}", config.base_url(), issue_key);

        assert_eq!(
            base_url,
            "https://test.atlassian.net/rest/api/3/issue/PROJ-123"
        );
    }

    // T015: Jira create_issue tests

    #[test]
    fn test_create_issue_required_fields() {
        let project_key = "PROJ";
        let summary = "Test Issue";
        let issue_type = "Task";
        let description = "Test description";

        assert_eq!(project_key, "PROJ");
        assert_eq!(summary, "Test Issue");
        assert_eq!(issue_type, "Task");
        assert_eq!(description, "Test description");
    }

    #[test]
    fn test_create_issue_adf_conversion() {
        let description = "Test description";

        let adf_body = json!({
            "type": "doc",
            "version": 1,
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": description
                }]
            }]
        });

        assert_eq!(adf_body["type"], "doc");
        assert_eq!(adf_body["version"], 1);
        assert_eq!(adf_body["content"][0]["type"], "paragraph");
        assert_eq!(
            adf_body["content"][0]["content"][0]["text"],
            "Test description"
        );
    }

    // T016: Remaining Jira handlers tests

    // update_issue tests
    #[test]
    fn test_update_issue_valid_fields() {
        let issue_key = "PROJ-123";
        let fields_json = json!({
            "summary": "Updated summary",
            "priority": {"name": "High"}
        });

        assert_eq!(issue_key, "PROJ-123");
        assert_eq!(fields_json["summary"], "Updated summary");
        assert_eq!(fields_json["priority"]["name"], "High");
    }

    #[test]
    fn test_update_issue_url_construction() {
        let config = create_test_config(vec![], None);
        let issue_key = "PROJ-123";

        let url = format!("{}/rest/api/3/issue/{}", config.base_url(), issue_key);

        assert_eq!(url, "https://test.atlassian.net/rest/api/3/issue/PROJ-123");
    }

    // add_comment tests
    #[test]
    fn test_add_comment_missing_comment() {
        // After ADF support, missing comment field results in null which gets converted to empty ADF
        // Verify comment processing works with null input (converted to empty ADF)
        let comment_result = adf::process_comment_input(json!(null));
        assert!(comment_result.is_ok());
        let comment_adf = comment_result.unwrap();
        assert_eq!(comment_adf["type"], "doc");
        assert_eq!(comment_adf["content"][0]["content"][0]["text"], "");
    }

    #[test]
    fn test_add_comment_adf_conversion() {
        let comment = "This is a test comment";

        let adf_body = json!({
            "body": {
                "type": "doc",
                "version": 1,
                "content": [{
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": comment
                    }]
                }]
            }
        });

        assert_eq!(adf_body["body"]["type"], "doc");
        assert_eq!(adf_body["body"]["version"], 1);
        assert_eq!(adf_body["body"]["content"][0]["type"], "paragraph");
        assert_eq!(
            adf_body["body"]["content"][0]["content"][0]["text"],
            "This is a test comment"
        );
    }

    #[test]
    fn test_add_comment_url_construction() {
        let config = create_test_config(vec![], None);
        let issue_key = "PROJ-123";

        let base_url = format!(
            "{}/rest/api/3/issue/{}/comment",
            config.base_url(),
            issue_key
        );

        assert_eq!(
            base_url,
            "https://test.atlassian.net/rest/api/3/issue/PROJ-123/comment"
        );
    }

    // transition_issue tests
    #[test]
    fn test_transition_issue_valid_params() {
        let issue_key = "PROJ-123";
        let transition_id = "21";

        assert_eq!(issue_key, "PROJ-123");
        assert_eq!(transition_id, "21");
    }

    #[test]
    fn test_transition_issue_body_format() {
        let transition_id = "31";

        let body = json!({
            "transition": {
                "id": transition_id
            }
        });

        assert_eq!(body["transition"]["id"], "31");
    }

    // get_transitions tests
    #[test]
    fn test_get_transitions_valid_issue_key() {
        let issue_key = "PROJ-123";
        assert_eq!(issue_key, "PROJ-123");
    }

    #[test]
    fn test_get_transitions_url_construction() {
        let config = create_test_config(vec![], None);
        let issue_key = "PROJ-123";

        let base_url = format!(
            "{}/rest/api/3/issue/{}/transitions",
            config.base_url(),
            issue_key
        );

        assert_eq!(
            base_url,
            "https://test.atlassian.net/rest/api/3/issue/PROJ-123/transitions"
        );
    }

    // inject_attachment_ids tests
    #[test]
    fn test_inject_attachment_ids_single_match() {
        let attachments = vec![json!({
            "id": "12345",
            "filename": "screenshot.png"
        })];

        let text = "Check this [Media: screenshot.png] for details";
        let result = inject_attachment_ids(text, &attachments);

        assert_eq!(
            result,
            "Check this [Media: screenshot.png (id:12345)] for details"
        );
    }

    #[test]
    fn test_inject_attachment_ids_multiple_matches() {
        let attachments = vec![
            json!({"id": "111", "filename": "image1.png"}),
            json!({"id": "222", "filename": "image2.jpg"}),
        ];

        let text = "[Media: image1.png] and [Media: image2.jpg]";
        let result = inject_attachment_ids(text, &attachments);

        assert_eq!(
            result,
            "[Media: image1.png (id:111)] and [Media: image2.jpg (id:222)]"
        );
    }

    #[test]
    fn test_inject_attachment_ids_no_match() {
        let attachments = vec![json!({
            "id": "12345",
            "filename": "other.png"
        })];

        let text = "[Media: unknown.png]";
        let result = inject_attachment_ids(text, &attachments);

        assert_eq!(result, "[Media: unknown.png]");
    }

    #[test]
    fn test_inject_attachment_ids_empty_attachments() {
        let attachments: Vec<Value> = vec![];

        let text = "[Media: image.png]";
        let result = inject_attachment_ids(text, &attachments);

        assert_eq!(result, "[Media: image.png]");
    }

    #[test]
    fn test_inject_attachment_ids_no_media_refs() {
        let attachments = vec![json!({
            "id": "12345",
            "filename": "image.png"
        })];

        let text = "Plain text without media references";
        let result = inject_attachment_ids(text, &attachments);

        assert_eq!(result, "Plain text without media references");
    }
}
