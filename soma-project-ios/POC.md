# SOMA iOS — Native App POC

SOMA runtime as a native iOS application. The iPhone becomes a SOMA instance — a physical peer with ports that access camera, location, sensors, contacts, notifications, and health data through iOS SDK APIs.

## What This Is

A native iOS app that embeds the soma-next Rust runtime as a static library (.a) linked via C FFI + Swift bridge. The app runs SOMA as a background task with a persistent activity. Ports call iOS SDK APIs directly through Swift callbacks. The phone participates in the SOMA peer network over TCP/TLS/WebSocket — any LLM or remote SOMA instance can invoke the phone's physical capabilities.

```
┌──────────────────────────────────────────────────────┐
│  iOS App (Swift)                                     │
│                                                      │
│  ┌──────────────┐  ┌─────────────────────────────┐   │
│  │ Background   │  │ UI (SwiftUI)                │   │
│  │ Task /       │  │  - Status / logs            │   │
│  │ Live         │  │  - Port status              │   │
│  │ Activity     │  │  - Peer connections         │   │
│  │  starts ──►  │  │  - Manual invoke            │   │
│  │  SOMA FFI    │  │  - High-risk confirmation   │   │
│  └──────┬───────┘  └─────────────────────────────┘   │
│         │                                            │
│  ┌──────▼───────────────────────────────────────┐    │
│  │ libsoma_ios.a (Rust)                         │    │
│  │                                              │    │
│  │  soma-next runtime                           │    │
│  │   ├── control loop                           │    │
│  │   ├── memory (episodes/schemas/routines)     │    │
│  │   ├── MCP server (TCP, not stdin)            │    │
│  │   ├── distributed transport (peer network)   │    │
│  │   └── port loader                            │    │
│  │                                              │    │
│  │  compiled-in ports (no dynamic loading)      │    │
│  │   ├── camera port   ──► FFI ──► AVFoundation │    │
│  │   ├── location port ──► FFI ──► CoreLocation │    │
│  │   ├── sensor port   ──► FFI ──► CoreMotion   │    │
│  │   ├── contacts port ──► FFI ──► Contacts.fwk │    │
│  │   ├── notify port   ──► FFI ──► UserNotif    │    │
│  │   ├── health port   ──► FFI ──► HealthKit    │    │
│  │   ├── storage port  ──► FFI ──► FileManager  │    │
│  │   ├── calendar port ──► FFI ──► EventKit     │    │
│  │   ├── speech port   ──► FFI ──► Speech.fwk   │    │
│  │   ├── haptic port   ──► FFI ──► CoreHaptics  │    │
│  │   ├── http port     (built-in, reqwest)      │    │
│  │   └── filesystem port (built-in, std::fs)    │    │
│  └──────────────────────────────────────────────┘    │
│                                                      │
│  Network: TCP :9100 (listener) + peer connections    │
│  Storage: app container for episodes/routines        │
└──────────────────────────────────────────────────────┘
```

### iOS Restrictions vs Android

iOS is more locked down than Android. Key differences that affect the architecture:

| Capability | Android | iOS | Impact |
|---|---|---|---|
| **SMS** | `SmsManager` — silent send, full inbox access | `MFMessageComposeViewController` — requires user tap per message | **No silent SMS port on iOS.** User-interactive only. |
| **Phone calls** | `TelecomManager` — programmatic dial | `tel:` URL scheme — requires user confirmation | Same UX restriction |
| **Background execution** | Foreground service — runs indefinitely | Background tasks limited to ~30s; BGProcessingTask ~minutes; Live Activities for UI | Must use Network Extension or Audio/Location background mode for persistent runtime |
| **Dynamic loading** | `.so` from app directory works | **Forbidden.** No `dlopen` of user code on non-jailbroken devices | Ports must be compiled in (same conclusion as Android, but enforced by OS) |
| **Sideloading** | APK install, no store needed | TestFlight (90 days) or App Store only; AltStore for dev | Harder to distribute |

The SMS restriction is fundamental — Apple blocks programmatic SMS by design. This is not a workaround-able limitation. The iOS SOMA instance is strongest as a **sensor/perception peer** (camera, location, motion, health) rather than an **actuation peer** (SMS, calls).

## Architecture

### Rust Library (libsoma_ios.a)

soma-next compiled as a `staticlib` for `aarch64-apple-ios`. iOS apps link static libraries — dynamic frameworks are possible but static is simpler and avoids code signing complexity.

The C FFI bridge uses `cbindgen` to generate a C header from Rust, which Swift imports directly.

```toml
# Cargo.toml for soma-ios lib
[lib]
name = "soma_ios"
crate-type = ["staticlib"]

[dependencies]
soma-next = { path = "../../soma-next" }
libc = "0.2"
```

**C FFI entry points exported by the .a:**

```rust
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

/// Opaque handle to the running SOMA runtime.
pub struct SomaHandle { /* runtime, bridge, tokio runtime */ }

/// Called once when the app starts.
/// `callback` is a C function pointer provided by Swift for port → iOS API calls.
/// `data_dir` is the app container path (UTF-8).
/// `config_json` is runtime config (peers, listen addr, packs).
/// Returns an opaque handle pointer.
#[no_mangle]
pub extern "C" fn soma_init(
    callback: extern "C" fn(*const c_char, *const c_char) -> *mut c_char,
    data_dir: *const c_char,
    config_json: *const c_char,
) -> *mut SomaHandle {
    // 1. Store callback function pointer for port invocations
    // 2. Load pack manifests from app bundle or data_dir
    // 3. Bootstrap runtime with compiled-in ports
    // 4. Start MCP server on TCP (not stdin)
    // 5. Start distributed listener if configured
    // 6. Return boxed handle
}

/// Invoke a port capability. Returns JSON result string (caller must free with soma_free_string).
#[no_mangle]
pub extern "C" fn soma_invoke_port(
    handle: *mut SomaHandle,
    port_id: *const c_char,
    capability: *const c_char,
    input_json: *const c_char,
) -> *mut c_char {
    // Delegates to runtime handle's invoke_port
    // Returns CString::into_raw()
}

/// Get full runtime state as JSON. Caller must free with soma_free_string.
#[no_mangle]
pub extern "C" fn soma_dump_state(handle: *mut SomaHandle) -> *mut c_char {
    // Returns dump_state JSON
}

/// Shutdown the runtime. Persists episodes, closes peers, drops runtime.
#[no_mangle]
pub extern "C" fn soma_shutdown(handle: *mut SomaHandle) {
    // Persist episodes, close peers, drop runtime
    let _ = unsafe { Box::from_raw(handle) };
}

/// Free a string returned by soma_invoke_port or soma_dump_state.
#[no_mangle]
pub extern "C" fn soma_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe { let _ = CString::from_raw(s); }
    }
}
```

**Generated C header (via cbindgen):**

```c
// soma_ios.h — auto-generated, included in Swift bridging header
#ifndef soma_ios_h
#define soma_ios_h

#include <stdint.h>

typedef struct SomaHandle SomaHandle;
typedef char* (*SomaBridgeCallback)(const char* method, const char* args_json);

SomaHandle* soma_init(SomaBridgeCallback callback, const char* data_dir, const char* config_json);
char* soma_invoke_port(SomaHandle* handle, const char* port_id, const char* capability, const char* input_json);
char* soma_dump_state(SomaHandle* handle);
void soma_shutdown(SomaHandle* handle);
void soma_free_string(char* s);

#endif
```

### Swift Bridge Pattern

iOS ports can't call SDK APIs from Rust directly. The bridge uses a C function pointer callback — Rust calls Swift through it, Swift dispatches to iOS frameworks, returns JSON.

```swift
// SomaBridge.swift — the bridge between Rust ports and iOS SDK

// This function is passed to soma_init as the callback.
// Rust calls it whenever a port needs an iOS API.
func somaBridgeCallback(
    method: UnsafePointer<CChar>?,
    argsJson: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>? {
    guard let method = method, let argsJson = argsJson else { return nil }

    let methodStr = String(cString: method)
    let argsStr = String(cString: argsJson)
    let args = try? JSONSerialization.jsonObject(with: argsStr.data(using: .utf8)!) as? [String: Any]

    let result: String
    switch methodStr {
    case "takePhoto":
        result = CameraBridge.takePhoto(args: args ?? [:])
    case "getLocation":
        result = LocationBridge.getLocation()
    case "readMotion":
        result = MotionBridge.readMotion(sensorType: args?["type"] as? String ?? "accelerometer")
    case "getContacts":
        result = ContactsBridge.search(query: args?["query"] as? String ?? "")
    case "sendNotification":
        result = NotificationBridge.send(
            title: args?["title"] as? String ?? "",
            body: args?["body"] as? String ?? ""
        )
    case "queryHealth":
        result = HealthBridge.query(
            dataType: args?["data_type"] as? String ?? "",
            since: args?["since"] as? String
        )
    case "getCalendarEvents":
        result = CalendarBridge.getEvents(
            from: args?["from"] as? String,
            to: args?["to"] as? String
        )
    case "recognizeSpeech":
        result = SpeechBridge.recognize(audioPath: args?["audio_path"] as? String ?? "")
    case "playHaptic":
        result = HapticBridge.play(pattern: args?["pattern"] as? String ?? "impact")
    default:
        result = """
        {"success": false, "error": "unknown method: \(methodStr)"}
        """
    }

    return strdup(result)  // Rust side must free via soma_free_string
}
```

**Individual bridge implementations:**

```swift
// LocationBridge.swift
class LocationBridge: NSObject, CLLocationManagerDelegate {
    private static let shared = LocationBridge()
    private let manager = CLLocationManager()
    private var continuation: CheckedContinuation<CLLocation?, Never>?

    static func getLocation() -> String {
        let location = shared.manager.location  // last known, or request fresh
        guard let loc = location else {
            return """
            {"success": false, "error": "location unavailable"}
            """
        }
        return """
        {"success": true, "lat": \(loc.coordinate.latitude), "lon": \(loc.coordinate.longitude), \
        "altitude_m": \(loc.altitude), "accuracy_m": \(loc.horizontalAccuracy), \
        "speed_mps": \(loc.speed), "heading": \(loc.course), "timestamp": "\(loc.timestamp.ISO8601Format())"}
        """
    }
}

// MotionBridge.swift
class MotionBridge {
    private static let motionManager = CMMotionManager()

    static func readMotion(sensorType: String) -> String {
        switch sensorType {
        case "accelerometer":
            guard let data = motionManager.accelerometerData else {
                return """{"success": false, "error": "accelerometer unavailable"}"""
            }
            return """
            {"success": true, "type": "accelerometer", \
            "x": \(data.acceleration.x), "y": \(data.acceleration.y), "z": \(data.acceleration.z), \
            "timestamp": \(data.timestamp)}
            """
        case "gyroscope":
            guard let data = motionManager.gyroData else {
                return """{"success": false, "error": "gyroscope unavailable"}"""
            }
            return """
            {"success": true, "type": "gyroscope", \
            "x": \(data.rotationRate.x), "y": \(data.rotationRate.y), "z": \(data.rotationRate.z), \
            "timestamp": \(data.timestamp)}
            """
        case "magnetometer":
            guard let data = motionManager.magnetometerData else {
                return """{"success": false, "error": "magnetometer unavailable"}"""
            }
            return """
            {"success": true, "type": "magnetometer", \
            "x": \(data.magneticField.x), "y": \(data.magneticField.y), "z": \(data.magneticField.z), \
            "timestamp": \(data.timestamp)}
            """
        default:
            return """{"success": false, "error": "unknown sensor: \(sensorType)"}"""
        }
    }
}

// HealthBridge.swift
class HealthBridge {
    private static let store = HKHealthStore()

    static func query(dataType: String, since: String?) -> String {
        // Map dataType string to HKQuantityTypeIdentifier
        // Execute HKStatisticsQuery or HKSampleQuery
        // Return JSON with samples
        // Example for step count:
        // {"success": true, "data_type": "stepCount", "value": 8432, "unit": "count",
        //  "start": "2026-04-10T00:00:00Z", "end": "2026-04-10T12:00:00Z"}
        return """
        {"success": true, "data_type": "\(dataType)", "samples": []}
        """
    }
}

// CameraBridge.swift
class CameraBridge {
    static func takePhoto(args: [String: Any]) -> String {
        // AVCaptureSession + AVCapturePhotoOutput
        // Save to app container, return path
        // {"success": true, "path": "/var/mobile/.../photo.jpg", "width": 4032, "height": 3024}
        return """
        {"success": true, "path": "", "width": 0, "height": 0}
        """
    }
}

// NotificationBridge.swift
class NotificationBridge {
    static func send(title: String, body: String) -> String {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default

        let request = UNNotificationRequest(
            identifier: UUID().uuidString,
            content: content,
            trigger: nil  // deliver immediately
        )
        UNUserNotificationCenter.current().add(request)
        return """
        {"success": true, "id": "\(request.identifier)"}
        """
    }
}
```

### iOS App Structure (Xcode Project)

```
SomaRuntime.xcodeproj
  SomaRuntime/
    App.swift                    # @main, SwiftUI app lifecycle
    SomaService.swift            # Runtime lifecycle manager
    SomaBridge.swift             # C callback → iOS API dispatch
    Bridges/
      LocationBridge.swift       # CoreLocation
      MotionBridge.swift         # CoreMotion
      CameraBridge.swift         # AVFoundation
      ContactsBridge.swift       # Contacts.framework
      NotificationBridge.swift   # UserNotifications
      HealthBridge.swift         # HealthKit
      CalendarBridge.swift       # EventKit
      SpeechBridge.swift         # Speech.framework
      HapticBridge.swift         # CoreHaptics
    Views/
      StatusView.swift           # Runtime status, port list
      LogView.swift              # Invocation log
      PeerView.swift             # Connected peers
    SomaRuntime-Bridging-Header.h  # #include "soma_ios.h"
    libsoma_ios.a                # Compiled Rust static library
    soma_ios.h                   # C header (cbindgen output)
    Info.plist
    SomaRuntime.entitlements
```

**SomaService.swift — runtime lifecycle:**

```swift
class SomaService: ObservableObject {
    @Published var isRunning = false
    @Published var portCount = 0
    @Published var peerCount = 0

    private var handle: OpaquePointer?

    func start() {
        let dataDir = FileManager.default
            .urls(for: .documentDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("soma")
            .path

        // Create soma data directory
        try? FileManager.default.createDirectory(
            atPath: dataDir,
            withIntermediateDirectories: true
        )

        let config = buildConfig()

        handle = dataDir.withCString { dir in
            config.withCString { cfg in
                soma_init(somaBridgeCallback, dir, cfg)
            }
        }

        isRunning = handle != nil
    }

    func invokePort(portId: String, capability: String, input: [String: Any]) -> [String: Any]? {
        guard let handle = handle else { return nil }

        let inputJson = String(data: try! JSONSerialization.data(withJSONObject: input), encoding: .utf8)!

        let resultPtr = portId.withCString { pid in
            capability.withCString { cap in
                inputJson.withCString { inp in
                    soma_invoke_port(handle, pid, cap, inp)
                }
            }
        }

        guard let resultPtr = resultPtr else { return nil }
        let result = String(cString: resultPtr)
        soma_free_string(resultPtr)

        return try? JSONSerialization.jsonObject(
            with: result.data(using: .utf8)!
        ) as? [String: Any]
    }

    func dumpState() -> String? {
        guard let handle = handle else { return nil }
        let ptr = soma_dump_state(handle)
        guard let ptr = ptr else { return nil }
        let result = String(cString: ptr)
        soma_free_string(ptr)
        return result
    }

    func stop() {
        guard let handle = handle else { return }
        soma_shutdown(handle)
        self.handle = nil
        isRunning = false
    }

    private func buildConfig() -> String {
        // Runtime config: listen address, peer addresses, pack paths
        return """
        {"listen": "0.0.0.0:9100", "peers": [], "packs": ["soma/packs"]}
        """
    }
}
```

### iOS Background Execution

iOS is restrictive about background execution. Strategies for keeping SOMA alive:

| Strategy | Duration | Use case |
|---|---|---|
| **BGAppRefreshTask** | ~30 seconds | Periodic sync, not suitable for persistent runtime |
| **BGProcessingTask** | Minutes (device decides) | Longer tasks, but not guaranteed or persistent |
| **Location updates** | Indefinite (with `Always` permission) | Continuous background if location port is active |
| **Audio session** | Indefinite (playing/recording) | Hacky, will be rejected by App Store review |
| **Network Extension** | Indefinite | Packet tunnel or content filter — keeps process alive |
| **Live Activity** | 8 hours max, user-visible | Good UX for showing SOMA status |
| **VOIP push** | Wakes app on push, ~30s to process | Good for on-demand invocations from server |

**Recommended approach**: Network Extension (packet tunnel provider) keeps the process alive legitimately. The SOMA TCP listener runs inside the extension. Combine with Live Activity for user-visible status and VOIP push for on-demand wake.

For development/testing: location background mode with `Always` permission is the simplest way to keep the runtime alive.

### iOS Permissions

```
<!-- Info.plist -->
NSCameraUsageDescription         — "SOMA camera port: take photos on request"
NSLocationWhenInUseUsageDescription — "SOMA location port: provide device coordinates"
NSLocationAlwaysUsageDescription — "SOMA location port: background location for peer network"
NSMotionUsageDescription         — "SOMA sensor port: accelerometer, gyroscope, magnetometer"
NSContactsUsageDescription       — "SOMA contacts port: search and read contacts"
NSSpeechRecognitionUsageDescription — "SOMA speech port: transcribe audio"
NSHealthShareUsageDescription    — "SOMA health port: read health data"
NSCalendarsUsageDescription      — "SOMA calendar port: read calendar events"
```

```
<!-- SomaRuntime.entitlements -->
com.apple.developer.networking.networkextension — packet-tunnel-provider
com.apple.developer.healthkit                    — health data access
com.apple.developer.healthkit.background-delivery — background health queries
```

## Ports

### All iOS Ports

| Port | Capabilities | iOS Framework | Notes |
|------|-------------|---------------|-------|
| **camera** | take_photo, record_video, list_photos | AVFoundation, PhotoKit | Full programmatic access |
| **location** | get_location, track_location, geofence | CoreLocation | Background with `Always` permission |
| **sensor** | read_accelerometer, read_gyroscope, read_magnetometer, read_barometer | CoreMotion | No permission needed for motion (since iOS 17 needs permission) |
| **contacts** | list_contacts, search_contacts | Contacts.framework | Read-only by default |
| **notify** | send_notification, schedule_notification, cancel_notification | UserNotifications | Full local notification control |
| **health** | query_steps, query_heart_rate, query_sleep, query_workouts | HealthKit | Read access, rich data types |
| **calendar** | list_events, create_event, list_reminders | EventKit | Read/write with permission |
| **speech** | recognize_audio, recognize_live | Speech.framework | On-device transcription (iOS 17+) |
| **haptic** | play_impact, play_notification, play_pattern | CoreHaptics | Taptic engine control |
| **storage** | list_files, read_file, write_file | FileManager | App container sandbox |
| **nfc** | read_tag, write_tag | CoreNFC | iPhone 7+ |
| **bluetooth** | scan_devices, connect, send_data | CoreBluetooth | BLE, full programmatic access |

Plus the two built-in SOMA ports (http, filesystem) that work without the Swift bridge.

**Not available as silent ports (iOS restriction):**
- ~~sms~~ — `MFMessageComposeViewController` requires user tap per message
- ~~phone~~ — `tel:` URL scheme requires user confirmation
- ~~clipboard~~ — iOS 16+ shows paste permission banner

### Port Manifest (location example)

```json
{
  "id": "soma.ports.location",
  "name": "Location",
  "version": "0.1.0",
  "namespace": "soma.ports.location",
  "description": "Device location via CoreLocation (GPS, WiFi, cellular)",
  "ports": [
    {
      "port_id": "location",
      "name": "location",
      "version": "0.1.0",
      "kind": "sensor",
      "description": "CoreLocation GPS/WiFi/cellular positioning",
      "namespace": "soma.ports.location",
      "trust_level": "verified",
      "capabilities": [
        {
          "capability_id": "get_location",
          "name": "get_location",
          "purpose": "Get current device location (lat, lon, altitude, accuracy, speed, heading)",
          "input_schema": {
            "schema": {
              "type": "object",
              "properties": {
                "accuracy": {
                  "type": "string",
                  "enum": ["best", "nearest_ten_meters", "hundred_meters", "kilometer"],
                  "description": "Desired accuracy level (default: best)"
                }
              }
            }
          },
          "output_schema": {
            "schema": {
              "type": "object",
              "properties": {
                "lat": { "type": "number" },
                "lon": { "type": "number" },
                "altitude_m": { "type": "number" },
                "accuracy_m": { "type": "number" },
                "speed_mps": { "type": "number" },
                "heading": { "type": "number" },
                "timestamp": { "type": "string" }
              }
            }
          },
          "effect_class": "pure_computation",
          "rollback_support": "not_applicable",
          "determinism_class": "non_deterministic",
          "idempotence_class": "idempotent",
          "risk_class": "low",
          "latency_profile": { "expected_latency_ms": 1000, "p95_latency_ms": 5000, "max_latency_ms": 15000 },
          "cost_profile": {
            "cpu_cost_class": "low",
            "memory_cost_class": "negligible",
            "io_cost_class": "negligible",
            "network_cost_class": "negligible",
            "energy_cost_class": "medium"
          },
          "remote_exposable": true
        },
        {
          "capability_id": "track_location",
          "name": "track_location",
          "purpose": "Start continuous location tracking (updates via observation stream)",
          "input_schema": {
            "schema": {
              "type": "object",
              "properties": {
                "interval_seconds": { "type": "integer", "description": "Minimum time between updates" },
                "distance_meters": { "type": "number", "description": "Minimum distance between updates" }
              }
            }
          },
          "output_schema": {
            "schema": {
              "type": "object",
              "properties": {
                "tracking": { "type": "boolean" },
                "session_id": { "type": "string" }
              }
            }
          },
          "effect_class": "local_state_mutation",
          "rollback_support": "supports_rollback",
          "determinism_class": "non_deterministic",
          "idempotence_class": "idempotent",
          "risk_class": "low",
          "latency_profile": { "expected_latency_ms": 100, "p95_latency_ms": 500, "max_latency_ms": 2000 },
          "cost_profile": {
            "cpu_cost_class": "low",
            "memory_cost_class": "negligible",
            "io_cost_class": "negligible",
            "network_cost_class": "negligible",
            "energy_cost_class": "high"
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

## Network Topology

### Standalone Mode

iPhone runs SOMA with a TCP listener. An MCP client (or LLM with network MCP transport) connects directly to the phone.

```
LLM ──► TCP :9100 ──► iPhone SOMA ──► CoreLocation ──► GPS coordinates
```

Requires the phone to be reachable (same WiFi, or Tailscale/WireGuard tunnel).

### Peer Mode (s2s)

iPhone SOMA connects to a server SOMA instance as a peer. LLM talks to the server. Server delegates sensor/perception operations to the phone.

```
                        TCP 9100
┌─────────────┐     ◄──────────────     ┌─────────────────┐
│ Server SOMA │                         │ iPhone SOMA     │
│  postgres   │     s2s wire proto      │  camera         │
│  s3         │    ────────────────►    │  location       │
│  smtp       │                         │  sensors        │
│  --listen   │                         │  health         │
└──────┬──────┘                         │  contacts       │
       │                                │  --peer server  │
  LLM (Claude)                          └─────────────────┘
  via MCP stdin
```

### Cross-Platform Mesh

```
                   Server SOMA (:9100)
                  /        |         \
           android-0    iphone-0    rpi-0
           (sms,cam)   (health,loc) (gpio,temp)
```

Android handles SMS and calls (programmatic access). iPhone handles health data and precise location (HealthKit, CoreLocation). Each device contributes what its OS allows. The LLM doesn't know which OS the peer runs — it sees ports and capabilities.

## Cross-Compilation (Proven)

soma-next cross-compiles for iOS with **zero code changes** beyond the same reqwest rustls fix needed for Android.

### What was required

Same two changes as Android — already merged:

1. `reqwest`: `default-features = false, features = ["json", "blocking", "rustls-tls"]` (drops openssl-sys)
2. `rustls`: `features = ["ring"]` (explicit crypto provider)

No iOS-specific changes. The same Cargo.toml builds for macOS, Android, and iOS.

### Toolchain setup

```bash
# Add iOS targets
rustup target add aarch64-apple-ios        # physical device
rustup target add aarch64-apple-ios-sim    # simulator (Apple Silicon Mac)
```

No NDK needed — Xcode provides the iOS SDK and linker. The Rust toolchain finds it automatically via `xcrun`.

### Build the static library

```bash
cd soma-next

# For physical device
cargo build --target aarch64-apple-ios --release

# For simulator (Apple Silicon)
cargo build --target aarch64-apple-ios-sim --release
```

### Build result

```
File:   target/aarch64-apple-ios/release/soma
Format: Mach-O 64-bit executable arm64
Size:   9 MB
```

This is the full soma-next runtime — same as the macOS and Android builds: control loop, memory system, MCP server (29 tools), distributed transport, built-in ports, policy engine, Ed25519 auth, rate limiting.

No cargo-ndk needed. No NDK. Just `rustup target add` + `cargo build --target`. Simpler than Android.

### Generate C header

```bash
# Install cbindgen
cargo install cbindgen

# Generate header from Rust FFI functions
cbindgen --config cbindgen.toml --crate soma-ios --output soma_ios.h
```

### Xcode integration

1. Add `libsoma_ios.a` to the Xcode project (drag into Frameworks)
2. Add `soma_ios.h` to the project
3. Create `SomaRuntime-Bridging-Header.h` with `#include "soma_ios.h"`
4. Set bridging header path in Build Settings → Swift Compiler → Objective-C Bridging Header
5. Add `-lsoma_ios` to Other Linker Flags
6. Add `libresolv.tbd` to linked frameworks (for DNS resolution)

## Storage

iOS apps have a sandboxed container. SOMA stores everything there:

```
/var/mobile/Containers/Data/Application/<UUID>/Documents/
  soma/
    episodes/          # Episode ring buffer persistence
    schemas/           # Induced schemas
    routines/          # Compiled routines
    checkpoints/       # Session checkpoints
    packs/             # Pack manifests (bundled in app, copied on first launch)
    config.toml        # Runtime config
```

Standard iOS app sandbox. No jailbreak needed. Survives app updates (Documents directory is preserved).

## Security

**Network layer:**
- TLS required for all remote connections (rustls, already in soma-next)
- Ed25519 peer authentication (already in soma-next distributed/auth.rs)
- Rate limiting per peer (already in soma-next distributed/rate_limit.rs)
- App Transport Security (ATS) is satisfied by rustls TLS

**Port layer:**
- SOMA policy engine enforces risk budgets per session
- The app UI can intercept invocations and show a confirmation dialog
- HealthKit data especially — policy should require explicit user approval

**iOS layer:**
- Runtime permissions granted explicitly by the user via system dialogs
- Background activity shown in status bar (location arrow, etc.)
- App Review: must justify each permission with usage descriptions in Info.plist
- No private API usage — all ports use public iOS SDK frameworks

**Trust model:**
- The phone owner controls which peers can connect (peer allowlist in config)
- The phone owner controls which ports are active (toggle per port in UI)
- The phone owner sees all invocations in the app log

## What This Proves

1. **SOMA runs on iOS.** The same Rust runtime that runs on macOS and Android compiles for iOS with zero code changes. 9MB static library, all 29 MCP tools, full memory system.

2. **iOS as a perception peer.** iPhone has the best sensor hardware (LiDAR, U1 chip, barometer, HealthKit integration). It's the ideal perception node in a SOMA mesh — where Android handles actuation (SMS, calls), iPhone handles sensing (health, precise location, spatial awareness).

3. **Cross-platform mesh.** Android SOMA + iOS SOMA + server SOMA, all connected via s2s. Each contributes capabilities its OS allows. The LLM sees a unified body across platforms. Same wire protocol, same port interface, different physical capabilities.

4. **The port abstraction holds across platforms.** `invoke_port("location", "get_location", {})` returns the same JSON schema whether the peer is an iPhone (CoreLocation), Android (FusedLocationProvider), or a Raspberry Pi (gpsd). The brain doesn't know or care about the platform.

## Milestones

| # | Milestone | What | Validates | Status |
|---|-----------|------|-----------|--------|
| 1 | **Rust compiles for iOS** | `cargo build --target aarch64-apple-ios --release` produces 9MB Mach-O binary. Zero code changes. | Cross-compilation toolchain works | **DONE** |
| 2 | **Swift FFI bridge works** | Swift calls soma_init, Rust calls Swift callback, round-trip JSON | C FFI bridge pattern is viable | |
| 3 | **Location port returns coordinates** | LLM → invoke_port("location", "get_location") → real GPS coordinates | First sensor port works | |
| 4 | **Background execution survives** | SOMA stays alive via Network Extension for 24h | iOS background model is handled | |
| 5 | **s2s peer connection** | iPhone SOMA connects to server SOMA, invoke_remote_skill works | Distributed body works on iOS | |
| 6 | **All ports operational** | Camera, location, sensors, contacts, notify, health, calendar, speech, haptic | Full device access through SOMA | |
| 7 | **Cross-platform mesh** | Android + iPhone + server, LLM routes to correct device by capability | Heterogeneous distributed body | |
| 8 | **Routine transfer** | Server compiles a routine, transfers it to iPhone, phone executes autonomously | iPhone learns from server | |
