# soma-port-image

`soma-port-image` is a pure-Rust `cdylib` SOMA port for image transforms and lightweight image sanitation.

- Port ID: `soma.image`
- Kind: `Custom`
- Trust level: `Verified`
- Remote exposure: `true`
- State model: stateless, local processing

## Capabilities

- `thumbnail`: generate a thumbnail from base64 `data` with target `width` and `height`
- `resize`: resize to exact `width` and `height`
- `crop`: crop `data` by `x`, `y`, `w`, `h`
- `format_convert`: convert `data` to `png`, `jpeg`, or `webp`
- `exif_strip`: strip metadata by decoding and re-encoding the image

## Input and Output Conventions

- All image inputs are base64-encoded byte strings in the `data` field.
- `thumbnail`, `resize`, and `crop` always return PNG output.
- `format_convert` returns the requested output format.
- `exif_strip` preserves the guessed source format when possible and falls back to PNG if format detection fails.

## Limits and Behavior

- Maximum requested dimension is `16384` pixels.
- `resize` uses `Lanczos3` for quality. `thumbnail` uses the `image` crate thumbnail path for faster downsizing.
- `crop` rejects zero-sized regions and out-of-bounds rectangles.
- There are no system library dependencies; the crate uses the Rust `image` ecosystem only.

## Production Notes

- This port is appropriate for synchronous, in-process transforms on bounded image payloads.
- It does not stream, tile, or offload large image workloads. If you need large-batch or very large-image processing, put a queue or worker boundary in front of it.

## Build

```bash
cargo build
cargo test
```
