"""
Synaptic Protocol — SOMA-to-SOMA communication.

Whitepaper Section 10.2:
  Signal:       compact, self-describing unit of communication
  Synapse:      connection between two SOMAs (TCP for this PoC)
  Transmission: async by default, sync available
  Discovery:    presence broadcasting (chemical gradient model)

A SOMA runs a synapse server (TCP listener) for incoming signals.
It sends signals to peers via TCP client connections.
"""

import json
import socket
import threading
import time
from dataclasses import dataclass, field
from datetime import datetime


@dataclass
class Signal:
    """The fundamental unit of inter-SOMA communication."""
    type: str           # "intent", "data", "discover", "ack"
    sender: str         # sender SOMA name
    recipient: str      # recipient SOMA name
    payload: dict       # signal content
    timestamp: str = ""

    def __post_init__(self):
        if not self.timestamp:
            self.timestamp = datetime.now().isoformat()

    def encode(self) -> bytes:
        return json.dumps({
            "type": self.type, "from": self.sender, "to": self.recipient,
            "payload": self.payload, "timestamp": self.timestamp,
        }).encode("utf-8") + b"\n"

    @staticmethod
    def decode(data: bytes) -> "Signal":
        d = json.loads(data.decode("utf-8").strip())
        return Signal(
            type=d["type"], sender=d["from"], recipient=d["to"],
            payload=d["payload"], timestamp=d["timestamp"],
        )


@dataclass
class Peer:
    """A known SOMA on the network."""
    name: str
    host: str
    port: int
    last_seen: float = 0.0


class SynapseServer:
    """TCP server for receiving signals from other SOMAs.
    Runs in a background thread. Calls on_signal for each received signal."""

    def __init__(self, name: str, host: str, port: int, on_signal=None):
        self.name = name
        self.host = host
        self.port = port
        self.on_signal = on_signal or (lambda s: None)
        self.peers: dict[str, Peer] = {}
        self.received: list[Signal] = []
        self._server = None
        self._thread = None
        self._running = False

    def start(self):
        self._running = True
        self._server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self._server.settimeout(1.0)
        self._server.bind((self.host, self.port))
        self._server.listen(5)
        self._thread = threading.Thread(target=self._listen, daemon=True)
        self._thread.start()

    def stop(self):
        self._running = False
        if self._server:
            self._server.close()
        if self._thread:
            self._thread.join(timeout=2)

    def _listen(self):
        while self._running:
            try:
                conn, addr = self._server.accept()
                data = b""
                while True:
                    chunk = conn.recv(65536)
                    if not chunk:
                        break
                    data += chunk
                    if b"\n" in data:
                        break
                conn.close()
                if data:
                    signal = Signal.decode(data)
                    self.received.append(signal)
                    # Handle discovery signals
                    if signal.type == "discover":
                        self.peers[signal.sender] = Peer(
                            name=signal.sender,
                            host=signal.payload.get("host", addr[0]),
                            port=signal.payload.get("port", 0),
                            last_seen=time.time(),
                        )
                    self.on_signal(signal)
            except socket.timeout:
                continue
            except OSError:
                break

    def register_peer(self, name: str, host: str, port: int):
        """Manually register a known peer."""
        self.peers[name] = Peer(name=name, host=host, port=port, last_seen=time.time())

    def send_signal(self, signal: Signal) -> bool:
        """Send a signal to a peer via TCP."""
        peer = self.peers.get(signal.recipient)
        if not peer:
            return False
        try:
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sock.settimeout(5.0)
            sock.connect((peer.host, peer.port))
            sock.sendall(signal.encode())
            sock.close()
            return True
        except (ConnectionRefusedError, OSError):
            return False

    def broadcast_discover(self):
        """Announce presence to all known peers (chemical gradient)."""
        sig = Signal(
            type="discover", sender=self.name, recipient="*",
            payload={"host": self.host, "port": self.port},
        )
        for peer in list(self.peers.values()):
            sig.recipient = peer.name
            self.send_signal(sig)

    def send_data(self, recipient: str, data) -> bool:
        """Send data to a peer SOMA."""
        # Serialize data for JSON transport
        if isinstance(data, (list, dict, str, int, float, bool)):
            serialized = data
        else:
            serialized = str(data)

        signal = Signal(
            type="data", sender=self.name, recipient=recipient,
            payload={"data": serialized},
        )
        return self.send_signal(signal)

    def send_intent(self, recipient: str, intent: str) -> bool:
        """Delegate an intent to a peer SOMA."""
        signal = Signal(
            type="intent", sender=self.name, recipient=recipient,
            payload={"intent": intent},
        )
        return self.send_signal(signal)
