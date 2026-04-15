# soma-port-slack

`soma-port-slack` is a `cdylib` SOMA port that provides messaging via the Slack Web API.

- Port ID: `soma.slack`
- Kind: `Messaging`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `send_message`: `channel`, `text`, `thread_ts`
- `list_channels`: `limit`
- `upload_file`: `channel`, `content`, `filename`, `title`
- `add_reaction`: `channel`, `timestamp`, `name`

## Configuration

| Env var | Description |
|---|---|
| `SOMA_SLACK_BOT_TOKEN` | Slack bot OAuth token (primary) |
| `SLACK_BOT_TOKEN` | Slack bot OAuth token (fallback) |

## Build

```bash
cargo build
cargo test
```
