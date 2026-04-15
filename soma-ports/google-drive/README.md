# soma-port-google-drive

`soma-port-google-drive` is a `cdylib` SOMA port that manages files and folders via the Google Drive API.

- Port ID: `soma.google.drive`
- Kind: `Cloud`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `list_files`: `query`, `page_size`, `page_token`
- `get_file`: `file_id`
- `upload_file`: `name`, `content`, `mime_type`, `parent_id`
- `delete_file`: `file_id`
- `create_folder`: `name`, `parent_id`

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
