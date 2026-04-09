# soma-port-smtp

`soma-port-smtp` is a `cdylib` SOMA messaging port that declares plain-text, HTML, and attachment email delivery over SMTP.

- Port ID: `soma.smtp`
- Kind: `Messaging`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Declared Capabilities

- `send_plain`: `to`, `subject`, `body`
- `send_html`: `to`, `subject`, `body`
- `send_attachment`: `to`, `subject`, `body`, `attachment_name`, `attachment_data`

## Intended Behavior

- The implementation uses `lettre` and is designed around SMTP relay delivery with STARTTLS.
- `send_attachment` expects `attachment_data` as base64 and attaches it as `application/octet-stream`.

## Current Limitation

- The current implementation never populates its SMTP host, port, credentials, sender address, or Tokio runtime.
- Because of that, the lifecycle state stays `Loaded`, not `Active`.
- Calls validate their payloads but fail at send time with `DependencyUnavailable`.

## Production Notes

- The declared interface is reasonable, but this crate is not complete until it has a real configuration and initialization path.
- Once wired, it will still need operational concerns outside the crate: retry policy, bounce handling, queueing, and delivery monitoring.

## Build

```bash
cargo build
cargo test
```
