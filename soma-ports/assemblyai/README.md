# soma-port-assemblyai

`soma-port-assemblyai` is a `cdylib` SOMA port for AssemblyAI speech-to-text and audio intelligence.

- Port ID: `assemblyai`
- Kind: `HTTP`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- Audio: `upload`
- Transcription: `transcribe`, `get_transcript`, `list_transcripts`, `delete_transcript`
- Export: `get_sentences`, `get_paragraphs`, `get_subtitles`, `word_search`
- PII: `get_redacted_audio`

Transcription supports optional audio intelligence features: speaker labels, sentiment analysis, entity detection, topic classification, PII redaction, content safety, auto highlights.

## Configuration

| Env var | Description |
|---|---|
| `SOMA_ASSEMBLYAI_API_KEY` | AssemblyAI API key (required) |
| `SOMA_ASSEMBLYAI_BASE_URL` | API base URL override |

## Build

```bash
cargo build
cargo test
```
