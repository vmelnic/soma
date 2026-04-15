# soma-port-pdf

`soma-port-pdf` is a `cdylib` SOMA port that generates PDF documents via the `printpdf` crate.

- Port ID: `soma.pdf`
- Kind: `Document`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: not required

## Capabilities

- `create_document`: `title`, `content`, `output_path`
- `add_page`: `document_path`, `content`
- `text_to_pdf`: `text`, `output_path`

## Configuration

No environment variables required. Pure local logic using the built-in Helvetica font.

## Build

```bash
cargo build
cargo test
```
