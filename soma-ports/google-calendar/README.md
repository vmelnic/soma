# soma-port-google-calendar

`soma-port-google-calendar` is a `cdylib` SOMA port that manages calendar events via the Google Calendar API.

- Port ID: `soma.google.calendar`
- Kind: `Cloud`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `list_events`: `calendar_id`, `time_min`, `time_max`, `max_results`
- `create_event`: `calendar_id`, `summary`, `start`, `end`, `description`, `location`
- `get_event`: `calendar_id`, `event_id`
- `delete_event`: `calendar_id`, `event_id`

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
