---
name: jira-confluence
description: Execute Jira/Confluence queries via atlassian-cli. All commands use flat structure (e.g., `jira comments`, `jira comment-add`). Support JQL/CQL searches with ADF-to-Markdown conversion, create/update tickets and pages, manage comments and attachments, handle issue transitions.
allowed-tools: Bash
---

# atlassian-cli

## URL Handling

When user provides a URL instead of ID, extract the identifier:

**Jira URLs:**
- `https://domain/browse/PROJ-123` → Issue Key: `PROJ-123`
- `https://domain/jira/browse/PROJ-123` → Issue Key: `PROJ-123`

**Confluence URLs:**
- `https://domain/wiki/spaces/SPACE/pages/12345/Title` → Page ID: `12345`
- `https://domain/wiki/pages/viewpage.action?pageId=12345` → Page ID: `12345`

Extract the ID/key and use with the appropriate command.

## Jira

**Reading**: `--format markdown` converts ADF to Markdown (recommended)
**Writing**: Plain text auto-converts to ADF. For rich text, use ADF JSON.

### Command Aliases
| Canonical | Aliases |
|-----------|---------|
| `get` | `view`, `show` |
| `search` | `list`, `ls`, `find` |
| `transition` | `move`, `trans` |
| `transitions` | `statuses` |
| `comments` | `comment` |

### Flag Shortcuts
- `--transition-id <id>` (alias: `--to`): alternative to positional transition ID
- `-p/--project`, `--summary/--title`, `-t/--type`: alternative to positional args for `create`

### Commands
```bash
# Get issue
atlassian-cli jira get PROJ-123 --format markdown

# Search (JQL)
atlassian-cli jira search "assignee = currentUser()" --format markdown --limit 20
atlassian-cli jira search "project = PROJ" --fields key,summary,status --limit 50

# Pagination (large datasets)
atlassian-cli jira search "project = PROJ" --all --format markdown
atlassian-cli jira search "project = PROJ" --all --stream > issues.jsonl

# Create/Update
atlassian-cli jira create PROJ "Summary" Bug --description "Plain text"
atlassian-cli jira update PROJ-123 '{"summary": "New title", "description": "Plain text"}'

# Comments
atlassian-cli jira comments PROJ-123 --format markdown
atlassian-cli jira comment-add PROJ-123 "Comment text"
atlassian-cli jira comment-update PROJ-123 <comment_id> "Updated text"

# Attachments
atlassian-cli jira attachments PROJ-123
atlassian-cli jira attachment-download <attachment_id> -o ./output.png

## Viewing Images in Comments/Description

When `--format markdown` is used, media references include attachment IDs:
`[Media: screenshot.png (id:12345)]`

To analyze an image:
1. Note the attachment ID from `[Media: filename (id:xxx)]`
2. Download: `atlassian-cli jira attachment download <id> -o /tmp/filename
3. View the downloaded image using the Read tool

# Transitions
atlassian-cli jira transitions PROJ-123
atlassian-cli jira transition PROJ-123 31

# User Search (find people by name or email)
atlassian-cli jira user-search "john"
atlassian-cli jira user-search "john.doe@example.com" --limit 10
```

### ADF Format (for rich text)

Root: `{"version": 1, "type": "doc", "content": [...]}`

| Node | Example |
|------|---------|
| paragraph | `{"type": "paragraph", "content": [{"type": "text", "text": "..."}]}` |
| heading | `{"type": "heading", "attrs": {"level": 2}, "content": [...]}` |
| bulletList | `{"type": "bulletList", "content": [{"type": "listItem", "content": [...]}]}` |
| codeBlock | `{"type": "codeBlock", "attrs": {"language": "python"}, "content": [...]}` |

Marks: `{"type": "text", "text": "bold", "marks": [{"type": "strong"}]}`
- `strong`, `em`, `code`, `strike`, `link` (with `attrs.href`)

List hierarchy: `bulletList` → `listItem` → `paragraph` → `text`

### Jira Options

| Option | Description | Applies To |
|--------|-------------|------------|
| `--format markdown` | Convert ADF to Markdown | get, search, comment list |
| `--fields` | Specify fields to return | search |
| `--limit N` | Results per page (default: 100) | search, user-search |
| `--all` | Fetch all results via token pagination | search |
| `--stream` | Output JSONL (requires --all) | search |
| `-o, --output` | Output file path | attachment download |

### User Search Output
Returns users with `accountId`, `displayName`, `emailAddress`, and `active` status.
Use `accountId` for JQL queries like `assignee = "<accountId>"`.

## Confluence

**Reading**: `--format markdown` converts HTML to Markdown (recommended)
**Writing**: HTML storage format required (e.g., `<p>text</p>`)

### Commands
```bash
# Get page
atlassian-cli confluence get 12345 --format markdown

# Search (CQL) - metadata only (fast)
atlassian-cli confluence search "space = TEAM" --limit 20

# Search with content (body included by default)
atlassian-cli confluence search "title ~ 'API'" --format markdown --limit 10

# Pagination
atlassian-cli confluence search "space = TEAM" --all --format markdown
atlassian-cli confluence search "space = TEAM" --all --stream > pages.jsonl

# Create/Update (HTML format)
atlassian-cli confluence create SPACE "Title" "<p>Content</p>"
atlassian-cli confluence update 12345 "Title" "<p>Updated</p>"

# Children & Comments
atlassian-cli confluence children 12345
atlassian-cli confluence comments 12345 --format markdown
```

### Options
| Option | Description | Applies To |
|--------|-------------|------------|
| `--format markdown` | Convert to Markdown | search, get, comments |
| `--limit N` | Max results (default: 10, max: 50 with body) | search |
| `--all` | Fetch all pages via cursor pagination | search |
| `--stream` | Output JSONL (requires --all) | search |
| `--expand <fields>` | Additional fields: `ancestors`, `space` (body.storage included by default) | search |

Note: `children` does not support `--format` (v2 API limitation).

## Common Options (Both Jira & Confluence)

| Option | Jira | Confluence |
|--------|------|------------|
| `--all` | Token pagination | Cursor pagination |
| `--stream` | JSONL output (requires --all) | JSONL output (requires --all) |
| `--format markdown` | ADF → Markdown | HTML → Markdown |
| `--limit N` | Results per page (default: 100) | Results per page (default: 10) |

## Authentication

Priority: CLI flags > Environment > Project config > Global config

```bash
# Environment variables
export ATLASSIAN_DOMAIN=company.atlassian.net
export ATLASSIAN_EMAIL=user@example.com
export ATLASSIAN_API_TOKEN=token
```

## Auto-Injection Filter

```toml
# .atlassian.toml
[default.jira]
projects_filter = ["PROJ1", "PROJ2"]

[default.confluence]
spaces_filter = ["SPACE1"]
```

Effect: JQL becomes `project IN (PROJ1,PROJ2) AND (your_jql)`