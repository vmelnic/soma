# soma-port-timer

`soma-port-timer` is an in-process `cdylib` SOMA port that tracks timeout and interval entries in memory.

- Port ID: `soma.timer`
- Kind: `Custom`
- Trust level: `BuiltIn`
- Remote exposure: `false`
- State model: in-memory only

## Capabilities

- `set_timeout`: create a one-shot timer with `label` and `delay_ms`
- `set_interval`: create an interval timer with `label` and `delay_ms`
- `cancel_timer`: cancel by `timer_id`
- `list_active`: list tracked timer entries and computed `remaining_ms`

## Runtime Behavior

- Timers are stored in a mutex-protected `HashMap` keyed by UUID.
- `delay_ms` must be positive.
- `list_active` computes remaining time relative to `Instant::now()`.
- `cancel_timer` returns `cancelled: false` when the timer does not exist.

## Important Caveat

- This port does not run a background scheduler and does not emit events when a timer reaches zero.
- It is a timer registry and state model, not a full execution engine.
- `set_interval` records interval metadata, but there is no worker loop that automatically advances or dispatches interval firings.

## Production Notes

- This crate is useful when another layer polls timer state and decides what to do next.
- If you need durable scheduling, wakeups, retries, or distributed coordination, build those outside this port or replace the implementation behind the same capability surface.

## Build

```bash
cargo build
cargo test
```
