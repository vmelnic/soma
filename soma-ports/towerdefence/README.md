# soma-port-towerdefence

`soma-port-towerdefence` is a `cdylib` SOMA port that simulates a grid-based tower defence game with three tower types (archer, cannon, mage) and four enemy types (basic, fast, tank, boss).

- Port ID: `towerdefence`
- Kind: `Custom`
- Trust level: `Trusted`
- Remote exposure: `false`
- Network access: not required

## Capabilities

- `reset`: create a game from layout and wave config
- `get_state`: return current game state
- `place_tower`: place a tower at grid coordinates
- `upgrade_tower`: upgrade an existing tower
- `sell_tower`: sell a tower for partial refund
- `start_wave`: begin the next wave
- `tick`: advance simulation by N ticks

## Build

```bash
cargo build
cargo test
```
