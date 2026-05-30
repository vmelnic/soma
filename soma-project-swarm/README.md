# soma-project-swarm

End-to-end proof of **horizontal gene transfer**: a routine evolved on one node
propagates over the real transport to peers and runs there — an instinct none of
them started with and no human wrote.

```bash
cd soma-project-swarm && cargo run --release
# exit 0 iff all phases PASS
```

## What it proves

Transfer goes over **real TCP** between in-process nodes. A `TcpRemoteExecutor`
client sends a `TransferRoutine` message; a `LocalDispatchHandler` listener — the
runtime's real receive path — registers the routine as
`RoutineOrigin::PeerTransferred`. Phases 2–3 use the production
`RemoteRoutineBroadcaster`, the same type the reactive monitor's breeding loop
calls when a mutant proves itself.

| Phase | Demonstrates |
|---|---|
| 1 — Wire | a routine sent over TCP lands in the peer's store as `PeerTransferred` |
| 2 — Gene flow | a mutated `stat` routine, evolved by the real mutation operator and validated on node A, is broadcast to node B, which runs it successfully |
| 3 — Swarm | one broadcast reaches two peers (B and C); both run the unauthored gene |

The evolved gene is produced by `soma-next`'s real `mutation::mutate` (origin
`Mutated`), so no human authored it. After transfer the peers see it as
`PeerTransferred` and execute it through the real reference port.

## How it relates to the runtime

- Send/receive: `soma-next/src/distributed/transport.rs` (`TcpRemoteExecutor`,
  `LocalDispatchHandler`, `start_listener_background`).
- Broadcast: `soma-next/src/runtime/remote.rs` (`RemoteRoutineBroadcaster`),
  triggered from the breeding loop in `soma-next/src/runtime/world_state.rs` when
  a proven mutant crosses the fitness floor.

The single-node evolution that produces such genes is proven in
`soma-project-evolution`. See `docs/evolutionary-soma.md` for the full thesis.
