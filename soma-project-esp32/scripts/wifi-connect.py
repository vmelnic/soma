#!/usr/bin/env python3
"""Connect a flashed SOMA ESP32 leaf to a WiFi network over UART0.

Walks the full handshake:
    1. Drain boot banner (wait for "Body alive" marker)
    2. (optional) wifi.scan  — list visible APs so you can pick one
    3. wifi.configure {ssid, password}  — stores creds in SPI flash
                                           and starts radio association
    4. Poll wifi.status until connected + IP assigned (or timeout)

Usage:
    wifi-connect.py <serial-port> <ssid> <password> [--scan]

Example:
    wifi-connect.py /dev/cu.usbserial-0001 MyNetwork MySecret
    wifi-connect.py /dev/cu.usbserial-1120 MyNetwork MySecret --scan

Exit codes:
    0 — connected, IP assigned
    1 — configure succeeded but DHCP didn't finish within timeout
    2 — configure failed (wrong password? network not reachable?)
    3 — usage error / port error
"""

import json
import struct
import sys
import time

import serial


BAUD = 115200
STATUS_POLL_TIMEOUT_SEC = 45.0
STATUS_POLL_INTERVAL_SEC = 1.5


def encode_frame(msg):
    body = json.dumps(msg).encode("utf-8")
    return struct.pack(">I", len(body)) + body


def read_frame(ser, timeout):
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
        time.sleep(0.02)
    return None


def reset_chip(ser):
    """Trigger the chip's reset via DTR/RTS. Must be called after opening
    the port so we catch the boot banner."""
    ser.dtr = False
    ser.rts = True
    time.sleep(0.1)
    ser.rts = False
    time.sleep(0.05)


def wait_for_boot(ser):
    """Reset the chip, then drain the boot banner until we see 'Body alive'."""
    print("[wifi] resetting chip and waiting for boot banner...")
    reset_chip(ser)
    deadline = time.time() + 10
    seen = bytearray()
    while time.time() < deadline:
        chunk = ser.read(4096)
        if chunk:
            seen.extend(chunk)
            if b"Body alive" in seen:
                time.sleep(0.3)   # drain the trailing "===" lines
                ser.read(8192)
                return True
        else:
            time.sleep(0.05)
    print("[wifi] warning: 'Body alive' not seen in 10s — proceeding anyway")
    return False


def call(ser, msg, timeout=8.0):
    ser.write(encode_frame(msg))
    ser.flush()
    return read_frame(ser, timeout=timeout)


def main():
    if len(sys.argv) < 4:
        print("usage: wifi-connect.py <serial-port> <ssid> <password> [--scan]",
              file=sys.stderr)
        return 3

    port = sys.argv[1]
    ssid = sys.argv[2]
    password = sys.argv[3]
    do_scan = "--scan" in sys.argv[4:]

    try:
        ser = serial.Serial(port, BAUD, timeout=0.05)
    except serial.SerialException as e:
        print(f"[wifi] cannot open {port}: {e}", file=sys.stderr)
        return 3

    wait_for_boot(ser)

    # Optional scan first — useful for picking the SSID, and also primes the
    # radio on ESP32 LX6 where the first wifi operation needs to happen before
    # any SPI flash writes (wifi.configure writes to flash).
    if do_scan:
        print("[wifi] scanning (up to 15 s)...")
        r = call(ser, {
            "type": "invoke_skill",
            "peer_id": "host",
            "skill_id": "wifi.scan",
            "input": {},
        }, timeout=15.0)
        if r and r.get("type") == "skill_result" and r["response"].get("success"):
            networks = r["response"]["structured_result"].get("networks", [])
            print(f"[wifi] found {len(networks)} network(s):")
            for n in networks:
                print(f"       {str(n.get('ssid'))[:30]:30s}  "
                      f"ch={n.get('channel'):2}  "
                      f"rssi={n.get('rssi'):4}  {n.get('security', '')}")
            print()
        else:
            print(f"[wifi] scan failed or timed out — response: {r}")

    # Configure + connect. wifi.configure:
    #   - persists ssid and password to FlashKvStore (survives reboot)
    #   - starts the radio if not already started
    #   - calls the esp-wifi connect() — returns Ok once associated or Err
    #     with the failure reason (AuthFailed / NoApFound / HardwareError)
    print(f"[wifi] calling wifi.configure for ssid={ssid!r}...")
    r = call(ser, {
        "type": "invoke_skill",
        "peer_id": "host",
        "skill_id": "wifi.configure",
        "input": {"ssid": ssid, "password": password},
    }, timeout=30.0)

    if r is None:
        print("[wifi] wifi.configure: TIMEOUT — chip did not respond")
        ser.close()
        return 2

    if r.get("type") != "skill_result":
        print(f"[wifi] wifi.configure: unexpected response: {json.dumps(r)}")
        ser.close()
        return 2

    resp = r.get("response", {})
    if not resp.get("success"):
        print(f"[wifi] wifi.configure FAILED: "
              f"{resp.get('failure_message', 'unknown error')}")
        ser.close()
        return 2

    print("[wifi] wifi.configure returned OK — credentials stored in flash")

    # CRITICAL: smoltcp's DhcpSocket polls the link state at boot. If it
    # sees link=Down at that first poll, it gives up on DHCP and never
    # retries even after the link comes up later. wifi.configure brings
    # the link up, but too late. Fix: reset the chip NOW so the next boot
    # auto-connects from stored credentials BEFORE smoltcp starts polling.
    print("[wifi] resetting chip so auto-connect fires before smoltcp starts...")
    reset_chip(ser)

    # Wait for the boot banner + auto-connect log lines to finish.
    deadline = time.time() + 10
    seen = bytearray()
    while time.time() < deadline:
        chunk = ser.read(4096)
        if chunk:
            seen.extend(chunk)
            if b"Body alive" in seen:
                time.sleep(0.3)
                ser.read(8192)
                break
        else:
            time.sleep(0.05)

    # Print any auto-connect status lines from boot
    boot_text = seen.decode('utf-8', errors='replace')
    for line in boot_text.split('\n'):
        if 'auto-connect' in line or '[net]' in line:
            print(f"       {line.strip()}")

    print("[wifi] polling wifi.status until DHCP assigns an IP...")

    # Poll wifi.status periodically. The IP comes from smoltcp's DhcpSocket
    # running inside run_dual_transport's poll loop. It can take 5-20 seconds
    # depending on the router.
    start = time.time()
    last_printed_state = None
    while time.time() - start < STATUS_POLL_TIMEOUT_SEC:
        r = call(ser, {
            "type": "invoke_skill",
            "peer_id": "host",
            "skill_id": "wifi.status",
            "input": {},
        }, timeout=5.0)

        if r and r.get("type") == "skill_result" and r["response"].get("success"):
            state = r["response"]["structured_result"]
            connected = state.get("connected")
            ip = state.get("ip")

            state_str = f"connected={connected}, ip={ip}"
            if state_str != last_printed_state:
                elapsed = int(time.time() - start)
                print(f"       [{elapsed:2d}s] {state_str}")
                last_printed_state = state_str

            if connected and ip:
                rssi = state.get("rssi")
                mac = state.get("mac")
                print()
                print("[wifi] ✓ CONNECTED")
                print(f"       SSID: {state.get('ssid') or ssid}")
                print(f"       IP:   {ip}")
                if rssi is not None:
                    print(f"       RSSI: {rssi} dBm")
                if mac:
                    print(f"       MAC:  {mac}")
                print()
                print("[wifi] credentials persisted in SPI flash — "
                      "chip will auto-reconnect on next boot")
                print(f"[wifi] TCP listener is now accepting on {ip}:9100")
                ser.close()
                return 0

        time.sleep(STATUS_POLL_INTERVAL_SEC)

    print(f"[wifi] ✗ DHCP did not complete in {STATUS_POLL_TIMEOUT_SEC}s")
    print("       wifi.configure succeeded, but the chip never got an IP.")
    print("       Common causes: wrong password (router dropped us silently),")
    print("       DHCP server unreachable, or MAC blocked.")
    ser.close()
    return 1


if __name__ == "__main__":
    sys.exit(main())
