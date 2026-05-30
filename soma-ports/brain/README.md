# soma-port-brain

`soma-port-brain` is a `cdylib` SOMA port that provides LLM-based reasoning for skill selection. Wraps OpenAI, Kimi, and GLM APIs behind a unified interface.

- Port ID: `brain`
- Kind: `Custom`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `reason`: send messages to the configured LLM and receive a structured response

## Configuration

| Env var | Description |
|---|---|
| `SOMA_BRAIN_PROVIDER` | Backend provider: `openai` (default), `kimi`, `glm` |
| `SOMA_BRAIN_API_URL` | API endpoint override |
| `SOMA_BRAIN_MODEL` | Model name override |
| `SOMA_BRAIN_API_KEY` | API key (primary, any provider) |
| `OPENAI_API_KEY` | OpenAI key (fallback when provider is `openai`) |
| `SOMA_KIMI_API_KEY` / `KIMI_API_KEY` | Kimi key (fallback when provider is `kimi`) |
| `SOMA_GLM_API_KEY` / `GLM_API_KEY` | GLM key (fallback when provider is `glm`) |

## Build

```bash
cargo build
cargo test
```
