# soma-port-glm

`soma-port-glm` is a `cdylib` SOMA port for ZhipuAI GLM chat completions.

- Port ID: `glm`
- Kind: `Custom`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `generate`: `messages`, `model`, `temperature`, `max_tokens`

Default model: `glm-5.1`. API base: `https://api.z.ai/api/paas/v4`.

## Configuration

| Env var | Description |
|---|---|
| `SOMA_GLM_API_KEY` | ZhipuAI API key (primary) |
| `GLM_API_KEY` | ZhipuAI API key (fallback) |

## Build

```bash
cargo build
cargo test
```
