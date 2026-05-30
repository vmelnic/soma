# soma-port-youtube

`soma-port-youtube` is a `cdylib` SOMA port for YouTube video/audio downloading and metadata via yt-dlp.

- Port ID: `youtube`
- Kind: `Custom`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `get_info`: fetch video metadata
- `list_formats`: list available download formats
- `download_video`: download video in a specified format
- `download_audio`: download audio only

Requires `yt-dlp` on the system PATH.

## Configuration

| Env var | Description |
|---|---|
| `SOMA_YTDLP_OUTPUT_DIR` | Output directory for downloads |

## Build

```bash
cargo build
cargo test
```
