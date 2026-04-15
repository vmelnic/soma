# soma-project-distributed

End-to-end test of SOMA's full distributed body — 3 instances cooperating over TCP.

## What it proves

| Feature | Test |
|---|---|
| Multi-instance boot | 3 instances (coordinator, scanner, processor) on ports 9900-9902 |
| Routine authoring | Coordinator authors routines via `author_routine` |
| Routine transfer | Coordinator transfers routine to scanner via `transfer_routine` |
| Routine replication | Coordinator replicates routine to all peers via `replicate_routine` |
| Remote skill invocation | Coordinator invokes skill on scanner via `invoke_remote_skill` |
| Routine versioning | Author v0, re-author v1, list versions, rollback to v0 |
| Sub-routine composition | Parent routine on coordinator calls sub-routine |
| Branching | Routine with Goto branching on success |
| Priority + exclusive | Two conflicting routines, higher priority wins |
| Belief sync | Coordinator syncs beliefs with scanner via `sync_beliefs` |
| Session migration | Coordinator migrates session to scanner via `migrate_session` |
| Heartbeat detection | Kill processor → coordinator detects peer offline |
| Failure alerting | World state emits `routine.*.last_failure` facts |
| 33 MCP tools | All tools listed |

## Usage

```bash
cd soma-next && cargo build --release
cd ../soma-project-distributed && ./run.sh
```

## Architecture

```
Coordinator (port 9900)  ──TCP──  Scanner (port 9901)
       │                              │
       └──────────TCP─────────  Processor (port 9902)
```

All 3 use the reference pack (filesystem port: stat, readdir, readfile, writefile, mkdir, touch).
Each is a full soma-next MCP server with heartbeat, reactive monitor, and world state persistence.
