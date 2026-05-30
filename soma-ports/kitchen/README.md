# soma-port-kitchen

`soma-port-kitchen` is a `cdylib` SOMA port that simulates a 2D kitchen countertop covering all 10 Meta-World ML10 manipulation tasks.

- Port ID: `kitchen`
- Kind: `Custom`
- Trust level: `Trusted`
- Remote exposure: `false`
- Network access: not required

## Capabilities

- Environment: `reset`, `scan`
- Manipulation: `push_board`, `pick_jar`, `pick_knife`, `place_shelf`, `place_counter`
- Fixtures: `door_open`, `door_close`, `drawer_open`, `drawer_close`
- Controls: `button_press`, `peg_insert`, `window_open`, `window_close`

## Build

```bash
cargo build
cargo test
```
