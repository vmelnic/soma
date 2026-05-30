# soma-port-runner

`soma-port-runner` is a `cdylib` SOMA port for process execution and Node.js tooling.

- Port ID: `runner`
- Kind: `Custom`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: not required

## Capabilities

- `exec`: run an arbitrary shell command with optional `cwd`, `env`, `timeout_ms`
- `npm_install`: run `npm install` in a directory
- `npm_test`: run `npm test` in a directory
- `npm_run`: run an npm script by name
- `node_run`: run a Node.js file directly

## Build

```bash
cargo build
cargo test
```
