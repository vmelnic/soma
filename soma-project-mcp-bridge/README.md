# soma-project-mcp-bridge

Proof that soma-next can load **any MCP server as a port**, in any language,
via the new `PortBackend::McpClient` backend. No dylib, no SDK, no Rust —
just an MCP server executable and a line of manifest config.

This is the collapsed form of what used to be two bridges:

| Used to be | Is now |
|---|---|
| **Bridge A**: a Rust shim that marshals JSON over stdio to a script in Python/Node/Bun/PHP | `McpTransport::Stdio` — one subprocess, any language |
| **Bridge B**: soma-next consuming remote MCP servers | `McpTransport::Http` — same port type, different transport |

Both collapse to one `McpClientPort` in `soma-next/src/runtime/mcp_client_port.rs`.
Writing "a port in language X" is the same as "writing an MCP server in X."

## What's in this project

```
soma-project-mcp-bridge/
  bin/soma                             # soma-next release binary (gitignored)
  packs/hello/manifest.json            # pack declaring THREE ports — one per language
  servers/
    hello_py/server.py                 # Python MCP server,  pure stdlib  (~120 lines)
    hello_node/server.mjs              # Node.js MCP server, pure stdlib  (~110 lines)
    hello_php/server.php               # PHP MCP server,     pure stdlib  (~120 lines)
  scripts/run-mcp.sh                   # launches soma-next in MCP mode with this pack
  scripts/list-skills.sh               # prints list_ports output
  scripts/test.sh                      # runs the smoke test against all three ports
  mcp-client.mjs                       # Node.js MCP client — drives soma over stdio
```

The pack declares three ports — `hello_py`, `hello_node`, `hello_php` — each
pointing at a different-language MCP server via `PortBackend::McpClient`. All
three expose the same two tools (`greet`, `reverse`). soma-next spawns all
three subprocesses at bootstrap, runs `tools/list` on each, and registers
them as first-class ports. The only thing the smoke test uses to tell them
apart is the `(from <lang>)` tag each server bakes into its greeting.

## What the proof actually proves

At load time, for each of the three ports in `packs/hello/manifest.json`:

1. soma-next sees `"backend": {"type": "mcp_client", "transport": {"type": "stdio", "command": ..., "args": [...]}}`.
2. `McpClientPort::spawn_and_discover`:
   - spawns the child process (`python3 servers/hello_py/server.py`, `node servers/hello_node/server.mjs`, or `php servers/hello_php/server.php`)
   - sends JSON-RPC `initialize`, receives the server info
   - sends `notifications/initialized`
   - sends `tools/list`, receives the `greet` + `reverse` tool definitions
3. The discovered tools are merged into the port's `PortSpec.capabilities`.
   The manifest itself declares zero capabilities — discovery populates them.
4. The port registers with the runtime in `Active` state.

All three subprocesses run in parallel for the lifetime of the soma-next
process. `list_ports` reports three distinct ports, each with its own
discovered capability list.

At invoke time:

```
mcp-client.mjs (Node)
       │ invoke_port {port_id:"hello_node", capability_id:"greet", input:{name:"marcu"}}
       ▼
soma-next MCP server
       │ Port::invoke("greet", {"name":"marcu"})
       ▼
McpClientPort (hello_node)
       │ tools/call {name:"greet", arguments:{name:"marcu"}}  over stdio
       ▼
node servers/hello_node/server.mjs
       │ returns {content:[{type:"text", text:"{\"message\":\"hello marcu! (from node)\"}"}], isError:false}
       ▲
McpClientPort extract_structured → structured_result:{message:"hello marcu! (from node)"}
       ▲
soma-next returns PortCallRecord { success:true, structured_result:{...}, ... }
```

Substitute `hello_py` / `python3` / `(from python)` or `hello_php` / `php` /
`(from php)` and the rest of the diagram is identical. Two chained stdio
bridges per port; the runtime never has to know which language is on the
other end.

## Running the smoke test

Prerequisites: `python3`, `node`, and `php` on PATH, and the soma-next
release binary copied into `bin/`:

```bash
cp ../soma-next/target/release/soma bin/soma
xattr -d com.apple.quarantine bin/soma 2>/dev/null || true   # macOS
codesign -fs - bin/soma                                       # macOS
```

Then:

```bash
./scripts/test.sh
```

Expected output:

```json
{
  "ports": {
    "python": {
      "greet":   {"message": "hello marcu! (from python)"},
      "reverse": {"reversed": "!ucram olleh"}
    },
    "node": {
      "greet":   {"message": "hello marcu! (from node)"},
      "reverse": {"reversed": "!ucram olleh"}
    },
    "php": {
      "greet":   {"message": "hello marcu! (from php)"},
      "reverse": {"reversed": "!ucram olleh"}
    }
  },
  "ok": true
}
```

Individual commands:

```bash
node mcp-client.mjs list_ports                             # show all three ports + discovered capabilities
node mcp-client.mjs greet   --port hello_py   --name marcu
node mcp-client.mjs greet   --port hello_node --name marcu
node mcp-client.mjs greet   --port hello_php  --name marcu
node mcp-client.mjs reverse --port hello_node --text 'hello'
```

## Writing your own port in $LANGUAGE

1. Write an MCP server in any language with an MCP SDK (or pure stdlib — see
   `servers/hello_py/server.py` for a 100-line pure-Python example with no
   dependencies).
2. Expose one or more tools via `tools/list` and `tools/call`.
3. Drop a SOMA pack manifest next to it with:
   ```json
   "backend": {
     "type": "mcp_client",
     "transport": {
       "type": "stdio",
       "command": "node",
       "args": ["servers/my_port.mjs"]
     }
   }
   ```
4. Load the pack with `soma --mcp --pack packs/my_pack/manifest.json`.
5. Capabilities are discovered automatically. `invoke_port` routes to
   `tools/call` on your server.

No Rust, no FFI, no dylib, no SDK version to match.

## Consuming remote MCP servers

Use the `http` transport variant instead of `stdio`:

```json
"backend": {
  "type": "mcp_client",
  "transport": {
    "type": "http",
    "url": "https://some-remote-mcp-server.example.com/",
    "headers": {"Authorization": "Bearer ..."}
  }
}
```

Same port type, same discovery, same invoke path — only the transport
changes. Every MCP server ever written in any hosted cloud becomes port
material.
