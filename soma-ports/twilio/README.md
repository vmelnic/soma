# soma-port-twilio

`soma-port-twilio` is a `cdylib` SOMA port that provides SMS, WhatsApp, and voice call communications via the Twilio REST API.

- Port ID: `soma.twilio`
- Kind: `Messaging`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `send_sms`: `to`, `body`
- `send_whatsapp`: `to`, `body`
- `make_call`: `to`, `twiml_url`
- `list_messages`: `limit`

## Configuration

| Env var | Description |
|---|---|
| `SOMA_TWILIO_ACCOUNT_SID` | Twilio account SID (primary) |
| `TWILIO_ACCOUNT_SID` | Twilio account SID (fallback) |
| `SOMA_TWILIO_AUTH_TOKEN` | Twilio auth token (primary) |
| `TWILIO_AUTH_TOKEN` | Twilio auth token (fallback) |
| `SOMA_TWILIO_FROM_NUMBER` | Sender phone number in E.164 format (primary) |
| `TWILIO_FROM_NUMBER` | Sender phone number in E.164 format (fallback) |

## Build

```bash
cargo build
cargo test
```
