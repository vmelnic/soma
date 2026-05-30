# soma-port-patch

`soma-port-patch` is a `cdylib` SOMA port for unified diff patch operations.

- Port ID: `patch`
- Kind: `Custom`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: not required

## Capabilities

- `apply_patch`: apply a unified diff to files
- `check_patch`: dry-run a patch to verify it applies cleanly
- `create_patch`: generate a unified diff between two file versions

## Build

```bash
cargo build
cargo test
```
