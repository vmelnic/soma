# SOMA Project — Claude Code Instructions

## What This Is

SOMA (Greek "soma" = body) — the runtime IS the program. Two production paths:

- **LLM-driven**: LLM calls `invoke_port` via MCP → SOMA executes against external systems. SOMA is the body, LLM is the brain. Proven with HelperBook (3 ports, 32 capabilities, frontend).
- **Autonomous**: SOMA receives goals → selects skills → invokes ports → learns from episodes → compiles routines → plan-following mode for known patterns. Proven with reference pack (filesystem skills, episode→schema→routine cycle).

Active deliverables:
- **soma-next/** — Rust runtime. 1177 tests, zero warnings. Cross-compiles to `aarch64-linux-android` (10MB ELF) and `aarch64-apple-ios` (9MB Mach-O) with no code changes after the rustls/reqwest fix.
- **soma-ports/** — 11 dynamically loaded port adapters + SDK.
- **soma-helperbook/** — Service marketplace app (postgres + redis + auth, Express frontend).
- **soma-project-smtp/** — Email delivery proof.
- **soma-project-s3/** — AWS S3 proof.
- **soma-project-postgres/** — PostgreSQL proof.
- **soma-project-llm/** — Ollama + SOMA proof. LLM generates SQL from natural language, SOMA executes via postgres port.
- **soma-project-mcp/** — Claude Code MCP integration. SOMA as MCP server for Claude (.mcp.json at repo root).
- **soma-project-s2s/** — SOMA-to-SOMA communication proof. Two instances cooperate over TCP: transport, delegation, schema/routine transfer. 42 tests across 3 levels.
- **soma-project-multistep/** — End-to-end proof of multi-step autonomous routine learning. Episodes → schema → routine → real `SessionController` plan-following walks 3 skills against `/tmp` and reaches `Completed`. See "Multi-step routines: PROVEN" below.
- **soma-project-android/** — POC doc for native Android app (Kotlin + JNI to libsoma_android.so). Rust cross-compilation to aarch64-linux-android verified.
- **soma-project-ios/** — POC doc for native iOS app (Swift + C FFI to libsoma_ios.a). Rust cross-compilation to aarch64-apple-ios verified.
- **soma-project-esp32/** — ESP32 leaf nodes. **DUAL-CHIP PROVEN on real hardware: ESP32-S3 (Sunton 1732S019) and ESP32 LX6 (WROOM-32D), both with and without wifi — 14/14 tests without wifi, 16/16 with wifi including real `wifi.scan` returning live APs. Runtime pin configuration + SSD1306 OLED display both proven over MCP against the WROOM-32D.** 14-crate cargo workspace: `leaf` wire protocol library, 12 hardware port crates (gpio, delay, uart, i2c, spi, adc, pwm, wifi, storage, thermistor, board, display), and a `firmware` binary that composes ports via cargo features.
  - **Chip selection via cargo feature**: `chip-esp32s3` (default) or `chip-esp32`, mutually exclusive, enforced by `compile_error!` in `firmware/src/chip/mod.rs`. Per-chip cargo config overlays under `firmware/chips/<chip>.toml` pin the target triple + espflash runner. Each chip has its own pin-map file (`firmware/src/chip/<chip>.rs`) implementing a uniform interface (`NAME`, `TEST_LED_PIN`, `init_peripherals`, `register_all_ports`). main.rs and port crates are chip-agnostic — adding a new chip is dropping ONE file.
  - **Wifi proven on both chips** via `./scripts/cycle.sh <chip> wifi`: real `wifi.scan` returns live APs, `wifi.status` works, smoltcp+DHCP socket polling active, TCP listener on port 9100 ready. Heap sized at **96 KB** (64 KB was too small — produced a garbage-looking "0 bytes failed" OOM panic that looked like a zero-byte alloc bug but was actually uninitialized stack in the alloc error handler).
  - **Vendored + patched esp-alloc 0.6** at `vendor/esp-alloc/` with zero-byte guard, wired via `[patch.crates-io]` in workspace root. Defensive measure, NOT the actual wifi fix (heap size was).
  - **Helper scripts under `scripts/`** — single source of truth for build/flash/test. `setup.sh` installs toolchain. `boards.sh` probes USB serial and suggests chip. `build.sh <chip> [wifi]`, `flash.sh <chip>`, `monitor.sh <chip>`, `test.sh <chip> "" [wifi]` (Python wire protocol exerciser, 14 tests + 2 wifi), `cycle.sh <chip> [wifi]` (build+flash+test). Per-machine serial ports in `scripts/devices.env`. chip→target/features/config mapping in `scripts/lib.sh`.
  - **Workspace profile override REQUIRED**: `[profile.release.package.esp-storage] opt-level = 3` — ESP32 LX6 esp-storage flash write loops are timing-sensitive and its build script refuses `opt-level = "s"`. Do not remove.
  - **Known ESP32 LX6 wifi quirk**: `wifi.scan` can crash with an illegal-instruction exception if called AFTER heavy SPI flash writes in the same boot cycle. Workaround baked in: `scripts/wire-test.py --wifi` runs wifi tests FIRST, before storage. ESP32-S3 is unaffected. Real esp-wifi 0.12 bug, not fixed here.
  - Real esp-hal 0.23, esp-wifi 0.12, esp-storage 0.4, smoltcp 0.12, xtensa-lx 0.10, xtensa-lx-rt 0.18. ESP-IDF app descriptor placed at the start of drom_seg via a custom `rwtext.x` linker fragment in `firmware/build.rs` (without it, the stage-2 bootloader rejects the image with a garbage "efuse blk rev" error).
  - Wire protocol extension (option B): soma-next added `ListCapabilities`, `RemoveRoutine` request variants and `Capabilities`, `RoutineStored`, `RoutineRemoved` response variants — backward compatible, all 1177 soma-next tests still pass.
  - **Runtime pin configuration**: `board` port exposes `chip_info`, `pin_map`, `configure_pin`, `probe_i2c_buses`, `reboot`. Pin assignments for every peripheral (i2c sda/scl, spi sck/mosi, adc, pwm, uart tx/rx, gpio test) are loaded from FlashKvStore at boot with `DEFAULT_*` constants as fallbacks. Changing a pin is one MCP call + a reboot — no reflash needed. ADC uses a typed `match` over valid ADC1-capable GPIOs because `esp-hal`'s `AdcChannel` trait is only implemented for concrete `GpioPin<N>`; everything else dispatches via `AnyPin::steal(n)`. Proven cycle: `board.probe_i2c_buses [[5,4],[21,22]]` → found OLED at 0x3C → `board.configure_pin` → `board.reboot` → new pin map loaded on next boot.
  - **Display port (SSD1306 OLED)**: ships with the firmware by default. Skills: `display.info`, `display.clear`, `display.draw_text {line, column?, text, invert?}`, `display.draw_text_xy {x, y, text}`, `display.fill_rect`, `display.set_contrast`, `display.flush`. Uses `ssd1306 0.10` + `embedded-graphics 0.8` + `embedded-hal-bus 0.3` (RefCellDevice) to share the I²C0 bus with the `i2c` port — both consumers get their own `RefCellDevice` handle into a leaked `&'static RefCell<I2c>`. The port crate (`ports/display/`) has NO esp-hal / ssd1306 deps; the firmware injects seven type-erased closures that capture the real driver. **PROVEN ON PHYSICAL HARDWARE (WROOM-32D)**: `scripts/thermistor-to-display.py` drives a 5-second-period sensor-to-OLED update loop from brain-side Python over direct TCP. Text is visible on the real OLED panel: "Temperature: 22.00 C" updating every tick, plus ancillary lines showing tick number and label. The MCP path works the same way (`invoke_remote_skill thermistor.read_temp` → `invoke_remote_skill display.draw_text`) — an LLM driving soma-next produces identical behavior with no firmware changes. Cleanest demonstration of the brain/body split in the codebase: leaf has no concept of "every 5 seconds" (brain cadence) or "read sensor, show on screen" (brain composition), yet the panel shows the sensor reading.

Legacy (in repo but not active): soma-core/, soma-plugins/, soma-synthesizer/, poc/, pow/

## Two Execution Paths

### LLM-driven (HelperBook pattern)
```
LLM → invoke_port("postgres", "query", {sql: "..."}) → SOMA → PostgreSQL → result → LLM
```
- LLM decides what to do (writes SQL, chooses operations)
- SOMA executes, returns PortCallRecord with tracing
- `dump_state` gives LLM complete runtime context in one call (~5KB)
- No skills needed, no goals, no episodes — direct port invocation

### Autonomous (Reference pack pattern)
```
create_goal("list files in /tmp") → selector picks readdir → filesystem port → observation → episode
```
- Control loop: goal → belief → select skill → execute via port → observe → critic → repeat
- Episodes stored with embeddings (HashEmbedder, 128-dim FNV-1a feature hash)
- PrefixSpan extracts frequent skill subsequences → schemas induced
- High-confidence schemas → routines compiled
- Plan-following mode: routine found → skip deliberation → walk compiled_skill_path
- Memory ring buffer (1024 episodes), consolidation evicts after schema extraction

## Memory System

Brain-like three-tier consolidation:
```
Episodes (ring buffer 1024) → PrefixSpan → Schemas → compile → Routines → plan-following
```

- **HashEmbedder** (memory/embedder.rs): FNV-1a feature hashing, 128-dim, deterministic, works on ESP32
- **PrefixSpan** (memory/sequence_mining.rs): frequent subsequence mining, min_support threshold
- **Schema induction** (memory/schemas.rs): cluster episodes by embedding similarity (cosine 0.8), run PrefixSpan per cluster
- **Routine compilation** (memory/routines.rs): high-confidence schema → fixed skill path
- **Plan-following** (runtime/session.rs): when routine matches, working_memory.active_plan set, control loop walks the plan without fresh selection each step

## Repository Structure

```
soma/
  soma-next/                  # Rust runtime
    src/
      main.rs                 # Entry point, CLI, MCP, REPL
      bootstrap.rs            # Runtime assembly, GoalEmbedder wiring
      config.rs               # TOML config, env var overrides (SOMA_*)
      errors.rs               # SomaError enum
      runtime/
        session.rs            # 16-step control loop + plan-following mode
        policy.rs             # Safety policy enforcement
        port.rs               # Port trait, DefaultPortRuntime, invoke pipeline
        pack.rs, skill.rs, selector.rs, belief.rs, critic.rs,
        predictor.rs, resource.rs, goal.rs, trace.rs, metrics.rs,
        proprioception.rs, dynamic_port.rs, port_verify.rs
      adapters.rs             # SkillRegistryAdapter (routine/schema-aware), EpisodeMemoryAdapter (embedding-aware), PolicyEngineAdapter, PortBackedSkillExecutor
      interfaces/
        cli.rs                # 11 commands, build_episode_from_session, attempt_learning
        mcp.rs                # 19 MCP tools (16 core + list_peers, invoke_remote_skill, transfer_routine), episode storage after create_goal
      memory/
        episodes.rs           # Ring buffer (VecDeque, 1024 cap), retrieve_by_embedding
        schemas.rs            # PrefixSpan-based induction with embedding clustering
        routines.rs           # Compilation from schemas, invalidation
        embedder.rs           # GoalEmbedder trait + HashEmbedder (FNV-1a)
        sequence_mining.rs    # PrefixSpan algorithm
        persistence.rs, checkpoint.rs, working.rs, world.rs
      distributed/            # TCP/TLS, WebSocket, Unix socket, delegation, sync
      types/                  # Domain model (session.rs has active_plan/plan_step in WorkingMemory)
      ports/                  # Built-in: filesystem, http
    packs/reference/          # 7 filesystem skills for autonomous testing

  soma-ports/                 # Port workspace
    sdk/                      # soma-port-sdk: Port trait, types
    auth/, crypto/, geo/, image/, postgres/, push/, redis/, s3/, smtp/, timer/

  soma-helperbook/            # Service marketplace app
    docker-compose.yml        # PostgreSQL 17, Redis 7, Mailcatcher
    schema.sql, seed.sql      # 19 tables, test data
    frontend/                 # Express + plain JS + Tailwind
    packs/                    # postgres, redis, auth manifests + .dylibs
    scripts/                  # setup-db, seed-db, clean-db, start, start-mcp, dump-state, show-memory
    capabilities-checklist/   # run.mjs (44 tests), persistence.mjs (3-process test)

  soma-project-smtp/          # Email delivery proof
  soma-project-s3/            # AWS S3 proof
  soma-project-postgres/      # PostgreSQL proof
  soma-project-llm/           # Ollama LLM + SOMA proof (ollama.js CLI, docker-compose)
  soma-project-mcp/           # Claude Code MCP integration (SOMA as MCP server)
  soma-project-s2s/           # SOMA-to-SOMA proof (transport, delegation, transfer)
  .mcp.json                   # Root MCP config — registers SOMA for Claude Code
  docs/                       # 7 docs: vision, architecture, mcp, ports, distributed, building-projects, helperbook
```

## Build and Test

```bash
# Runtime
cd soma-next
cargo build --release        # ~10MB binary
cargo test                   # 1177+ tests, must all pass
cargo clippy                 # Must be zero warnings

# Ports
cd soma-ports
cargo build --workspace --release
cargo build --release --manifest-path redis/Cargo.toml  # redis built separately

# HelperBook
cd soma-helperbook
docker compose up -d --wait
scripts/setup-db.sh && scripts/seed-db.sh
node capabilities-checklist/run.mjs          # 44 runtime capability tests
node capabilities-checklist/persistence.mjs  # memory persistence across restarts

# Autonomous goal test
cd soma-next
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_goal","arguments":{"objective":"list files in /tmp"}}}' \
| cargo run --release -- --mcp --pack packs/reference/manifest.json
```

## Key Design Decisions

- **soma-port-sdk dependency**: soma-next depends on SDK for correct vtable. SdkPortAdapter bridges via JSON serialization.
- **Library naming**: `libsoma_port_{port_id}.dylib` — manifest port_id determines filename.
- **SOMA_PORTS_PLUGIN_PATH**: colon-separated. Applied even without soma.toml (env override fix).
- **observable_fields**: must be empty `[]` or only output schema fields. Port-level, checked per invocation.
- **output_schema**: use `{"schema": {"description": "any"}}` for ports returning non-objects (redis, crypto).
- **Policy**: read-only skills skip rule evaluation. Destructive/irreversible require confirmation or host override.
- **Episode ring buffer**: VecDeque with capacity 1024. Evicts oldest, returns to caller for consolidation.
- **Plan-following**: WorkingMemory.active_plan + plan_step. Set from matching routine's compiled_skill_path. Critic advances/clears plan.
- **MCP episode storage**: create_goal in MCP handler stores episodes + triggers learning (was missing, added).
- **MCP protocol compliance**: tools/list must return `inputSchema` (camelCase, not snake_case). tools/call results must be wrapped in `{"content": [{"type": "text", "text": "..."}]}`. Both were bugs, both fixed.
- **macOS binary copy**: copied binaries may need `xattr -d com.apple.quarantine` + `codesign -fs -` to run.
- **Failure recovery spec**: BindingFailure → SwitchCandidate, not Continue. The architecture.md failure recovery table is the source of truth for critic behavior.
- **Predictor calibration**: SimpleCandidatePredictor must penalize skills that fail repeatedly within a session. Score decay prevents infinite retry loops.
- **MCP distributed tools**: `list_peers`, `invoke_remote_skill`, `transfer_routine` (3 tools added for s2s). MCP mode now supports `--listen`, `--peer`, `--unix-listen`, `--unix-peer` flags.
- **MCP mode + listeners**: MCP mode bootstraps a separate listener runtime for incoming connections. The MCP server and listener are independent runtimes sharing no state.
- **LocalDispatchHandler stores**: `with_stores()` constructor wires `SchemaStore` and `RoutineStore` so transferred schemas/routines are actually persisted. Without stores, transfers are silently accepted (backward-compatible).
- **Precondition format**: `{ condition_type: String, expression: Value, description: String }`. Missing `expression` or `description` fields cause serde deserialization failures.
- **Wire protocol framing**: 4-byte big-endian length prefix + JSON. Max frame 16 MB. `TransportMessage` uses `#[serde(tag = "type", rename_all = "snake_case")]`.

## Core Analogy

SOMA = body. The runtime is an organism's body — it executes, senses, adapts. It does NOT interpret intent. Understanding these roles is critical:

| Role | Responsibility | Example |
|---|---|---|
| **Brain** (LLM or caller) | Intent interpretation, decision-making, composing inputs | Decides `table="users"`, writes SQL, chooses which port to call |
| **Body** (SOMA runtime) | Execution, observation, adaptation, proprioception | Invokes the port, records the result, updates belief, learns from episodes |

The body does not think. It acts. An organism's hand doesn't decide where to reach — the brain does. The hand provides proprioception (where it is, what it's touching), and the brain uses that to decide the next action.

**This means for soma-next:**
- The runtime MUST be domain-agnostic. It knows about skills, ports, observations, episodes — never about SQL, table names, HTTP verbs, or any port-specific semantics.
- Input binding comes from the caller (brain), belief state, working memory, or goal fields — never from hardcoded domain extraction in the runtime.
- The autonomous path works when skills have self-contained input schemas and the caller provides bindings via `GoalSpec.objective.structured` or prior observations populate working memory.
- `goal_utils.rs` extracts filesystem paths because `/tmp` is syntactically recognizable (starts with `/`). This is pattern recognition, not domain knowledge. Do NOT add SQL parsing, table name extraction, or any port-specific logic here.

**When editing soma-next, ask:** "Would this code change if the port were different?" If yes, it doesn't belong in the runtime.

## Rules

- **NEVER GUESS.** Read the code. Read the spec. If neither answers, ask the user.
- **NO SPEC CITATIONS IN COMMENTS.** Comments explain what and why, not where the requirement came from.
- **BODY ≠ BRAIN.** Never add port-specific or domain-specific logic to soma-next. The runtime is universal. Domain knowledge lives in pack manifests, skill declarations, and the caller (LLM).
- **READ THE ARCHITECTURE.** Before changing session.rs, adapters.rs, or any runtime component, read docs/architecture.md. The 16-step control loop, failure recovery table, and skill lifecycle are specified there. Follow the spec — don't invent new behavior.
- **NO HEDGING ABOUT EFFORT.** Never describe a task as "hard," "long," "significant work," "1-2 weeks," "non-trivial," "the biggest single step," etc. Just do it. Break it into concrete first steps and ship each one. Hedging language pre-defends against underdelivery instead of producing work. If you genuinely cannot complete a task in one shot, ship what you have and list the remaining steps as next work — never as warnings up front.
- **TIMEBOX DEBUGGING.** When the first or second patch doesn't fix a bug, STOP. Do not keep adding patches, debug prints, or workarounds on top of a wrong diagnosis. Back out to the last known-good state and re-examine the hypothesis from scratch. If a panic / error value looks suspiciously round (0, 0xFFFFFFFF) or absurdly huge, suspect **uninitialized memory, stack corruption, or format-string mismatch** before you suspect "zero-sized alloc" or "huge alloc" paths. Check the simplest causes first: heap too small, stack too small, buffer overflow, missing init. If the user says "stop" or "you're looping", you already overshot — immediately confirm nothing working has been regressed, summarize the state honestly, and offer to back out.
- **HONEST STATUS REPORTING.** "Proven" means a feature produced the expected user-visible behavior end-to-end on real hardware / real data, with test output captured in this session. "Compiles" means only that `cargo build` succeeded. "Boots" / "initialized" / "got further than last time" are NOT proof of working. When a test has intermediate success (e.g. 15/16 with 1 failure), report the exact failure — do not generalize to "working". Run the actual user-facing test (e.g. `./scripts/cycle.sh <chip>` for soma-project-esp32) before claiming "proven".

## When Editing

### soma-next
- `cargo test` after changes — 1177+ tests passing.
- `cargo clippy` — zero warnings.
- MCP tool changes: update build_tools(), add handler, add routing (tools/call AND direct dispatch), update tool count in tests (currently 19). The `McpTool` struct uses `#[serde(rename = "inputSchema")]` — MCP spec requires camelCase. tools/call responses are wrapped via `tool_success_response()` into MCP content array format.
- Episode/learning changes: update both cli.rs AND mcp.rs (both paths store episodes).
- Memory system: embedder.rs (GoalEmbedder trait), sequence_mining.rs (PrefixSpan), schemas.rs (induction), routines.rs (compilation).
- Plan-following: session.rs (active_plan logic after step 6), adapters.rs (SkillRegistryAdapter, SimpleSessionCritic).

### soma-ports
- Each port: cdylib crate, depends on soma-port-sdk, exports `soma_port_init`.
- `cargo build --release` builds all (except redis — separate manifest).
- After rebuilding ports, re-copy .dylib to project directories.

### Pack Manifests
- Full PackSpec format. Use packs/reference/manifest.json as template.
- port_id matches library name. observable_fields = [].
- Skills need all fields (see reference pack for exact format).

### Projects
- Copy binary: `cp soma-next/target/release/soma <project>/bin/soma`
- Copy port: `cp soma-ports/target/release/libsoma_port_*.dylib <project>/packs/*/`
- macOS: `xattr -d com.apple.quarantine bin/soma && codesign -fs - bin/soma`

## Current State (April 2026)

**What works:**
- LLM-driven path: invoke_port, list_ports, dump_state — all ports, all projects
- Autonomous path: create_goal → skill selection → port execution → episode → schema → routine → plan-following
- Memory: ring buffer, HashEmbedder, PrefixSpan, consolidation, disk persistence
- 6 proof projects (SMTP, S3, Postgres, LLM, MCP, S2S) + HelperBook app + multistep proof
- 44/44 capabilities checklist, 1177 unit tests
- Cross-compilation to Android (aarch64-linux-android) and iOS (aarch64-apple-ios) — both verified
- **ESP32-S3 + ESP32 LX6 firmware dual-chip with wifi** — 14/14 non-wifi and 16/16 wifi tests on both real boards. `./scripts/cycle.sh esp32s3 wifi` and `./scripts/cycle.sh esp32 wifi`. Real `wifi.scan` returns live APs on both chips.
- **ESP32 runtime pin configuration + SSD1306 OLED display port** — `board.configure_pin` + `board.reboot` persists pin assignments to flash across boots (verified end-to-end over MCP). `display.draw_text` renders visibly on a real OLED panel on the WROOM-32D via `embedded-hal-bus::RefCellDevice` sharing the I²C0 bus with the `i2c` port. `scripts/thermistor-to-display.py` 5-second loop showing "Temperature: 22.00 C" + ancillary lines confirmed on the physical panel by the user.
- **mDNS auto-discovery for leaf peers** — soma-next `--discover-lan` browses `_soma._tcp.local.` via `mdns-sd` and registers discovered peers. ESP32 leaf announces via `edge-mdns` + smoltcp UDP on 224.0.0.251:5353 after DHCP. `list_peers` returns the leaf without any static configuration.

## Multi-step routines: PROVEN

`soma-project-multistep` (April 2026) proves the full multi-step learning chain works against the real library, not just unit tests in isolation:

| Component | Status | How proven |
|---|---|---|
| Multi-step episodes can be stored | PROVEN | Phase 1: 5 episodes with 3-step traces stored in `DefaultEpisodeStore` |
| PrefixSpan induces multi-step schemas | PROVEN | Phase 2: `induce_from_episodes_with_embedder` produces schema with 3-step `candidate_skill_ordering`, confidence 0.950 |
| Multi-step schemas compile to multi-step routines | PROVEN | Phase 3: `compile_from_schema` produces routine with 3-step `compiled_skill_path` |
| Plan-following logic walks every step | PROVEN | Phase 4: simulated against `runtime/session.rs:1612-1913`, 3 steps walked, 2 Continue + 1 Stop |
| **Real `SessionController.run_step()` walks multi-step routines end-to-end** | **PROVEN** | **Phase 5: bootstrap + reference pack + injected routine + real `FilesystemPort` against `/tmp`. Trace: stat → readdir → stat. Final status: `Completed`** |

What's still NOT proven (separate concerns, not blockers for multi-step routines):
- Real autonomous control loop producing multi-step episodes from a single goal naturally — requires selector/critic to chain skills without explicit prompting. Multi-step routines work *given* multi-step episodes; producing them organically from one goal is a separate selector/critic question.
- `SimpleBeliefSource` extracting bindings from `goal.objective.structured`. The default belief source ignores the goal entirely (`build_initial_belief(_goal)`). Phase 5 worked around this by injecting `Binding { name: "path", value: "/tmp", ... }` directly into `session.belief.active_bindings`.

How to reproduce: `cd soma-project-multistep && cargo run`. Five phases, all PASS, exit 0.

## Cross-compilation: PROVEN

Both Android and iOS compile from `soma-next` with no code changes after two Cargo.toml fixes:

1. `reqwest = { version = "0.12", default-features = false, features = ["json", "blocking", "rustls-tls"] }` — drops openssl-sys (which can't cross-compile), uses rustls which SOMA already depends on.
2. `rustls = { version = "0.23", features = ["ring"] }` — explicit crypto provider so the test that constructs a TLS client can install_default.

One unit test (`distributed::transport::tests::tls_executor_ok_without_ca`) needed `rustls::crypto::ring::default_provider().install_default()` because both ring and aws-lc-rs are pulled in by different transitive deps.

Build commands:
- Android: `rustup target add aarch64-linux-android && cargo install cargo-ndk && cd soma-next && cargo ndk -t arm64-v8a build --release` (NDK installed via Android Studio SDK Tools, version 30.0.14904198 verified)
- iOS: `rustup target add aarch64-apple-ios && cd soma-next && cargo build --target aarch64-apple-ios --release` (no NDK, Xcode SDK is auto-discovered via xcrun)

Output: 10MB ELF (Android), 9MB Mach-O (iOS). Both contain the full runtime — control loop, memory pipeline, MCP server (19 tools), distributed transport, all built-in ports.

iOS restriction worth noting: programmatic SMS is blocked by Apple (`MFMessageComposeViewController` requires user tap per message). iOS is best as a *perception peer* (camera, location, sensors, HealthKit). Android handles *actuation* (SMS, calls). See soma-project-android/POC.md and soma-project-ios/POC.md.

## Design proposals (not implemented)

- **docs/semantic-memory.md** — declarative memory tier as a "second brain." Associative graph with spreading activation, consolidation, decay. Structure-driven extraction (no manifest declarations, no LLM-at-extraction-time). Status: design proposal, not implemented. Body extracts topology from `PortCallRecord.structured_result`; brain (LLM) supplies labels later.
- **docs/memory-fusion.md** — what happens when the existing procedural pipeline (PrefixSpan over skill sequences) combines with the proposed semantic memory (entity extraction from observation contents). Produces **entity-parameterized routines** — compiled procedures with typed inputs and outputs that generalize across novel entities of known types. Includes content-addressed `InputStore` (deduplicated, no PortCallRecord ABI change), enterprise-grade blocker analysis (10 ranked items), and async extraction architecture with episode retention watermark. Status: design proposal, not implemented.

These are the next architectural directions if SOMA pursues the cognitive-architecture path. They build on multi-step routines (now proven) and the existing schema/routine pipeline.

## What's next (revised)

Now that multi-step routines are proven:
- Make the autonomous control loop produce multi-step episodes naturally (selector + critic must chain skills from one goal without explicit prompting). This is the remaining gap before fusion has organic input.
- Default `BeliefSource` should extract bindings from `goal.objective.structured` (small change to `SimpleBeliefSource::build_initial_belief`). Currently the goal is ignored entirely.
- Composite skills (skill sequences as first-class entities) — alternative path to multi-step that doesn't require learning.
- ONNX embedding model — only matters for ESP32 (no LLM on-device); on every other deployment, the LLM does semantic matching and HashEmbedder is sufficient.
- Memory fusion (semantic memory + entity-parameterized routines) — see docs/memory-fusion.md. Substantial work, requires multi-step natural episode production first.
- ESP32 target (no_std, pre-loaded routines from capable peer)
- Native Android app build (libsoma_android.so + Kotlin bridge) — see soma-project-android/POC.md
- Native iOS app build (libsoma_ios.a + Swift FFI) — see soma-project-ios/POC.md

## What's Deferred

- TransformerMind architecture
- WASM sandbox for untrusted plugins
- Plugin registry (download/cache)
- Diffuse memory tier (peer queries)
