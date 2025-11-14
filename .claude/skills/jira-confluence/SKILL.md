---
name: jira-confluence
version: 0.1.0
description: Atlassian Cloud CLI for Jira and Confluence. Use for JQL/CQL queries, creating/updating issues, managing pages, transitions, comments, bulk operations, or ADF formatting. Project-specific development skill with codebase integration. Triggers - Jira tickets, sprint planning, Confluence docs, issue tracking, wiki pages, CLI development.
allowed-tools: [Bash, Read, Grep, Glob]
---

# Jira & Confluence CLI Expert (Project Developer Skill)

Developer-focused skill for the `atlassian` CLI project. Includes CLI usage + codebase integration knowledge.

**Context**: This skill is project-level and has access to codebase information (see [CLAUDE.md](../../CLAUDE.md)).

## Project Integration

**Binary location**: `target/release/atlassian` (development) or `~/.local/bin/atlassian` (installed)

**Development workflow**:
```bash
# Build and test
cargo build --release
cargo test

# Install locally
./install.sh

# Test with local config
./target/release/atlassian config show
```

**Related files**:
- `CLAUDE.md` - Developer guide (architecture, patterns)
- `README.md` - User documentation
- `src/jira/api.rs` - Jira operations
- `src/confluence/api.rs` - Confluence operations
- `src/config.rs` - 4-tier configuration system

## Quick Start

**Check installation**:
```bash
atlassian --version
# or during development:
./target/release/atlassian --version
```

**Configuration check**:
```bash
atlassian config show
```

## Authentication (4-Tier Priority)

1. **CLI flags** (highest): `--domain company.atlassian.net --email user@example.com --token TOKEN`
2. **Environment**: `ATLASSIAN_DOMAIN`, `ATLASSIAN_EMAIL`, `ATLASSIAN_API_TOKEN`
3. **Project config**: `./.atlassian.toml`
4. **Global config**: `~/.config/atlassian-cli/config.toml`

**Domain format**: `company.atlassian.net` (NOT `https://company.atlassian.net`)

**Implementation**: See `config.rs:load()` for priority resolution logic.

## Jira Operations (8 Commands)

### Get Issue
```bash
atlassian jira get PROJ-123
# Returns: Direct issue object with fields
```

**Implementation**: `jira/api.rs:get_issue()` - GET `/rest/api/3/issue/{key}`

### Search Issues (JQL)
```bash
atlassian jira search "assignee = currentUser() AND status != Done" --limit 50
atlassian jira search "project = PROJ ORDER BY priority DESC" --fields key,summary,status

# Returns: {"items": [...], "total": N}
```

**Field optimization** (60-70% reduction):
- **Default 17 fields**: `jira/fields.rs:DEFAULT_SEARCH_FIELDS`
- Excludes: `description` (large), `id` (redundant), `renderedFields` (HTML)
- **Priority**: CLI `--fields` > `JIRA_SEARCH_DEFAULT_FIELDS` > defaults + `JIRA_SEARCH_CUSTOM_FIELDS`

**Implementation**: `jira/api.rs:search()` + `jira/fields.rs:resolve_search_fields()`

**Override fields**:
```bash
--fields key,summary,status                    # Highest priority
JIRA_SEARCH_DEFAULT_FIELDS=key,summary         # Replaces all defaults
JIRA_SEARCH_CUSTOM_FIELDS=customfield_10015    # Extends defaults
```

### Create Issue
```bash
atlassian jira create PROJ "Bug title" Bug --description "Plain text description"
atlassian jira create PROJ "Task" Task --description '{"type":"doc","version":1,"content":[...]}'

# Returns: {"key": "PROJ-123", "id": "12345"}
```

**ADF processing**: `jira/adf.rs:process_adf_input()` - Auto-converts plain text or validates JSON ADF.

**Implementation**: `jira/api.rs:create_issue()` - POST `/rest/api/3/issue`

### Update Issue
```bash
atlassian jira update PROJ-123 '{"summary": "Updated title"}'
atlassian jira update PROJ-123 '{"description": "New description"}'

# Returns: {} (empty = success)
```

**Implementation**: `jira/api.rs:update_issue()` - PUT `/rest/api/3/issue/{key}`

### Comments
```bash
# Add comment
atlassian jira comment add PROJ-123 "Comment text"

# Update comment (get ID from issue response)
comment_id=$(atlassian jira get PROJ-123 | jq -r '.fields.comment.comments[0].id')
atlassian jira comment update PROJ-123 "$comment_id" "Updated text"
```

**Implementation**:
- `jira/api.rs:add_comment()` - POST `/rest/api/3/issue/{key}/comment`
- `jira/api.rs:update_comment()` - PUT `/rest/api/3/issue/{key}/comment/{id}`

### Transitions
```bash
# List available transitions
atlassian jira transitions PROJ-123
# Returns: [{id, name, to: {name}}, ...]

# Execute transition
trans_id=$(atlassian jira transitions PROJ-123 | jq -r '.[] | select(.name=="In Progress").id')
atlassian jira transition PROJ-123 "$trans_id"
# Returns: {} (empty = success)
```

**Implementation**:
- `jira/api.rs:get_transitions()` - GET `/rest/api/3/issue/{key}/transitions`
- `jira/api.rs:transition_issue()` - POST `/rest/api/3/issue/{key}/transitions`

## Confluence Operations (6 Commands)

### Search Pages (CQL)
```bash
atlassian confluence search "title ~ 'Meeting Notes'" --limit 20
atlassian confluence search "space = TEAM AND created >= now()-7d"

# Returns: {"items": [...], "total": N}
```

**Implementation**: `confluence/api.rs:search()` - GET `/wiki/rest/api/content/search` (v1 API)

### Get Page
```bash
atlassian confluence get 12345

# Returns: Direct page object (v2 API format)
```

**Implementation**: `confluence/api.rs:get_page()` - GET `/wiki/api/v2/pages/{id}`

**Field filtering**: `confluence/fields.rs:apply_v2_filtering()`

### Page Children & Comments
```bash
atlassian confluence children 12345
atlassian confluence comments 12345

# Returns: {"items": [...]}
```

**Implementation**:
- `confluence/api.rs:get_page_children()` - GET `/wiki/api/v2/pages/{id}/children`
- `confluence/api.rs:get_comments()` - GET `/wiki/api/v2/pages/{id}/footer-comments`

### Create Page
```bash
atlassian confluence create SPACE "Page Title" "<p>HTML content with <strong>formatting</strong></p>"

# Returns: {"id": "...", "title": "..."}
```

**Important**:
- Use space KEY (e.g., "TEAM"), not ID (CLI auto-converts)
- Content format: HTML storage format (NOT Markdown)
- Space key conversion: `confluence/api.rs:resolve_space_id()`

**Implementation**: `confluence/api.rs:create_page()` - POST `/wiki/api/v2/pages`

### Update Page
```bash
atlassian confluence update 12345 "Updated Title" "<p>New content</p>"

# Returns: {"id": "...", "version": {...}}
```

**Version handling**: CLI auto-increments version (no manual version needed).

**Implementation**: `confluence/api.rs:update_page()` - PUT `/wiki/api/v2/pages/{id}`

## Advanced Patterns

### Bulk Operations
```bash
# Serial execution with error handling
for key in $(atlassian jira search "status=Open" --limit 100 | jq -r '.items[].key'); do
  atlassian jira comment add "$key" "Bulk comment" || echo "Failed: $key"
done

# Parallel execution (4 concurrent)
atlassian jira search "status=Open" | jq -r '.items[].key' | \
  xargs -P 4 -I {} atlassian jira comment add {} "Comment"
```

### Project/Space Auto-Injection
```toml
# .atlassian.toml or ~/.config/atlassian-cli/config.toml
[default.jira]
projects_filter = ["PROJ1", "PROJ2"]

[default.confluence]
spaces_filter = ["SPACE1"]
```

**Effect**: JQL becomes `project IN (PROJ1,PROJ2) AND (your_jql)`

**ORDER BY handling**: CLI correctly places ORDER BY outside parentheses.

**Implementation**: `jira/api.rs:search()` - JQL injection logic

### Multi-line Content
```bash
# From file
atlassian confluence create SPACE "Title" "$(cat page.html)"

# Heredoc
content="$(cat <<'EOF'
Line 1
Line 2 with "quotes"
EOF
)"
atlassian jira create PROJ "Title" Bug --description "$content"
```

### JSON Escaping
```bash
# Single quotes (no variable expansion)
atlassian jira update PROJ-123 '{"summary":"New title"}'

# With variables: double quotes + escape
title="Bug fix"
atlassian jira update PROJ-123 "{\"summary\":\"$title\"}"
```

### JQL/CQL Quote Escaping
```bash
atlassian jira search "summary ~ \"bug fix\""
```

## Response Structures

**Search responses** (standardized):
```json
{
  "items": [...],
  "total": N
}
```

**Single item** (get/create):
```json
{
  "key": "PROJ-123",
  "id": "12345",
  "fields": {...}
}
```

**Empty success** (update/transition): `{}`

**Implementation**: Response normalization in `jira/api.rs` and `confluence/api.rs`

## Error Handling

**Common errors**:
- `401 Unauthorized`: Check `ATLASSIAN_EMAIL` and `ATLASSIAN_API_TOKEN`
- `403 Forbidden`: Insufficient permissions (check `config.rs:projects_filter`)
- `404 Not Found`: Invalid issue key or page ID
- `400 Bad Request`: JQL/CQL syntax error or invalid fields

**Error handling**: `anyhow::Result` pattern used throughout codebase.

**Exit codes**: 0 = success, non-zero = error

**Debugging**:
```bash
# Verbose mode (stderr logs)
atlassian -v jira search "..."

# Error suppression
atlassian jira get PROJ-123 2>/dev/null || echo "Not found"

# Conditional execution
set -e  # Exit on error
```

## Config Commands (5 Utilities)

```bash
atlassian config init [--global]     # Create config file
atlassian config show                # Display current config (token masked)
atlassian config list                # List config locations + env vars
atlassian config path [--global]     # Print config file path
atlassian config edit [--global]     # Open config in $EDITOR
```

**Implementation**: `main.rs:ConfigSubcommand` + `config.rs`

## Output & Debugging

**Output streams**:
- **stdout**: Compact JSON (default). Use `--pretty` for formatted output.
- **stderr**: Logs and errors. Suppress with `2>/dev/null`.

**Verbose mode**: Use `-v` flag for detailed logs (stderr).

**jq pipeline**:
```bash
atlassian jira search "assignee=currentUser()" | jq -r '.items[].key'
```

## Performance Tuning

**Environment variables**:
```bash
REQUEST_TIMEOUT_MS=60000   # Increase for slow networks (default: 30000)
MAX_CONNECTIONS=200        # Increase for bulk operations (default: 100)
```

**Implementation**: `http.rs:create_client()` - reqwest configuration

## ADF (Atlassian Document Format)

**Plain text** (recommended):
```bash
atlassian jira create PROJ "Title" Bug --description "Plain text"
```

**JSON ADF** (advanced):
```bash
atlassian jira create PROJ "Title" Bug --description '{
  "type": "doc",
  "version": 1,
  "content": [
    {"type": "paragraph", "content": [{"type": "text", "text": "Rich text"}]}
  ]
}'
```

**ADF processing**:
- `jira/adf.rs:text_to_adf()` - Plain text → ADF conversion
- `jira/adf.rs:process_adf_input()` - Auto-conversion or validation
- `jira/adf.rs:validate_adf()` - Top-level validation only

**Validation rules** (top-level):
- `type` must be "doc"
- `version` must be 1
- `content` must be array

**Zero-copy pattern**:
```rust
// Extract value without cloning (jira/api.rs)
let description = args.get_mut("description")
    .map(|v| std::mem::replace(v, Value::Null))
    .unwrap_or(Value::Null);
```

## Development Patterns

### Adding New Jira Command

See `CLAUDE.md` for detailed instructions. Summary:

1. **main.rs**: Add to `JiraSubcommand` enum
2. **main.rs**: Add handler
3. **jira/api.rs**: Implement function
4. **Test**: Add test in `jira/api.rs`

### Modifying Field Filtering

1. **jira/fields.rs**: Update `DEFAULT_SEARCH_FIELDS`
2. **Test impact**: Check test fixtures
3. **Update docs**: README.md field count

### Testing

```bash
cargo test                  # All tests
cargo test jira::adf       # Module tests
cargo test -- --nocapture  # With output
```

**Test utilities**: `src/test_utils.rs` - Shared test helpers

## Practical Examples

### Daily Standup Report
```bash
# Get my issues updated today
atlassian jira search "assignee = currentUser() AND updated >= startOfDay()" \
  --fields key,summary,status | jq -r '.items[] | "\(.key): \(.fields.summary)"'
```

### Sprint Planning
```bash
# Create stories for new feature
atlassian jira create PROJ "User authentication" Story --description "Implement OAuth2"
atlassian jira create PROJ "API integration" Story --description "Connect to external API"
```

### Documentation Workflow
```bash
# Create meeting notes
atlassian confluence create TEAM "Meeting Notes $(date +%Y-%m-%d)" \
  "<h1>Attendees</h1><ul><li>Person 1</li></ul>"

# Update existing page
page_id=$(atlassian confluence search "title=ProjectSpec" | jq -r '.items[0].id')
atlassian confluence update "$page_id" "Project Spec v2" "$(cat spec.html)"
```

### Bulk Transition
```bash
# Move all open bugs to "In Progress"
trans_id=$(atlassian jira transitions PROJ-1 | jq -r '.[] | select(.name=="In Progress").id')

atlassian jira search "project=PROJ AND status=Open AND issuetype=Bug" | \
  jq -r '.items[].key' | \
  xargs -I {} atlassian jira transition {} "$trans_id"
```

## Architecture Quick Reference

**Data flow**:
```
Terminal → clap::Parser → CLI Command (main.rs)
  ↓
Load Config (config.rs) - 4-tier priority
  ↓
API Operation (jira/api.rs or confluence/api.rs)
  ↓
Field filtering (jira/fields.rs or confluence/fields.rs)
  ↓
HTTP Request (http.rs) - reqwest + rustls
  ↓
JSON Response → stdout
```

**Key patterns**:
- **No response filtering**: `filter.rs` is dead code
- **Field optimization**: 17 defaults for Jira search (60-70% reduction)
- **ADF auto-conversion**: Plain text → JSON ADF
- **Config caching**: `base_url` computed once

## Dependencies

| Crate | Purpose | Location |
|-------|---------|----------|
| clap | CLI parsing (derive API) | `main.rs` |
| tokio | Async runtime | Throughout |
| reqwest | HTTP client (rustls) | `http.rs` |
| serde_json | JSON serialization | Throughout |
| anyhow | Error handling | Throughout |
| toml | Config file parsing | `config.rs` |
| dirs | Platform-specific paths | `config.rs` |

## Resources

- [CLAUDE.md](../../CLAUDE.md) - Developer guide (architecture, patterns)
- [README.md](../../README.md) - User documentation
- [Jira REST API v3](https://developer.atlassian.com/cloud/jira/platform/rest/v3/)
- [Confluence REST API v2](https://developer.atlassian.com/cloud/confluence/rest/v2/)
- [ADF Specification](https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/)

## Testing This Skill

**Activation test**: "Show me my Jira issues"

**Expected**: Uses `atlassian jira search "assignee = currentUser()"`

**Codebase integration**: "Where is field optimization implemented?"

**Expected**: References `jira/fields.rs:DEFAULT_SEARCH_FIELDS`
