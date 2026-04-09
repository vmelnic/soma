# soma-port-sdk

`soma-port-sdk` is the shared crate used by every external SOMA port pack. It defines the `Port` trait, the port metadata model, error types, and the structured call record returned to `soma-next`.

## What It Exports

- `Port`: the trait every port implementation must satisfy
- `PortSpec` and `PortCapabilitySpec`: the runtime-visible declaration of a port and its capabilities
- `PortCallRecord`: the structured result of a capability invocation
- `PortError`: validation, dependency, transport, external, timeout, and internal error variants
- Common enums used by the runtime contract: side effects, trust, risk, determinism, rollback, auth, sandbox, lifecycle, and port kind
- `prelude`: the usual imports needed when authoring a port crate

## Dynamic Port Contract

Each external port crate in this repository compiles as a `cdylib` and exports:

```rust
#[unsafe(no_mangle)]
pub extern "C" fn soma_port_init() -> *mut dyn Port
```

That is the current contract used by `soma-next` when it dynamically loads ports.

## Important ABI Note

The contract above uses a Rust trait object across a dynamic library boundary. That is the contract the runtime and all current ports share today, but it is not a hardened C ABI. Any future ABI hardening must be coordinated between this SDK, every port crate, and the `soma-next` loader at the same time.

## Authoring a Port

At minimum, a port crate needs to:

1. Implement `Port` for a concrete adapter type.
2. Return a complete `PortSpec` that describes capabilities, failure modes, auth, sandbox requirements, and exposure.
3. Export `soma_port_init()` so the runtime can instantiate the adapter.

## Build

```bash
cargo build
cargo test
```
