# soma-port-deepgram

`soma-port-deepgram` is a `cdylib` SOMA port for Deepgram speech-to-text, text-to-speech, and text intelligence.

- Port ID: `deepgram`
- Kind: `HTTP`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `transcribe`: speech-to-text with diarization, smart formatting, summarization, topics, intents, sentiment, PII redaction
- `speak`: text-to-speech with Aura models
- `analyze_text`: text intelligence with sentiment, summary, topics, intents

## Configuration

| Env var | Description |
|---|---|
| `SOMA_DEEPGRAM_API_KEY` | Deepgram API key (required) |
| `SOMA_DEEPGRAM_BASE_URL` | API base URL override |

## Build

```bash
cargo build
cargo test
```
