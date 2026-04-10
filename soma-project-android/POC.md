# SOMA Android — Native App POC

SOMA runtime as a native Android application. The phone becomes a SOMA instance — a physical peer with ports that access SMS, camera, location, sensors, and telephony through Android SDK APIs.

## What This Is

A native Android app that embeds the soma-next Rust runtime as a shared library (.so) loaded via JNI. The app runs SOMA as a foreground service with a persistent notification. Ports call Android SDK APIs directly through JNI callbacks. The phone participates in the SOMA peer network over TCP/TLS/WebSocket — any LLM or remote SOMA instance can invoke the phone's physical capabilities.

```
┌──────────────────────────────────────────────────────┐
│  Android App (Kotlin)                                │
│                                                      │
│  ┌──────────────┐  ┌─────────────────────────────┐   │
│  │ Foreground   │  │ UI (optional)               │   │
│  │ Service      │  │  - Status / logs            │   │
│  │              │  │  - Port status              │   │
│  │  starts ──►  │  │  - Peer connections         │   │
│  │  SOMA JNI    │  │  - Manual invoke            │   │
│  └──────┬───────┘  └─────────────────────────────┘   │
│         │                                            │
│  ┌──────▼───────────────────────────────────────┐    │
│  │ libsoma_android.so (Rust)                    │    │
│  │                                              │    │
│  │  soma-next runtime                           │    │
│  │   ├── control loop                           │    │
│  │   ├── memory (episodes/schemas/routines)     │    │
│  │   ├── MCP server (TCP, not stdin)            │    │
│  │   ├── distributed transport (peer network)   │    │
│  │   └── port loader                            │    │
│  │                                              │    │
│  │  compiled-in ports (no dynamic loading)      │    │
│  │   ├── sms port      ──► JNI ──► SmsManager  │    │
│  │   ├── camera port   ──► JNI ──► CameraX     │    │
│  │   ├── location port ──► JNI ──► FusedLoc    │    │
│  │   ├── sensor port   ──► JNI ──► SensorMgr   │    │
│  │   ├── contacts port ──► JNI ──► ContentRes   │    │
│  │   ├── phone port    ──► JNI ──► TelecomMgr  │    │
│  │   ├── notify port   ──► JNI ──► NotifMgr    │    │
│  │   ├── storage port  ──► JNI ──► MediaStore  │    │
│  │   ├── http port     (built-in, reqwest)      │    │
│  │   └── filesystem port (built-in, std::fs)    │    │
│  └──────────────────────────────────────────────┘    │
│                                                      │
│  Network: TCP :9100 (listener) + peer connections    │
│  Storage: app internal dir for episodes/routines     │
└──────────────────────────────────────────────────────┘
```

## Architecture

### Rust Library (libsoma_android.so)

soma-next compiled as a `cdylib` for `aarch64-linux-android` (and `armv7-linux-androideabi` for older devices). The Cargo target produces a `.so` that the Android app loads via `System.loadLibrary()`.

Key change from the desktop binary: ports are **compiled in**, not dynamically loaded. On Android, dynamic `.so` loading from arbitrary paths is restricted. Instead, each Android port implements the same `Port` trait and is registered at startup by the Rust init function.

```toml
# Cargo.toml for soma-android lib
[lib]
name = "soma_android"
crate-type = ["cdylib"]

[dependencies]
soma-next = { path = "../soma-next", default-features = false }
jni = "0.21"
android_logger = "0.14"
log = "0.4"
```

**JNI entry points exported by the .so:**

```rust
// Called once when the service starts.
// Receives the JNI env and a callback object for invoking Android APIs.
#[no_mangle]
pub extern "C" fn Java_com_soma_runtime_SomaRuntime_init(
    env: JNIEnv,
    _class: JClass,
    callback: JObject,     // Kotlin object implementing SomaBridge interface
    data_dir: JString,     // app internal storage path
    config_json: JString,  // runtime config (peers, listen addr, packs)
) -> jlong {
    // 1. Store global ref to callback for JNI port calls
    // 2. Load pack manifests from assets or data_dir
    // 3. Bootstrap runtime with compiled-in ports
    // 4. Start MCP server on TCP (not stdin — no terminal)
    // 5. Start distributed listener if configured
    // 6. Return pointer to runtime handle (opaque to Kotlin)
}

// Called to submit a goal or invoke a port from the UI.
#[no_mangle]
pub extern "C" fn Java_com_soma_runtime_SomaRuntime_invokePort(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    port_id: JString,
    capability: JString,
    input_json: JString,
) -> jstring {
    // Delegates to runtime handle's invoke_port
    // Returns JSON result string
}

// Called to get runtime state (for UI display).
#[no_mangle]
pub extern "C" fn Java_com_soma_runtime_SomaRuntime_dumpState(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    // Returns dump_state JSON
}

// Shutdown.
#[no_mangle]
pub extern "C" fn Java_com_soma_runtime_SomaRuntime_shutdown(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    // Persist episodes, close peers, drop runtime
}
```

### JNI Bridge Pattern

Android ports can't call SDK APIs directly from Rust. The bridge:

1. **Rust side**: Port's `invoke()` calls a stored JNI global ref to a Kotlin callback object.
2. **Kotlin side**: Callback implements the `SomaBridge` interface, dispatches to Android SDK APIs.
3. **Return path**: Kotlin method returns a JSON string, Rust parses it into `PortCallRecord`.

```kotlin
// Kotlin — the bridge between Rust ports and Android SDK
interface SomaBridge {
    fun sendSms(to: String, body: String): String          // returns JSON result
    fun takePhoto(outputPath: String): String
    fun getLocation(): String
    fun readSensor(sensorType: String): String
    fun getContacts(query: String): String
    fun sendNotification(title: String, body: String): String
    fun makeCall(number: String): String
    fun listFiles(path: String): String
}
```

```rust
// Rust — SMS port implementation (inside libsoma_android.so)
pub struct SmsPort {
    spec: PortSpec,
    bridge: Arc<Mutex<JniBridge>>,  // holds GlobalRef to SomaBridge
}

impl Port for SmsPort {
    fn invoke(&self, capability_id: &str, input: Value) -> Result<PortCallRecord> {
        let start = Instant::now();

        let result = match capability_id {
            "send_sms" => {
                let to = input["to"].as_str().ok_or(/* validation error */)?;
                let body = input["body"].as_str().ok_or(/* validation error */)?;

                let bridge = self.bridge.lock().unwrap();
                bridge.call_method("sendSms", &[to, body])?
            }
            "list_received" => {
                let since = input.get("since").and_then(|v| v.as_str());
                let bridge = self.bridge.lock().unwrap();
                bridge.call_method("listReceived", &[since.unwrap_or("")])?
            }
            _ => return Err(/* unknown capability */)
        };

        Ok(PortCallRecord {
            port_id: "sms".into(),
            capability_id: capability_id.into(),
            success: true,
            raw_result: serde_json::from_str(&result)?,
            latency_ms: start.elapsed().as_millis() as u64,
            // ... standard fields
        })
    }
}
```

The JniBridge struct manages the JNI env attachment (Android requires `AttachCurrentThread` for Rust threads calling back into Java):

```rust
pub struct JniBridge {
    jvm: JavaVM,           // cached from JNI_OnLoad or init
    callback: GlobalRef,   // SomaBridge Kotlin object
}

impl JniBridge {
    pub fn call_method(&self, method: &str, args: &[&str]) -> Result<String> {
        let mut env = self.jvm.attach_current_thread()?;

        // Build JNI call based on method name and args
        // Each method on SomaBridge returns a String (JSON)
        let result = env.call_method(
            &self.callback,
            method,
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
            &[JValue::from(env.new_string(args[0])?),
              JValue::from(env.new_string(args[1])?)],
        )?;

        let jstr = JString::from(result.l()?);
        let rstr = env.get_string(&jstr)?;
        Ok(rstr.to_str()?.to_owned())
    }
}
```

### Android App (Kotlin)

```
app/
  src/main/
    java/com/soma/runtime/
      SomaRuntime.kt         # JNI bindings (native method declarations)
      SomaBridge.kt           # Interface for Android API callbacks
      SomaBridgeImpl.kt       # Implementation: SMS, camera, location, etc.
      SomaService.kt          # Foreground service — starts/stops runtime
      MainActivity.kt         # Optional UI: status, logs, manual invoke
    jniLibs/
      arm64-v8a/
        libsoma_android.so    # Compiled Rust library
      armeabi-v7a/
        libsoma_android.so    # (optional: 32-bit support)
    res/
      layout/activity_main.xml
    AndroidManifest.xml       # Permissions, foreground service declaration
  build.gradle.kts
```

**SomaRuntime.kt — JNI declarations:**

```kotlin
class SomaRuntime {
    companion object {
        init { System.loadLibrary("soma_android") }
    }

    external fun init(bridge: SomaBridge, dataDir: String, configJson: String): Long
    external fun invokePort(handle: Long, portId: String, capability: String, inputJson: String): String
    external fun dumpState(handle: Long): String
    external fun shutdown(handle: Long)
}
```

**SomaService.kt — foreground service:**

```kotlin
class SomaService : Service() {
    private var runtimeHandle: Long = 0
    private lateinit var runtime: SomaRuntime
    private lateinit var bridge: SomaBridgeImpl

    override fun onCreate() {
        super.onCreate()
        runtime = SomaRuntime()
        bridge = SomaBridgeImpl(this)

        val config = buildConfig()  // listen addr, peer addrs, pack paths
        runtimeHandle = runtime.init(bridge, filesDir.absolutePath, config)

        // Start as foreground service to survive battery optimization
        val notification = buildNotification("SOMA running — ${bridge.portCount()} ports active")
        startForeground(NOTIFICATION_ID, notification)
    }

    override fun onDestroy() {
        runtime.shutdown(runtimeHandle)
        super.onDestroy()
    }

    override fun onBind(intent: Intent): IBinder? = null
}
```

**SomaBridgeImpl.kt — actual Android API calls:**

```kotlin
class SomaBridgeImpl(private val context: Context) : SomaBridge {

    override fun sendSms(to: String, body: String): String {
        return try {
            val smsManager = context.getSystemService(SmsManager::class.java)
            smsManager.sendTextMessage(to, null, body, null, null)
            """{"success": true, "to": "$to", "length": ${body.length}}"""
        } catch (e: Exception) {
            """{"success": false, "error": "${e.message}"}"""
        }
    }

    override fun takePhoto(outputPath: String): String {
        // CameraX capture to outputPath
        // Returns: {"success": true, "path": "/data/.../photo.jpg", "size_bytes": 2048576}
    }

    override fun getLocation(): String {
        val fusedClient = LocationServices.getFusedLocationProviderClient(context)
        // Request last known or fresh location
        // Returns: {"lat": 47.0105, "lon": 28.8638, "accuracy_m": 12.5, "provider": "fused"}
    }

    override fun readSensor(sensorType: String): String {
        val sensorManager = context.getSystemService(SensorManager::class.java)
        // Read accelerometer, gyroscope, light, proximity, etc.
        // Returns: {"type": "accelerometer", "values": [0.12, 9.78, 0.34], "accuracy": 3}
    }

    override fun getContacts(query: String): String {
        val cursor = context.contentResolver.query(
            ContactsContract.CommonDataKinds.Phone.CONTENT_URI, ...)
        // Returns: {"contacts": [{"name": "...", "phone": "..."}], "count": 42}
    }

    override fun sendNotification(title: String, body: String): String {
        val manager = context.getSystemService(NotificationManager::class.java)
        // Build and post notification
        // Returns: {"success": true, "notification_id": 12345}
    }

    override fun makeCall(number: String): String {
        val intent = Intent(Intent.ACTION_CALL, Uri.parse("tel:$number"))
        intent.flags = Intent.FLAG_ACTIVITY_NEW_TASK
        context.startActivity(intent)
        // Returns: {"success": true, "number": "..."}
    }

    override fun listFiles(path: String): String {
        // MediaStore query or direct file listing
        // Returns: {"files": [...], "count": 15}
    }
}
```

### Android Permissions

```xml
<manifest>
    <uses-permission android:name="android.permission.SEND_SMS" />
    <uses-permission android:name="android.permission.READ_SMS" />
    <uses-permission android:name="android.permission.RECEIVE_SMS" />
    <uses-permission android:name="android.permission.CAMERA" />
    <uses-permission android:name="android.permission.ACCESS_FINE_LOCATION" />
    <uses-permission android:name="android.permission.ACCESS_COARSE_LOCATION" />
    <uses-permission android:name="android.permission.READ_CONTACTS" />
    <uses-permission android:name="android.permission.CALL_PHONE" />
    <uses-permission android:name="android.permission.BODY_SENSORS" />
    <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
    <uses-permission android:name="android.permission.INTERNET" />
    <uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
    <uses-permission android:name="android.permission.FOREGROUND_SERVICE_CONNECTED_DEVICE" />
    <uses-permission android:name="android.permission.WAKE_LOCK" />

    <application>
        <service
            android:name=".SomaService"
            android:foregroundServiceType="connectedDevice"
            android:exported="false" />
    </application>
</manifest>
```

Runtime permissions (SMS, camera, location, contacts, phone, sensors) are requested on first launch via the standard Android permission flow.

## Ports

### Port Manifest (sms example)

Same PackSpec format as all SOMA ports. Loaded from app assets or internal storage.

```json
{
  "id": "soma.ports.sms",
  "name": "SMS",
  "version": "0.1.0",
  "namespace": "soma.ports.sms",
  "description": "Send and receive SMS through the device SIM card",
  "ports": [
    {
      "port_id": "sms",
      "name": "sms",
      "version": "0.1.0",
      "kind": "service",
      "description": "SMS via Android SmsManager",
      "namespace": "soma.ports.sms",
      "trust_level": "verified",
      "capabilities": [
        {
          "capability_id": "send_sms",
          "name": "send_sms",
          "purpose": "Send an SMS message through the device SIM card",
          "input_schema": {
            "schema": {
              "type": "object",
              "required": ["to", "body"],
              "properties": {
                "to": { "type": "string", "description": "Phone number (E.164 format)" },
                "body": { "type": "string", "description": "Message text (max 160 chars per segment)" }
              }
            }
          },
          "output_schema": {
            "schema": {
              "type": "object",
              "properties": {
                "success": { "type": "boolean" },
                "to": { "type": "string" },
                "segments": { "type": "integer" }
              }
            }
          },
          "effect_class": "external_state_mutation",
          "rollback_support": "not_applicable",
          "determinism_class": "non_deterministic",
          "idempotence_class": "non_idempotent",
          "risk_class": "medium",
          "latency_profile": { "expected_latency_ms": 500, "p95_latency_ms": 3000, "max_latency_ms": 10000 },
          "cost_profile": {
            "cpu_cost_class": "negligible",
            "memory_cost_class": "negligible",
            "io_cost_class": "negligible",
            "network_cost_class": "low",
            "energy_cost_class": "low"
          },
          "remote_exposable": true
        },
        {
          "capability_id": "list_received",
          "name": "list_received",
          "purpose": "List received SMS messages from the device inbox",
          "input_schema": {
            "schema": {
              "type": "object",
              "properties": {
                "since": { "type": "string", "description": "ISO 8601 timestamp — only messages after this time" },
                "from": { "type": "string", "description": "Filter by sender number" },
                "limit": { "type": "integer", "description": "Max messages to return (default 50)" }
              }
            }
          },
          "output_schema": {
            "schema": {
              "type": "object",
              "properties": {
                "messages": {
                  "type": "array",
                  "items": {
                    "type": "object",
                    "properties": {
                      "from": { "type": "string" },
                      "body": { "type": "string" },
                      "timestamp": { "type": "string" },
                      "read": { "type": "boolean" }
                    }
                  }
                },
                "count": { "type": "integer" }
              }
            }
          },
          "effect_class": "pure_computation",
          "rollback_support": "not_applicable",
          "determinism_class": "non_deterministic",
          "idempotence_class": "idempotent",
          "risk_class": "low",
          "latency_profile": { "expected_latency_ms": 100, "p95_latency_ms": 500, "max_latency_ms": 2000 },
          "cost_profile": {
            "cpu_cost_class": "negligible",
            "memory_cost_class": "negligible",
            "io_cost_class": "low",
            "network_cost_class": "negligible",
            "energy_cost_class": "negligible"
          },
          "remote_exposable": true
        }
      ],
      "observable_fields": [],
      "remote_exposure": true
    }
  ]
}
```

### All Android Ports

| Port | Capabilities | Android API | Risk |
|------|-------------|-------------|------|
| **sms** | send_sms, list_received, delete_sms | SmsManager, ContentResolver | medium |
| **camera** | take_photo, record_video, list_photos | CameraX, MediaStore | low |
| **location** | get_location, track_location, geofence | FusedLocationProvider | low |
| **sensor** | read_sensor, list_sensors, stream_sensor | SensorManager | low |
| **contacts** | list_contacts, search_contacts, add_contact | ContactsContract | medium |
| **phone** | make_call, end_call, call_log | TelecomManager | high |
| **notify** | send_notification, cancel_notification, list_channels | NotificationManager | low |
| **storage** | list_files, read_file, write_file, delete_file, media_scan | MediaStore, SAF | medium |
| **battery** | get_status, get_health, get_temperature | BatteryManager | low |
| **wifi** | scan_networks, get_connection, get_signal | WifiManager | low |
| **bluetooth** | scan_devices, pair, send_data | BluetoothAdapter | medium |
| **clipboard** | get, set, clear | ClipboardManager | low |

Plus the two built-in SOMA ports (http, filesystem) that work without JNI.

## Network Topology

### Standalone Mode

Phone runs SOMA with a TCP listener. An MCP client (or LLM with network MCP transport) connects directly to the phone.

```
LLM ──► TCP :9100 ──► Phone SOMA ──► SmsManager ──► cellular network
```

Requires the phone to be reachable (same WiFi, or Tailscale/WireGuard tunnel).

### Peer Mode (s2s)

Phone SOMA connects to a server SOMA instance as a peer. LLM talks to the server. Server delegates physical-world operations to the phone.

```
                        TCP 9100
┌─────────────┐     ◄──────────────     ┌─────────────────┐
│ Server SOMA │                         │ Phone SOMA      │
│  postgres   │     s2s wire proto      │  sms            │
│  s3         │    ────────────────►    │  camera         │
│  smtp       │                         │  location       │
│  --listen   │                         │  sensors        │
└──────┬──────┘                         │  --peer server  │
       │                                └─────────────────┘
  LLM (Claude)
  via MCP stdin
```

The LLM sees all capabilities from both peers:

```json
{"method": "tools/call", "params": {"name": "invoke_port",
  "arguments": {"port_id": "postgres", "capability": "query",
    "input": {"sql": "SELECT * FROM users"}}}}

{"method": "tools/call", "params": {"name": "invoke_remote_skill",
  "arguments": {"peer_id": "phone-0", "skill_id": "sms.send_sms",
    "input": {"to": "+40712345678", "body": "Meeting confirmed"}}}}
```

One LLM, one conversation, database queries on the server, SMS through the phone. This is the distributed body — multiple physical nodes, one brain.

### Multi-Device

```
                   Server SOMA (:9100)
                  /        |         \
           phone-0      phone-1     rpi-0
           (sms,cam)   (sms,loc)   (gpio,temp)
```

Multiple Android phones, a Raspberry Pi, all as peers. LLM picks the right device based on capabilities. `list_peers` shows what's available, each peer advertises its ports.

## Cross-Compilation (Proven)

soma-next cross-compiles for Android with **zero runtime code changes**. Only two build config changes were needed, both already merged.

### What was required

**1. reqwest: OpenSSL → rustls**

Android cross-compilation fails with `openssl-sys` because there's no OpenSSL sysroot for `aarch64-linux-android`. SOMA already uses rustls for TLS everywhere else, so reqwest was switched to match:

```toml
# Before (fails on Android — openssl-sys can't find OpenSSL for cross-compilation)
reqwest = { version = "0.12", features = ["json", "blocking"] }

# After (uses rustls, which SOMA already depends on — zero new dependencies)
reqwest = { version = "0.12", default-features = false, features = ["json", "blocking", "rustls-tls"] }
```

**2. rustls: explicit crypto provider**

With both `ring` and `aws-lc-rs` present (pulled by different deps), rustls can't auto-select a crypto provider. One test needed an explicit install:

```toml
# Cargo.toml — explicit ring feature
rustls = { version = "0.23", features = ["ring"] }
```

```rust
// One test needed this line added (transport.rs — tls_executor_ok_without_ca)
let _ = rustls::crypto::ring::default_provider().install_default();
```

That's it. No `#[cfg(target_os = "android")]`, no conditional compilation, no feature flags. The full runtime compiles as-is.

### Toolchain setup

**Prerequisites:**
- Rust stable toolchain
- Android Studio with NDK installed (Settings → Languages & Frameworks → Android SDK → SDK Tools → NDK (Side by side))

**Install once:**

```bash
# Add the Android aarch64 target to Rust
rustup target add aarch64-linux-android

# Install cargo-ndk — handles NDK linker config automatically
cargo install cargo-ndk
```

NDK version proven: `30.0.14904198`. cargo-ndk auto-discovers the NDK from `~/Library/Android/sdk/ndk/`.

### Build the binary

```bash
cd soma-next
cargo ndk -t arm64-v8a build --release
```

Output: `target/aarch64-linux-android/release/soma`

### Build result

```
File:   target/aarch64-linux-android/release/soma
Format: ELF 64-bit LSB pie executable, ARM aarch64, version 1 (SYSV)
Linker: dynamically linked, interpreter /system/bin/linker64
Size:   10 MB
```

This is the full soma-next binary — complete runtime with:
- 16-step control loop with plan-following
- Memory system (episodes, schemas, routines, PrefixSpan, HashEmbedder)
- MCP server (19 tools)
- Distributed transport (TCP, TLS, WebSocket, Unix socket)
- Built-in ports (HTTP via reqwest, filesystem via std::fs)
- Dynamic port loading (libloading, loads .so on Android)
- Policy engine, belief state, critic, predictor, selector
- Ed25519 peer authentication, rate limiting, heartbeat

All 1177 tests pass on the host after these changes. Zero clippy warnings.

### Build as library (.so for JNI)

For the native Android app, soma-next compiles as a shared library instead of a binary. This requires adding a `lib.rs` to soma-next that exports the runtime bootstrap, port registration, and MCP handler as library functions. The `main.rs` CLI/REPL logic stays separate.

```toml
# soma-project-android/rust/Cargo.toml
[lib]
name = "soma_android"
crate-type = ["cdylib"]

[dependencies]
soma-next = { path = "../../soma-next" }
jni = "0.21"
android_logger = "0.14"
log = "0.4"
```

```bash
# Build .so and place in Android project jniLibs
cd soma-project-android/rust
cargo ndk -t arm64-v8a -o ../app/src/main/jniLibs build --release
```

Output: `app/src/main/jniLibs/arm64-v8a/libsoma_android.so`

### Conditional compilation (minimal, only where needed)

```rust
// MCP transport: TCP on Android (no terminal), stdin/stdout on desktop
#[cfg(target_os = "android")]
fn mcp_transport() -> impl Transport { TcpTransport::new(bind_addr) }

#[cfg(not(target_os = "android"))]
fn mcp_transport() -> impl Transport { StdioTransport::new() }
```

The runtime itself needs no `#[cfg]` gates. Only the transport layer and logging differ.

## Storage

Android restricts file access. SOMA stores everything in the app's internal directory:

```
/data/data/com.soma.runtime/files/
  soma/
    episodes/          # Episode ring buffer persistence
    schemas/           # Induced schemas
    routines/          # Compiled routines
    checkpoints/       # Session checkpoints
    packs/             # Pack manifests (copied from assets on first launch)
    config.toml        # Runtime config
```

No external storage needed. No root needed. Standard Android app sandbox.

## Security

The phone is a physical device with real capabilities. Security is not optional.

**Network layer:**
- TLS required for all remote connections (rustls, already in soma-next)
- Ed25519 peer authentication (already in soma-next distributed/auth.rs)
- Rate limiting per peer (already in soma-next distributed/rate_limit.rs)

**Port layer:**
- SOMA policy engine enforces risk budgets per session
- `send_sms` and `make_call` are `risk_class: medium/high` — policy can require confirmation
- The app UI can intercept high-risk invocations and show a confirmation dialog before execution

**Android layer:**
- Runtime permissions granted explicitly by the user
- Foreground service notification shows SOMA is active (transparent)
- No root, no ADB, no side-loading required — standard Play Store app

**Trust model:**
- The phone owner controls which peers can connect (peer allowlist in config)
- The phone owner controls which ports are active (toggle per port in UI)
- The phone owner sees all invocations in the notification/log

## What This Proves

1. **SOMA as a physical body.** Not just databases and APIs — actual hardware: cellular radio, camera sensor, GPS receiver, accelerometer. The runtime IS the body becomes literal.

2. **Web 4 on a phone.** No app source code for the "application." The phone runs a generic runtime. What it does is determined by which ports are active and which LLM is driving it. The same phone instance can be a messaging app, a camera tool, a location tracker, or a sensor dashboard — depending on what the LLM asks for.

3. **Distributed embodiment.** Server SOMA has cloud ports (postgres, s3, smtp). Phone SOMA has physical ports (sms, camera, location). They cooperate as peers. One brain, distributed body. The s2s infrastructure already handles this — the phone is just another peer with different capabilities.

4. **The port abstraction holds.** `invoke_port("sms", "send_sms", {...})` on Android is the same interface as `invoke_port("postgres", "query", {...})` on a server. The LLM doesn't know or care that one is a database and the other is a cellular radio. Same brain, same protocol, different body parts.

## Milestones

| # | Milestone | What | Validates | Status |
|---|-----------|------|-----------|--------|
| 1 | **Rust compiles for Android** | `cargo ndk -t arm64-v8a build --release` produces 10MB ELF binary. NDK 30.0, zero code changes. | Cross-compilation toolchain works | **DONE** |
| 2 | **JNI bridge works** | Kotlin calls Rust init, Rust calls Kotlin callback, round-trip JSON | JNI bridge pattern is viable | |
| 3 | **SMS port sends a message** | LLM → invoke_port("sms", "send_sms") → real SMS delivered | First physical-world port works | |
| 4 | **Foreground service survives** | SOMA stays alive for 24h with battery optimization enabled | Android lifecycle is handled | |
| 5 | **s2s peer connection** | Phone SOMA connects to server SOMA, invoke_remote_skill works | Distributed body works | |
| 6 | **All ports operational** | SMS, camera, location, sensors, contacts, phone, notify, storage | Full device access through SOMA | |
| 7 | **Routine transfer** | Server compiles a routine, transfers it to phone, phone executes it autonomously | Phone learns from server | |
| 8 | **Multi-device mesh** | 2+ phones + server, LLM routes to correct device by capability | Distributed body scales | |
