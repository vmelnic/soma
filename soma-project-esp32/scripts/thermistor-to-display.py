#!/usr/bin/env python3
"""Periodically read the thermistor and draw the value on the OLED.

This script is the brain side of the "read a sensor, show it on a
screen" loop. It connects directly to the ESP32 leaf over TCP (port
9100, same wire protocol the soma-next MCP server uses internally)
and invokes two skills per tick:

    1. thermistor.read_temp {channel: 0}      -> {temp_c: float}
    2. display.draw_text   {line: 0, text: "Temp: 23.50 C"}
    3. display.draw_text   {line: 1, text: "uptime Xs"}
    4. display.draw_text   {line: 2, text: "soma leaf"}

It uses the same direct-TCP path as the end-to-end tests because
it's the shortest, fastest way to prove the feature works. The same
sequence is available via `soma-next --mcp --discover-lan`:

    invoke_remote_skill {peer_id: "lan-soma-<chip>-<mac>",
                         skill_id: "thermistor.read_temp",
                         input: {"channel": 0}}
    invoke_remote_skill {peer_id: ...,
                         skill_id: "display.draw_text",
                         input: {"line": 0, "text": "Temp: ..."}}

so an LLM-driven brain can run the same loop without any code changes
in the firmware.

Usage:
    ./scripts/thermistor-to-display.py                 # defaults: 192.168.100.203, 5s
    ./scripts/thermistor-to-display.py --host 10.0.0.5
    ./scripts/thermistor-to-display.py --interval 2    # tick every 2 seconds
    ./scripts/thermistor-to-display.py --ticks 6       # run 6 ticks then exit
    ./scripts/thermistor-to-display.py --discover      # mDNS browse for a leaf

The script prints each tick's sensor reading + round-trip time to
stdout so you can see what's happening without having to watch the
OLED. Exits cleanly on Ctrl-C.
"""

import argparse
import json
import socket
import struct
import sys
import time


LEAF_TCP_PORT = 9100
DEFAULT_HOST = "192.168.100.203"
MDNS_SERVICE = "_soma._tcp.local."


def encode_frame(msg: dict) -> bytes:
    body = json.dumps(msg).encode("utf-8")
    return struct.pack(">I", len(body)) + body


def recv_exact(sock: socket.socket, n: int, deadline: float) -> bytes:
    buf = bytearray()
    while len(buf) < n:
        sock.settimeout(max(0.01, deadline - time.time()))
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("leaf closed connection")
        buf.extend(chunk)
    return bytes(buf)


def read_frame(sock: socket.socket, timeout: float = 5.0) -> dict:
    deadline = time.time() + timeout
    header = recv_exact(sock, 4, deadline)
    length = struct.unpack(">I", header)[0]
    if length == 0 or length > 16 * 1024 * 1024:
        raise ValueError(f"unreasonable frame length: {length}")
    body = recv_exact(sock, length, deadline)
    return json.loads(body)


def invoke_skill(sock: socket.socket, skill_id: str, input_payload: dict) -> dict:
    # `peer_id` is required by the leaf's TransportMessage::InvokeSkill
    # variant — it's the brain's self-identifier, echoed back in the
    # response for correlation. For a direct-TCP script like this we
    # can use any stable string; soma-next uses its own node id.
    msg = {
        "type": "invoke_skill",
        "peer_id": "thermistor-to-display",
        "skill_id": skill_id,
        "input": input_payload,
    }
    sock.sendall(encode_frame(msg))
    return read_frame(sock)


def tick(sock: socket.socket, tick_num: int) -> bool:
    """Run one loop iteration. Returns True on success."""
    t0 = time.time()

    # 1. Read the thermistor.
    temp_resp = invoke_skill(sock, "thermistor.read_temp", {"channel": 0})
    if temp_resp.get("type") != "skill_result":
        print(f"[tick {tick_num}] unexpected response: {temp_resp}", file=sys.stderr)
        return False
    result = temp_resp.get("response", {})
    if not result.get("success"):
        print(f"[tick {tick_num}] thermistor.read_temp failed: {result.get('failure_message')}", file=sys.stderr)
        return False
    obs = result.get("structured_result", {})
    temp_c = obs.get("temp_c")
    if temp_c is None:
        print(f"[tick {tick_num}] missing temp_c in observation: {obs}", file=sys.stderr)
        return False

    # 2. Draw the temperature on line 0.
    line0 = f"Temp: {temp_c:5.2f} C"
    draw_resp = invoke_skill(
        sock, "display.draw_text", {"line": 0, "text": line0}
    )
    if draw_resp.get("type") != "skill_result" or not draw_resp.get("response", {}).get("success"):
        print(f"[tick {tick_num}] display.draw_text line 0 failed: {draw_resp}", file=sys.stderr)
        return False

    # 3. Draw the tick + elapsed seconds on line 1.
    elapsed = int((time.time() - tick_num * 0) - 0)  # wall-clock not needed here
    line1 = f"tick #{tick_num}"
    invoke_skill(sock, "display.draw_text", {"line": 1, "text": line1})

    # 4. Label on line 2 so the panel has a 3-line readout.
    invoke_skill(sock, "display.draw_text", {"line": 2, "text": "soma leaf"})

    dt_ms = (time.time() - t0) * 1000
    print(
        f"[tick {tick_num:3d}] temp_c={temp_c:5.2f}  "
        f"roundtrip={dt_ms:5.0f}ms  -> displayed",
        flush=True,
    )
    return True


def discover_leaf_host(timeout: float = 8.0) -> str:
    """Browse mDNS for _soma._tcp.local. and return the first IPv4 found."""
    try:
        from zeroconf import Zeroconf, ServiceBrowser
    except ImportError:
        print(
            "error: zeroconf not installed. Install with:\n"
            "    ~/somavenv/bin/pip install zeroconf",
            file=sys.stderr,
        )
        sys.exit(2)

    found = []

    class Listener:
        def add_service(self, zc, type_, name):
            info = zc.get_service_info(type_, name)
            if info and info.addresses:
                addr = socket.inet_ntoa(info.addresses[0])
                found.append(addr)

        def update_service(self, zc, type_, name):
            pass

        def remove_service(self, zc, type_, name):
            pass

    zc = Zeroconf()
    ServiceBrowser(zc, MDNS_SERVICE, Listener())
    end = time.time() + timeout
    while not found and time.time() < end:
        time.sleep(0.1)
    zc.close()
    if not found:
        print(f"error: no leaf found via mDNS on {MDNS_SERVICE} within {timeout}s", file=sys.stderr)
        sys.exit(3)
    return found[0]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--host", default=DEFAULT_HOST,
                    help=f"Leaf IP address (default: {DEFAULT_HOST})")
    ap.add_argument("--port", type=int, default=LEAF_TCP_PORT,
                    help=f"Leaf TCP port (default: {LEAF_TCP_PORT})")
    ap.add_argument("--interval", type=float, default=5.0,
                    help="Seconds between ticks (default: 5)")
    ap.add_argument("--ticks", type=int, default=0,
                    help="Number of ticks before exiting (0 = run forever)")
    ap.add_argument("--discover", action="store_true",
                    help="Auto-discover the leaf via mDNS instead of using --host")
    args = ap.parse_args()

    host = discover_leaf_host() if args.discover else args.host
    print(f"[init] connecting to leaf at {host}:{args.port}", flush=True)

    try:
        sock = socket.create_connection((host, args.port), timeout=5.0)
    except OSError as e:
        print(f"error: cannot connect to {host}:{args.port}: {e}", file=sys.stderr)
        return 1

    sock.settimeout(5.0)

    # Clear the display first so old content doesn't linger.
    try:
        invoke_skill(sock, "display.clear", {})
        print("[init] display cleared", flush=True)
    except Exception as e:
        print(f"error: display.clear failed: {e}", file=sys.stderr)
        return 1

    tick_num = 0
    try:
        while args.ticks == 0 or tick_num < args.ticks:
            tick_num += 1
            ok = tick(sock, tick_num)
            if not ok:
                return 1
            # Sleep with a responsive interrupt: wake up every 100ms
            # so Ctrl-C returns promptly.
            end = time.time() + args.interval
            while time.time() < end:
                time.sleep(min(0.1, end - time.time()))
    except KeyboardInterrupt:
        print("\n[exit] Ctrl-C", flush=True)
    finally:
        try:
            sock.close()
        except Exception:
            pass

    return 0


if __name__ == "__main__":
    sys.exit(main())
