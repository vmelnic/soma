#!/usr/bin/env python3
"""Exercise the SOMA leaf wire protocol over UART0.

Sends Ping, ListCapabilities, then a battery of InvokeSkill calls
covering every effect class (read_only / state_mutation / external_effect)
across multiple ports. Then transfers a multi-step routine, invokes it,
and removes it.

Used by scripts/test.sh — pass the serial port and the test_pin (the gpio
pin claimed by the chip module's gpio port) as positional args. Add --wifi
to also exercise wifi.status and wifi.scan against the actual radio.

    wire-test.py /dev/cu.usbserial-1120 15              # ESP32-S3, GPIO15
    wire-test.py /dev/cu.usbserial-0001 13              # ESP32 LX6, GPIO13
    wire-test.py /dev/cu.usbserial-1120 15 --wifi       # + radio scan

Exits 0 if all calls succeed, 1 otherwise.
"""

import json
import struct
import sys
import time

import serial


BAUD = 115200


def encode_frame(msg):
    body = json.dumps(msg).encode("utf-8")
    return struct.pack(">I", len(body)) + body


def read_frame(ser, timeout=4.0):
    """Scan stream for a 4-byte length prefix followed by valid JSON.

    The leaf firmware shares UART0 with esp-println, so the host parser
    must skip log lines until it finds bytes that decode as a valid frame.
    """
    buf = bytearray()
    end = time.time() + timeout
    while time.time() < end:
        chunk = ser.read(4096)
        if chunk:
            buf.extend(chunk)
        i = 0
        while i + 4 <= len(buf):
            length = struct.unpack(">I", bytes(buf[i:i + 4]))[0]
            if 0 < length < 16384 and i + 4 + length <= len(buf):
                body = bytes(buf[i + 4:i + 4 + length])
                try:
                    return json.loads(body)
                except json.JSONDecodeError:
                    pass
            i += 1
        time.sleep(0.01)
    return None


class TestRunner:
    def __init__(self, ser):
        self.ser = ser
        self.passed = 0
        self.failed = 0

    def call(self, label, msg, expect_type, expect_success=None):
        self.ser.write(encode_frame(msg))
        self.ser.flush()
        r = read_frame(self.ser)
        ok = False
        detail = ""
        if r is None:
            detail = "TIMEOUT"
        elif r.get("type") != expect_type:
            detail = f"wrong type: {r.get('type')!r}"
        elif expect_success is not None and expect_type == "skill_result":
            actual = r.get("response", {}).get("success")
            if actual != expect_success:
                detail = f"success={actual}, expected {expect_success}"
            else:
                ok = True
                if expect_type == "skill_result":
                    sr = r["response"].get("structured_result")
                    detail = f"-> {sr}"
        else:
            ok = True
            if expect_type == "capabilities":
                detail = (f"primitives={len(r.get('primitives', []))}, "
                          f"routines={len(r.get('routines', []))}")
            elif expect_type == "pong":
                detail = f"nonce={r.get('nonce')}, load={r.get('load')}"
            elif expect_type == "routine_stored":
                detail = (f"routine_id={r.get('routine_id')}, "
                          f"step_count={r.get('step_count')}")
            elif expect_type == "routine_removed":
                detail = f"routine_id={r.get('routine_id')}"

        marker = "PASS" if ok else "FAIL"
        print(f"  [{marker}] {label}  {detail}")
        if ok:
            self.passed += 1
        else:
            self.failed += 1
        return r


def main():
    if len(sys.argv) < 3:
        print("usage: wire-test.py <serial-port> <gpio-test-pin> [--wifi]",
              file=sys.stderr)
        return 2
    port = sys.argv[1]
    test_pin = int(sys.argv[2])
    wifi_variant = "--wifi" in sys.argv[3:]

    print(f"[test] connecting to {port} at {BAUD} baud")
    ser = serial.Serial(port, BAUD, timeout=0.05)

    # Drain the boot banner. ESP32 LX6 takes ~2-3s to finish printing its
    # 30-primitive self-model + cross-port self-test output; ESP32-S3 is
    # faster. Wait for the "Body alive" marker rather than a fixed sleep
    # so the same code handles both chips without flakiness. Cap the wait
    # at 10s — if the marker doesn't arrive the chip never booted.
    boot_deadline = time.time() + 10
    boot_buf = bytearray()
    while time.time() < boot_deadline:
        chunk = ser.read(4096)
        if chunk:
            boot_buf.extend(chunk)
            if b"Body alive" in boot_buf:
                # Give the chip a beat to finish printing the remaining
                # banner lines after "Body alive".
                time.sleep(0.3)
                # Drain whatever's still buffered from the banner print.
                ser.read(8192)
                break
        else:
            time.sleep(0.02)
    else:
        print("[test] warning: 'Body alive' marker not seen in 10s — proceeding anyway",
              file=sys.stderr)

    t = TestRunner(ser)

    # NOTE: wifi tests run FIRST when the --wifi variant is requested.
    # esp-wifi 0.12 on ESP32 LX6 has a known interaction where wifi.scan
    # crashes with an illegal-instruction exception after heavy storage
    # flash writes have happened in the same boot cycle. S3 is unaffected.
    # Running the scan first — while the radio's state is clean — avoids
    # the crash and proves the radio is alive on both chips. The 14 wire
    # protocol tests still run afterwards to prove they don't regress.
    if wifi_variant:
        print()
        print("=== Wifi primitives (real radio — run first to avoid "
              "ESP32 LX6 state interaction with storage writes) ===")
        t.call("wifi.status",
               {"type": "invoke_skill", "peer_id": "test",
                "skill_id": "wifi.status", "input": {}},
               expect_type="skill_result", expect_success=True)

        print("  (scanning — 15 s cap)")
        ser.reset_input_buffer()
        ser.write(encode_frame({
            "type": "invoke_skill", "peer_id": "test",
            "skill_id": "wifi.scan", "input": {},
        }))
        ser.flush()
        raw = bytearray()
        scan_deadline = time.time() + 15.0
        while time.time() < scan_deadline:
            chunk = ser.read(4096)
            if chunk:
                raw.extend(chunk)

        r = None
        i = 0
        while i + 4 <= len(raw):
            length = struct.unpack(">I", bytes(raw[i:i + 4]))[0]
            if 0 < length < 16384 and i + 4 + length <= len(raw):
                try:
                    r = json.loads(bytes(raw[i + 4:i + 4 + length]))
                    break
                except json.JSONDecodeError:
                    pass
            i += 1

        if r and r.get("type") == "skill_result" and r.get("response", {}).get("success"):
            networks = r["response"]["structured_result"].get("networks", [])
            print(f"  [PASS] wifi.scan  {len(networks)} network(s) found")
            for n in networks[:5]:
                print(f"         {str(n.get('ssid'))[:30]:30s}  "
                      f"ch={n.get('channel'):2}  "
                      f"rssi={n.get('rssi'):4}  {n.get('security', '')}")
            if len(networks) > 5:
                print(f"         ... and {len(networks) - 5} more")
            t.passed += 1
        else:
            print(f"  [FAIL] wifi.scan  raw={len(raw)} bytes")
            if raw:
                snippet = bytes(raw[:200]).decode('utf-8', errors='replace')
                print(f"    {snippet!r}")
            t.failed += 1

    print()
    print("=== Liveness ===")
    t.call("Ping",
           {"type": "ping", "nonce": 99},
           expect_type="pong")

    print()
    print("=== Self-model ===")
    t.call("ListCapabilities",
           {"type": "list_capabilities"},
           expect_type="capabilities")

    print()
    print(f"=== GPIO round-trip on pin {test_pin} ===")
    t.call(f"gpio.write pin={test_pin} value=true",
           {"type": "invoke_skill", "peer_id": "test",
            "skill_id": "gpio.write",
            "input": {"pin": test_pin, "value": True}},
           expect_type="skill_result", expect_success=True)
    t.call(f"gpio.read pin={test_pin}",
           {"type": "invoke_skill", "peer_id": "test",
            "skill_id": "gpio.read",
            "input": {"pin": test_pin}},
           expect_type="skill_result", expect_success=True)
    t.call(f"gpio.write pin={test_pin} value=false",
           {"type": "invoke_skill", "peer_id": "test",
            "skill_id": "gpio.write",
            "input": {"pin": test_pin, "value": False}},
           expect_type="skill_result", expect_success=True)
    t.call(f"gpio.toggle pin={test_pin}",
           {"type": "invoke_skill", "peer_id": "test",
            "skill_id": "gpio.toggle",
            "input": {"pin": test_pin}},
           expect_type="skill_result", expect_success=True)

    print()
    print("=== Delay ===")
    t.call("delay.ms 50",
           {"type": "invoke_skill", "peer_id": "test",
            "skill_id": "delay.ms", "input": {"ms": 50}},
           expect_type="skill_result", expect_success=True)

    print()
    print("=== Sensor (thermistor) ===")
    t.call("thermistor.read_temp",
           {"type": "invoke_skill", "peer_id": "test",
            "skill_id": "thermistor.read_temp", "input": {"channel": 0}},
           expect_type="skill_result", expect_success=True)

    print()
    print("=== Persistent storage ===")
    t.call("storage.set",
           {"type": "invoke_skill", "peer_id": "test",
            "skill_id": "storage.set",
            "input": {"key": "wire_test_key",
                      "value": "wire-test.py was here"}},
           expect_type="skill_result", expect_success=True)
    t.call("storage.get",
           {"type": "invoke_skill", "peer_id": "test",
            "skill_id": "storage.get",
            "input": {"key": "wire_test_key"}},
           expect_type="skill_result", expect_success=True)
    t.call("storage.list",
           {"type": "invoke_skill", "peer_id": "test",
            "skill_id": "storage.list", "input": {}},
           expect_type="skill_result", expect_success=True)

    print()
    print("=== Multi-step routine: transfer + invoke + remove ===")
    routine = {
        "routine_id": "wire_test_blink",
        "description": "Toggle gpio test pin three times with 100ms delays",
        "steps": [
            {"skill_id": "gpio.toggle", "input": {"pin": test_pin}},
            {"skill_id": "delay.ms",    "input": {"ms": 100}},
            {"skill_id": "gpio.toggle", "input": {"pin": test_pin}},
            {"skill_id": "delay.ms",    "input": {"ms": 100}},
            {"skill_id": "gpio.toggle", "input": {"pin": test_pin}},
            {"skill_id": "delay.ms",    "input": {"ms": 100}},
            {"skill_id": "gpio.toggle", "input": {"pin": test_pin}},
        ],
    }
    t.call("TransferRoutine wire_test_blink",
           {"type": "transfer_routine", "routine": routine},
           expect_type="routine_stored")
    t.call("invoke wire_test_blink",
           {"type": "invoke_skill", "peer_id": "test",
            "skill_id": "wire_test_blink", "input": {}},
           expect_type="skill_result", expect_success=True)
    t.call("RemoveRoutine wire_test_blink",
           {"type": "remove_routine", "routine_id": "wire_test_blink"},
           expect_type="routine_removed")

    ser.close()

    print()
    print("=" * 60)
    print(f"[test] passed: {t.passed}, failed: {t.failed}")
    print("=" * 60)
    return 0 if t.failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
