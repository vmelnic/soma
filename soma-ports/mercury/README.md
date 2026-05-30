# soma-port-mercury

`soma-port-mercury` is a `cdylib` SOMA port for Inception Labs Mercury, a diffusion LLM that generates tokens via parallel iterative denoising.

- Port ID: `mercury`
- Kind: `Custom`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `generate`: chat completion via diffusion
- `reason`: structured reasoning output

## Configuration

| Env var | Description |
|---|---|
| `SOMA_MERCURY_API_KEY` | Inception Labs API key (primary) |
| `INCEPTION_API_KEY` | Inception Labs API key (fallback) |

## Build

```bash
cargo build
cargo test
```
