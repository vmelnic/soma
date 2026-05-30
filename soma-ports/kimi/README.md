# soma-port-kimi

`soma-port-kimi` is a `cdylib` SOMA port for Moonshot AI Kimi chat completions.

- Port ID: `kimi`
- Kind: `Custom`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `generate`: `messages`, `model`, `temperature`, `max_tokens`

Default model: `moonshot-v1-auto`. API base: `https://api.moonshot.ai/v1`.

## Configuration

| Env var | Description |
|---|---|
| `SOMA_KIMI_API_KEY` | Moonshot AI API key (primary) |
| `KIMI_API_KEY` | Moonshot AI API key (fallback) |

## Build

```bash
cargo build
cargo test
```
