# soma-port-github

`soma-port-github` is a `cdylib` SOMA port that provides access to the GitHub REST API.

- Port ID: `soma.github`
- Kind: `Custom`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

### Issues

- `issue.create`: `owner`, `repo`, `title`, `body`, `labels`, `assignees`
- `issue.list`: `owner`, `repo`, `state`, `labels`, `assignee`, `per_page`
- `issue.get`: `owner`, `repo`, `issue_number`
- `issue.update`: `owner`, `repo`, `issue_number`, `title`, `body`, `state`, `labels`, `assignees`
- `issue.comment`: `owner`, `repo`, `issue_number`, `body`

### Pull Requests

- `pr.create`: `owner`, `repo`, `title`, `head`, `base`, `body`, `draft`
- `pr.list`: `owner`, `repo`, `state`, `head`, `base`, `per_page`
- `pr.get`: `owner`, `repo`, `pull_number`
- `pr.merge`: `owner`, `repo`, `pull_number`, `commit_title`, `commit_message`, `merge_method`

### Repository

- `repo.read_file`: `owner`, `repo`, `path`, `ref` — returns `decoded_content` (base64 decoded)
- `repo.list_branches`: `owner`, `repo`, `per_page`

## Configuration

| Env var | Description |
|---|---|
| `SOMA_GITHUB_TOKEN` | GitHub personal access token (primary) |
| `GITHUB_TOKEN` | GitHub personal access token (fallback) |

The token must have appropriate scopes for the operations you intend to use:
- `repo` — full repository access (PRs, file reads, branches)
- `issues` — issue read/write

## Build

```bash
cargo test
cargo build --release
```

## Example invocation

```json
{
  "owner": "octocat",
  "repo": "hello-world",
  "title": "Bug: login fails on Safari",
  "body": "Steps to reproduce...",
  "labels": ["bug", "p1"]
}
```
