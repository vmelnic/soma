# soma-port-search

`soma-port-search` is a `cdylib` SOMA port for code search operations: text, file, and symbol lookup.

- Port ID: `search`
- Kind: `Custom`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: not required

## Capabilities

- `text_search`: grep for text patterns across files
- `file_search`: find files by name or glob pattern
- `symbol_search`: find function, class, const, and export definitions by name

All capabilities accept a `cwd` parameter.

## Build

```bash
cargo build
cargo test
```
