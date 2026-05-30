# soma-port-git

`soma-port-git` is a `cdylib` SOMA port for local Git repository operations.

- Port ID: `git`
- Kind: `Custom`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: not required

## Capabilities

- Read: `status`, `diff`, `log`, `blame`, `branch_list`, `changed_files`
- Write: `add`, `commit`, `init`

All capabilities accept a `cwd` parameter to specify the working directory.

## Build

```bash
cargo build
cargo test
```
