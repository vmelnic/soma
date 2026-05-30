# soma-port-gridworld

`soma-port-gridworld` is a `cdylib` SOMA port providing grid world environments for MiniGrid benchmark comparison. Includes BFS-based macro-skill navigation.

- Port ID: `gridworld`
- Kind: `Custom`
- Trust level: `Trusted`
- Remote exposure: `false`
- Network access: not required

## Capabilities

- Environment: `reset`
- Observation: `scan`
- Navigation: `go_to`, `go_to_key`, `go_to_door`, `go_to_goal`, `go_to_ball`, `go_to_box`
- Interaction: `pickup`, `drop`, `toggle`

## Environments

`reset` accepts an `env` parameter: `empty`, `doorkey`, `distshift`, `crossing`, `fourrooms`, `multiroom`, `lavagap`, `fetch`, or a custom layout.

## Build

```bash
cargo build
cargo test
```
