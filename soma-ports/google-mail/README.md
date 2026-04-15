# soma-port-google-mail

`soma-port-google-mail` is a `cdylib` SOMA port that sends and reads email via the Gmail API.

- Port ID: `soma.google.mail`
- Kind: `Cloud`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `send_email`: `to`, `subject`, `body`, `cc`, `bcc`
- `list_messages`: `query`, `max_results`, `page_token`
- `get_message`: `message_id`
- `list_labels`: *(no parameters)*

## Configuration

| Env var | Description |
|---|---|
| `SOMA_GOOGLE_ACCESS_TOKEN` | Google OAuth2 access token (primary) |
| `GOOGLE_ACCESS_TOKEN` | Google OAuth2 access token (fallback) |

## Build

```bash
cargo build
cargo test
```
