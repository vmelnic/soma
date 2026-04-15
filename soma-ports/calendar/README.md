# soma-port-calendar

`soma-port-calendar` is a `cdylib` SOMA port that manages local iCalendar (.ics) files on the filesystem.

- Port ID: `soma.calendar`
- Kind: `Utility`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: not required

## Capabilities

- `create_event`: `calendar`, `summary`, `start`, `end`, `description`, `location`
- `list_events`: `calendar`, `date_from`, `date_to`
- `delete_event`: `calendar`, `event_id`
- `list_calendars`: *(no parameters)*

## Configuration

| Env var | Description |
|---|---|
| `SOMA_CALENDAR_DIR` | Calendar storage directory (default: `~/.soma/calendars/`) |

## Build

```bash
cargo build
cargo test
```
