#!/usr/bin/env python3
"""List WiFi networks visible to a flashed SOMA ESP32 leaf.

Drains the boot banner, sends wifi.scan over UART0, and pretty-prints the
networks returned by esp-wifi 0.12 (SSID, channel, RSSI, security).

Usage:
    wifi-scan.py <serial-port>

Example:
    wifi-scan.py /dev/cu.usbserial-0001      # ESP32 WROOM-32D
    wifi-scan.py /dev/cu.usbserial-1120      # ESP32-S3 Sunton 1732S019

Exit codes:
    0 — at least one network visible
    1 — scan returned zero networks
    2 — wifi.scan failed (TIMEOUT or error response)
    3 — usage / port error

IMPORTANT: On ESP32 LX6, run this IMMEDIATELY after boot (before any storage
writes in the same boot cycle). esp-wifi 0.12 has a known bug where
wifi.scan crashes with an illegal-instruction exception if called after
SPI flash writes. ESP32-S3 is unaffected.
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
    """Trigger the chip's reset via DTR/RTS toggle — same sequence espflash
    uses. Must be done AFTER opening the port so we're listening when the
    boot output arrives. DTR=false + RTS=true asserts EN low (reset),
    RTS=false releases it and the chip starts booting."""
    ser.dtr = False
    ser.rts = True
    time.sleep(0.1)
    ser.rts = False
    time.sleep(0.05)


def wait_for_boot(ser):
    print("[wifi-scan] resetting chip and waiting for boot banner...",
          file=sys.stderr)
    reset_chip(ser)
    deadline = time.time() + 10
    seen = bytearray()
    while time.time() < deadline:
        chunk = ser.read(4096)
        if chunk:
            seen.extend(chunk)
            if b"Body alive" in seen:
                time.sleep(0.3)
                ser.read(8192)
                return True
        else:
            time.sleep(0.05)
    print("[wifi-scan] warning: 'Body alive' not seen in 10s — proceeding",
          file=sys.stderr)
    return False


def main():
    if len(sys.argv) < 2:
        print("usage: wifi-scan.py <serial-port>", file=sys.stderr)
        return 3

    port = sys.argv[1]

    try:
        ser = serial.Serial(port, BAUD, timeout=0.05)
    except serial.SerialException as e:
        print(f"[wifi-scan] cannot open {port}: {e}", file=sys.stderr)
        return 3

    wait_for_boot(ser)

    print("[wifi-scan] sending wifi.scan (up to 15 s)...", file=sys.stderr)
    ser.write(encode_frame({
        "type": "invoke_skill",
        "peer_id": "host",
        "skill_id": "wifi.scan",
        "input": {},
    }))
    ser.flush()

    r = read_frame(ser, timeout=15.0)
    ser.close()

    if r is None:
        print("[wifi-scan] TIMEOUT — no response from chip", file=sys.stderr)
        return 2

    if r.get("type") != "skill_result":
        print(f"[wifi-scan] unexpected response: {json.dumps(r)}",
              file=sys.stderr)
        return 2

    resp = r.get("response", {})
    if not resp.get("success"):
        print(f"[wifi-scan] scan failed: "
              f"{resp.get('failure_message', 'unknown error')}",
              file=sys.stderr)
        return 2

    networks = resp.get("structured_result", {}).get("networks", [])
    if not networks:
        print("[wifi-scan] scan succeeded but returned 0 networks")
        print("           (antenna placement, shielding, or weak signal?)")
        return 1

    # Sort strongest first.
    networks.sort(key=lambda n: n.get("rssi", -999), reverse=True)

    print()
    print(f"Found {len(networks)} WiFi network(s):")
    print()
    print(f"  {'SSID':32}  {'Ch':>4}  {'RSSI':>6}  Security")
    print(f"  {'-' * 32}  {'-' * 4}  {'-' * 6}  {'-' * 30}")
    for n in networks:
        ssid = str(n.get("ssid") or "(hidden)")[:32]
        ch = n.get("channel", "?")
        rssi = n.get("rssi", "?")
        sec = str(n.get("security", ""))
        # Strip esp-wifi's Option<T> debug formatting noise
        if sec.startswith("Some(") and sec.endswith(")"):
            sec = sec[5:-1]
        print(f"  {ssid:32}  {ch:>4}  {rssi:>4} dBm  {sec}")
    print()

    return 0


if __name__ == "__main__":
    sys.exit(main())
