# soma-project-github

GitHub API via SOMA MCP. Read issues, PRs, files, and branches from any public (or token-authenticated) repository.

## Setup

```bash
# Copy runtime binary and port (or rebuild from source)
cp ../soma-next/target/release/soma bin/soma
cp ../soma-ports/target/release/libsoma_port_github.dylib packs/github/

# On macOS, fix quarantine
xattr -d com.apple.quarantine bin/soma 2>/dev/null || true
codesign -fs - bin/soma 2>/dev/null || true
```

## Configuration

Copy `.env.example` to `.env` and optionally set a GitHub token for higher rate limits:

```bash
SOMA_GITHUB_TOKEN=ghp_xxxxxxxxxxxxxxxxxxxx
SOMA_GITHUB_OWNER=vmelnic
SOMA_GITHUB_REPO=soma
```

## Commands

```bash
# List available MCP tools
node mcp-client.mjs skills

# List branches
node mcp-client.mjs branches

# Read a file from the repo
node mcp-client.mjs read-file --path README.md

# List open issues
node mcp-client.mjs issues

# Get a specific issue
node mcp-client.mjs issue-get --number 1

# List open PRs
node mcp-client.mjs prs

# Get a specific PR
node mcp-client.mjs pr-get --number 1

# Run all read-only smoke tests
node mcp-client.mjs smoke
```

## Smoke test

The smoke test runs four read-only operations against the configured repo and verifies:

1. `repo.list_branches` returns at least one branch
2. `repo.read_file` on `README.md` decodes and contains "SOMA"
3. `issue.list` returns an array
4. `pr.list` returns an array
