use std::collections::HashMap;
use std::env;
use std::io::{self, BufRead, Write as IoWrite};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use soma_next::bootstrap;
use soma_next::config::SomaConfig;
use soma_next::distributed::transport::{
    LocalDispatchHandler, PeerAddressMap, TcpRemoteExecutor, TlsTcpRemoteExecutor,
    start_listener_background, start_tls_listener_background,
};
#[cfg(unix)]
use soma_next::distributed::unix_transport::{
    UnixPeerPathMap, UnixRemoteExecutor, start_unix_listener_background,
};
use soma_next::distributed::ws_transport::start_ws_listener_background;
use soma_next::interfaces::cli::{CliCommand, CliRunner, DefaultCliRunner};
use soma_next::interfaces::mcp::{McpRequest, McpServer};

fn main() {
    let args: Vec<String> = env::args().collect();

    // No args or just the binary name → show usage
    if args.len() <= 1 {
        print_usage();
        return;
    }

    // Check for --mcp flag → MCP JSON-RPC mode over stdin/stdout
    if args.iter().any(|a| a == "--mcp") {
        // Extract --pack, --listen, --peer, --unix-listen, --unix-peer,
        // --discover-lan for MCP mode.
        let mut mcp_pack_paths: Vec<String> = Vec::new();
        let mut mcp_listen: Option<SocketAddr> = None;
        let mut mcp_peer_addrs: Vec<SocketAddr> = Vec::new();
        #[cfg(unix)]
        let mut mcp_unix_listen: Option<std::path::PathBuf> = None;
        #[cfg(unix)]
        let mut mcp_unix_peers: Vec<std::path::PathBuf> = Vec::new();
        let mut mcp_discover_lan = false;
        let mut mcp_skip = false;
        for (i, arg) in args.iter().enumerate() {
            if mcp_skip {
                mcp_skip = false;
                continue;
            }
            if arg == "--pack"
                && let Some(path) = args.get(i + 1) {
                    mcp_pack_paths.push(path.clone());
                    mcp_skip = true;
                }
            if arg == "--listen"
                && let Some(addr_str) = args.get(i + 1) {
                    if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                        mcp_listen = Some(addr);
                    }
                    mcp_skip = true;
                }
            if arg == "--peer"
                && let Some(addr_str) = args.get(i + 1) {
                    if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                        mcp_peer_addrs.push(addr);
                    }
                    mcp_skip = true;
                }
            if arg == "--discover-lan" {
                mcp_discover_lan = true;
            }
            #[cfg(unix)]
            if arg == "--unix-listen"
                && let Some(path_str) = args.get(i + 1) {
                    mcp_unix_listen = Some(std::path::PathBuf::from(path_str));
                    mcp_skip = true;
                }
            #[cfg(unix)]
            if arg == "--unix-peer"
                && let Some(path_str) = args.get(i + 1) {
                    mcp_unix_peers.push(std::path::PathBuf::from(path_str));
                    mcp_skip = true;
                }
        }

        let mcp_distributed = McpDistributedConfig {
            listen: mcp_listen,
            peer_addrs: mcp_peer_addrs,
            discover_lan: mcp_discover_lan,
            #[cfg(unix)]
            unix_listen: mcp_unix_listen,
            #[cfg(unix)]
            unix_peers: mcp_unix_peers,
        };
        run_mcp_server(&mcp_pack_paths, mcp_distributed);
        return;
    }

    // Extract --pack <path>, --listen <addr>, --ws-listen <addr>, --peer <addr>,
    // --unix-listen <path>, and --unix-peer <path> arguments
    let mut pack_paths: Vec<String> = Vec::new();
    let mut listen_addr: Option<SocketAddr> = None;
    let mut ws_listen_addr: Option<SocketAddr> = None;
    let mut peer_addrs: Vec<SocketAddr> = Vec::new();
    #[cfg(unix)]
    let mut unix_listen_path: Option<std::path::PathBuf> = None;
    #[cfg(unix)]
    let mut unix_peer_paths: Vec<std::path::PathBuf> = Vec::new();
    let mut filtered_args: Vec<String> = Vec::new();
    let mut skip_next = false;

    for (i, arg) in args.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "--pack"
            && let Some(path) = args.get(i + 1) {
                pack_paths.push(path.clone());
                skip_next = true;
                continue;
            }
        if arg == "--listen"
            && let Some(addr_str) = args.get(i + 1) {
                match addr_str.parse::<SocketAddr>() {
                    Ok(addr) => listen_addr = Some(addr),
                    Err(e) => {
                        eprintln!("error: invalid --listen address '{}': {}", addr_str, e);
                        std::process::exit(1);
                    }
                }
                skip_next = true;
                continue;
            }
        if arg == "--ws-listen"
            && let Some(addr_str) = args.get(i + 1) {
                match addr_str.parse::<SocketAddr>() {
                    Ok(addr) => ws_listen_addr = Some(addr),
                    Err(e) => {
                        eprintln!("error: invalid --ws-listen address '{}': {}", addr_str, e);
                        std::process::exit(1);
                    }
                }
                skip_next = true;
                continue;
            }
        if arg == "--peer"
            && let Some(addr_str) = args.get(i + 1) {
                match addr_str.parse::<SocketAddr>() {
                    Ok(addr) => peer_addrs.push(addr),
                    Err(e) => {
                        eprintln!("error: invalid --peer address '{}': {}", addr_str, e);
                        std::process::exit(1);
                    }
                }
                skip_next = true;
                continue;
            }
        #[cfg(unix)]
        if arg == "--unix-listen"
            && let Some(path_str) = args.get(i + 1) {
                unix_listen_path = Some(std::path::PathBuf::from(path_str));
                skip_next = true;
                continue;
            }
        #[cfg(unix)]
        if arg == "--unix-peer"
            && let Some(path_str) = args.get(i + 1) {
                unix_peer_paths.push(std::path::PathBuf::from(path_str));
                skip_next = true;
                continue;
            }
        filtered_args.push(arg.clone());
    }

    // Load config
    let config = SomaConfig::load(Path::new("soma.toml")).unwrap_or_default();

    // Build peer address map for the TCP remote executor. Each peer gets an
    // auto-generated ID based on its address.
    let peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
    for (i, addr) in peer_addrs.iter().enumerate() {
        let peer_id = format!("peer-{}", i);
        peer_map.lock().unwrap().insert(peer_id, *addr);
    }

    // Build Unix peer path map if any --unix-peer arguments were given.
    #[cfg(unix)]
    let unix_peer_map: UnixPeerPathMap = Arc::new(Mutex::new(HashMap::new()));
    #[cfg(unix)]
    for (i, path) in unix_peer_paths.iter().enumerate() {
        let peer_id = format!("unix-peer-{}", i);
        unix_peer_map
            .lock()
            .unwrap()
            .insert(peer_id, path.clone());
    }

    // Bootstrap the runtime, optionally with a remote executor (TCP or Unix).
    let has_tcp_peers = !peer_addrs.is_empty();
    #[cfg(unix)]
    let has_unix_peers = !unix_peer_paths.is_empty();
    #[cfg(not(unix))]
    let has_unix_peers = false;
    let has_peers = has_tcp_peers || has_unix_peers;

    // Select the appropriate remote executor. When TLS config is present and
    // TCP peers are in use, wrap the connection with TLS. Otherwise fall back
    // to plain TCP. Unix peers are used when no TCP peers are configured.
    let tls_config = config.distributed.tls_config();
    let make_executor = || -> Box<dyn soma_next::distributed::remote::RemoteExecutor> {
        if has_tcp_peers {
            if let Some(ref tls) = tls_config {
                match TlsTcpRemoteExecutor::new(Arc::clone(&peer_map), tls) {
                    Ok(executor) => {
                        eprintln!("Using TLS for outbound peer connections");
                        return Box::new(executor);
                    }
                    Err(e) => {
                        eprintln!("warning: TLS executor setup failed ({}), falling back to plain TCP", e);
                    }
                }
            }
            Box::new(TcpRemoteExecutor::new(Arc::clone(&peer_map)))
        } else {
            #[cfg(unix)]
            {
                Box::new(UnixRemoteExecutor::new(Arc::clone(&unix_peer_map)))
            }
            #[cfg(not(unix))]
            {
                unreachable!()
            }
        }
    };

    let is_auto = pack_paths.len() == 1 && pack_paths[0] == "auto";

    let bootstrap_result = if is_auto {
        eprintln!("auto: discovering ports from plugin search paths");
        bootstrap::bootstrap_auto(&config)
    } else if pack_paths.is_empty() {
        let default_manifest = "packs/reference/manifest.json";
        let effective_packs: Vec<String> = if Path::new(default_manifest).exists() {
            vec![default_manifest.to_string()]
        } else {
            vec![]
        };
        if has_peers {
            bootstrap::bootstrap_with_remote(&config, &effective_packs, make_executor())
        } else {
            bootstrap::bootstrap(&config, &effective_packs)
        }
    } else if has_peers {
        bootstrap::bootstrap_with_remote(&config, &pack_paths, make_executor())
    } else {
        bootstrap::bootstrap(&config, &pack_paths)
    };

    let runtime = match bootstrap_result {
        Ok(rt) => rt,
        Err(e) => {
            if pack_paths.is_empty() {
                eprintln!("warning: failed to load default pack: {e}");
                // Fall through without a runtime for stub mode.
                let cli = DefaultCliRunner::stub();
                run_cli(cli, &filtered_args);
                return;
            } else {
                eprintln!("error: failed to bootstrap runtime: {e}");
                std::process::exit(1);
            }
        }
    };

    // If any listener flag is specified, start transport listeners in
    // background threads. All listeners share the same runtime handler.
    #[cfg(unix)]
    let has_unix_listener = unix_listen_path.is_some();
    #[cfg(not(unix))]
    let has_unix_listener = false;
    let has_any_listener =
        listen_addr.is_some() || ws_listen_addr.is_some() || has_unix_listener;
    if has_any_listener {
        let schema_store = Arc::clone(&runtime.schema_store);
        let routine_store = Arc::clone(&runtime.routine_store);
        let runtime_arc = Arc::new(Mutex::new(runtime));
        let handler: Arc<dyn soma_next::distributed::transport::IncomingHandler> =
            Arc::new(LocalDispatchHandler::with_stores(
                Arc::clone(&runtime_arc), schema_store, routine_store));

        if let Some(addr) = listen_addr {
            if let Some(ref tls) = tls_config {
                match start_tls_listener_background(addr, Arc::clone(&handler), tls) {
                    Ok(_handle) => {
                        eprintln!("TLS TCP transport listening on {}", addr);
                    }
                    Err(e) => {
                        eprintln!("warning: TLS listener setup failed ({}), falling back to plain TCP", e);
                        let _tcp_handle = start_listener_background(addr, Arc::clone(&handler));
                        eprintln!("TCP transport listening on {}", addr);
                    }
                }
            } else {
                let _tcp_handle = start_listener_background(addr, Arc::clone(&handler));
                eprintln!("TCP transport listening on {}", addr);
            }
        }

        if let Some(addr) = ws_listen_addr {
            let _ws_handle = start_ws_listener_background(addr, Arc::clone(&handler));
            eprintln!("WebSocket transport listening on {}", addr);
        }

        #[cfg(unix)]
        if let Some(ref path) = unix_listen_path {
            let _unix_handle =
                start_unix_listener_background(path.clone(), Arc::clone(&handler));
            eprintln!("Unix transport listening on {}", path.display());
        }

        // In listener mode, we wrap the runtime in an Arc so the CLI runner
        // and the listeners can share it. The CLI runner uses a clone.
        let cli = DefaultCliRunner::with_runtime_arc(Arc::clone(&runtime_arc));

        if filtered_args.len() > 1 {
            let command = match cli.parse_args(filtered_args[1..].to_vec()) {
                Ok(cmd) => cmd,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };

            if command == CliCommand::Repl {
                run_repl(&cli);
            } else {
                match cli.execute(command) {
                    Ok(output) => println!("{output}"),
                    Err(e) => eprintln!("error: {e}"),
                }
                // Keep process alive to serve incoming connections.
                eprintln!("Listening for incoming connections. Press Ctrl+C to stop.");
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            }
        } else {
            // No command — just listen. Enter REPL for interactive use.
            run_repl(&cli);
        }
        return;
    }

    let cli = DefaultCliRunner::with_runtime(runtime);
    run_cli(cli, &filtered_args);
}

/// Run the CLI with the given filtered arguments.
fn run_cli(cli: DefaultCliRunner, filtered_args: &[String]) {
    if filtered_args.len() <= 1 {
        print_usage();
        return;
    }

    let command = match cli.parse_args(filtered_args[1..].to_vec()) {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    if command == CliCommand::Repl {
        run_repl(&cli);
        return;
    }

    match cli.execute(command) {
        Ok(output) => println!("{output}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn print_usage() {
    println!(
        "\
soma-next — goal-driven runtime

USAGE:
    soma <command> [options]

COMMANDS:
    run --goal <text>         Submit and execute a goal
    inspect --session <id>    Inspect a session
    restore <session_id>      Restore a session from disk checkpoint
    sessions                  List all sessions
    packs                     List loaded packs
    skills                    List available skills
    metrics [--format <fmt>]  Show runtime metrics (fmt: text, json, prometheus)
    verify-port <path>        Verify Ed25519 signature of a port library
    repl                      Interactive REPL mode

FLAGS:
    --mcp                     Start MCP JSON-RPC server on stdin/stdout
    --listen <addr>           Start TCP listener for incoming peer connections
    --ws-listen <addr>        Start WebSocket listener for browser/UI connections
    --peer <addr>             Register a remote TCP peer at the given address
    --unix-listen <path>      Start Unix socket listener for fast local IPC
    --unix-peer <path>        Register a remote peer via Unix socket path"
    );
}

/// Configuration for distributed features in MCP mode.
struct McpDistributedConfig {
    listen: Option<SocketAddr>,
    peer_addrs: Vec<SocketAddr>,
    /// When true, start an mDNS browser for `_soma._tcp.local.` and
    /// register discovered peers dynamically. Discovered peers share
    /// the same peer_map and peer_ids list as static `--peer` entries.
    discover_lan: bool,
    #[cfg(unix)]
    unix_listen: Option<std::path::PathBuf>,
    #[cfg(unix)]
    unix_peers: Vec<std::path::PathBuf>,
}

/// MCP server: read JSON-RPC requests from stdin, write responses to stdout.
fn run_mcp_server(pack_paths: &[String], distributed: McpDistributedConfig) {
    use soma_next::interfaces::mcp::RuntimeHandle;

    let config = SomaConfig::load(Path::new("soma.toml")).unwrap_or_default();

    // Build peer maps and determine if we need a remote executor.
    let has_tcp_peers = !distributed.peer_addrs.is_empty();
    #[cfg(unix)]
    let has_unix_peers = !distributed.unix_peers.is_empty();
    #[cfg(not(unix))]
    let has_unix_peers = false;
    // With --discover-lan the TCP remote executor is needed even when no
    // static --peer is given, because discovered peers come in over TCP.
    let has_peers = has_tcp_peers || has_unix_peers || distributed.discover_lan;

    let tcp_peer_map: PeerAddressMap = Arc::new(Mutex::new(HashMap::new()));
    // Dynamic peer-id list — Arc<Mutex<Vec<String>>> so the LAN discovery
    // background thread can push/remove entries while the MCP handler
    // reads them. Static --peer entries from the CLI are pre-populated
    // into the same list.
    let shared_peer_ids: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    for (i, addr) in distributed.peer_addrs.iter().enumerate() {
        let pid = format!("peer-{}", i);
        tcp_peer_map.lock().unwrap().insert(pid.clone(), *addr);
        shared_peer_ids.lock().unwrap().push(pid);
    }
    #[cfg(unix)]
    let unix_peer_map: soma_next::distributed::unix_transport::UnixPeerPathMap =
        Arc::new(Mutex::new(HashMap::new()));
    #[cfg(unix)]
    for (i, path) in distributed.unix_peers.iter().enumerate() {
        let pid = format!("unix-peer-{}", i);
        unix_peer_map.lock().unwrap().insert(pid.clone(), path.clone());
        shared_peer_ids.lock().unwrap().push(pid);
    }

    // Start the LAN discovery browser if requested. The handle needs to
    // stay alive for the lifetime of the process — we leak it into a
    // static via Box::leak since MCP mode never cleanly shuts down.
    if distributed.discover_lan {
        match soma_next::distributed::discovery::spawn_lan_browser(
            Arc::clone(&tcp_peer_map),
            Arc::clone(&shared_peer_ids),
        ) {
            Ok(daemon) => {
                eprintln!(
                    "MCP: LAN discovery active (browsing {})",
                    soma_next::distributed::discovery::SOMA_SERVICE_TYPE
                );
                // Leak so the daemon thread keeps running
                Box::leak(Box::new(daemon));
            }
            Err(e) => {
                eprintln!("MCP: LAN discovery failed to start: {}", e);
            }
        }
    }

    // Build remote executor if peers are configured.
    //
    // With --discover-lan, use the TCP executor even if no static --peer
    // is given, because discovered peers arrive on TCP and need an
    // executor to route invocations to them.
    let use_tcp_executor = has_tcp_peers || distributed.discover_lan;
    let make_executor = || -> Option<Box<dyn soma_next::distributed::remote::RemoteExecutor>> {
        if !has_peers {
            return None;
        }
        if use_tcp_executor {
            let tls_config = config.distributed.tls_config();
            if let Some(ref tls) = tls_config
                && let Ok(executor) = TlsTcpRemoteExecutor::new(Arc::clone(&tcp_peer_map), tls)
            {
                eprintln!("MCP: Using TLS for outbound peer connections");
                return Some(Box::new(executor));
            }
            Some(Box::new(TcpRemoteExecutor::new(Arc::clone(&tcp_peer_map))))
        } else {
            #[cfg(unix)]
            {
                use soma_next::distributed::unix_transport::UnixRemoteExecutor;
                Some(Box::new(UnixRemoteExecutor::new(Arc::clone(&unix_peer_map))))
            }
            #[cfg(not(unix))]
            {
                None
            }
        }
    };

    let bootstrap_runtime = |packs: &[String]| -> std::result::Result<crate::bootstrap::Runtime, String> {
        if has_peers {
            let exec = make_executor().unwrap();
            bootstrap::bootstrap_with_remote(&config, packs, exec)
                .map_err(|e| e.to_string())
        } else {
            bootstrap::bootstrap(&config, packs)
                .map_err(|e| e.to_string())
        }
    };

    let mcp_is_auto = pack_paths.len() == 1 && pack_paths[0] == "auto";

    let make_server = |runtime: crate::bootstrap::Runtime| -> McpServer {
        let handle = RuntimeHandle::from_runtime(runtime);
        let handle = if has_peers {
            if let Some(exec) = make_executor() {
                handle.with_remote_shared(exec, Arc::clone(&shared_peer_ids))
            } else {
                handle
            }
        } else {
            handle
        };
        McpServer::new(handle)
    };

    let server = if mcp_is_auto {
        eprintln!("MCP: auto-discovering ports from plugin search paths");
        match bootstrap::bootstrap_auto(&config) {
            Ok(runtime) => make_server(runtime),
            Err(e) => {
                eprintln!("error: failed to auto-bootstrap runtime: {e}");
                std::process::exit(1);
            }
        }
    } else if pack_paths.is_empty() {
        let default_manifest = "packs/reference/manifest.json";
        let packs = if Path::new(default_manifest).exists() {
            vec![default_manifest.to_string()]
        } else {
            vec![]
        };
        match bootstrap_runtime(&packs) {
            Ok(runtime) => make_server(runtime),
            Err(e) => {
                eprintln!("warning: failed to bootstrap runtime: {e}");
                McpServer::new_stub()
            }
        }
    } else {
        match bootstrap_runtime(pack_paths) {
            Ok(runtime) => make_server(runtime),
            Err(e) => {
                eprintln!("error: failed to bootstrap runtime: {e}");
                std::process::exit(1);
            }
        }
    };

    // Start TCP listener if requested (background thread).
    if let Some(addr) = distributed.listen {
        // Bootstrap a listener runtime with schema/routine stores wired in
        // so that transferred schemas and routines are actually stored.
        let listener_packs: Vec<String> = if pack_paths.is_empty() {
            let dm = "packs/reference/manifest.json";
            if Path::new(dm).exists() { vec![dm.to_string()] } else { vec![] }
        } else {
            pack_paths.to_vec()
        };
        match bootstrap::bootstrap(&config, &listener_packs) {
            Ok(listener_rt) => {
                let schema_store = Arc::clone(&listener_rt.schema_store);
                let routine_store = Arc::clone(&listener_rt.routine_store);
                let runtime_arc = Arc::new(Mutex::new(listener_rt));
                let handler: Arc<dyn soma_next::distributed::transport::IncomingHandler> =
                    Arc::new(LocalDispatchHandler::with_stores(
                        Arc::clone(&runtime_arc), schema_store, routine_store));
                let _tcp_handle = start_listener_background(addr, handler);
                eprintln!("MCP: TCP transport listening on {}", addr);
            }
            Err(e) => {
                eprintln!("warning: failed to bootstrap listener runtime: {e}");
            }
        }
    }

    // Start Unix listener if requested.
    #[cfg(unix)]
    if let Some(ref path) = distributed.unix_listen {
        let listener_packs: Vec<String> = if pack_paths.is_empty() {
            let dm = "packs/reference/manifest.json";
            if Path::new(dm).exists() { vec![dm.to_string()] } else { vec![] }
        } else {
            pack_paths.to_vec()
        };
        match bootstrap::bootstrap(&config, &listener_packs) {
            Ok(listener_rt) => {
                let schema_store = Arc::clone(&listener_rt.schema_store);
                let routine_store = Arc::clone(&listener_rt.routine_store);
                let runtime_arc = Arc::new(Mutex::new(listener_rt));
                let handler: Arc<dyn soma_next::distributed::transport::IncomingHandler> =
                    Arc::new(LocalDispatchHandler::with_stores(
                        Arc::clone(&runtime_arc), schema_store, routine_store));
                let _unix_handle =
                    soma_next::distributed::unix_transport::start_unix_listener_background(
                        path.clone(), handler);
                eprintln!("MCP: Unix transport listening on {}", path.display());
            }
            Err(e) => {
                eprintln!("warning: failed to bootstrap Unix listener runtime: {e}");
            }
        }
    }
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Parse JSON-RPC request
        let request: McpRequest = match serde_json::from_str(trimmed) {
            Ok(req) => req,
            Err(e) => {
                let error_response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "error": {
                        "code": -32700,
                        "message": format!("Parse error: {e}")
                    },
                    "id": null
                });
                let _ = writeln!(stdout, "{}", error_response);
                let _ = stdout.flush();
                continue;
            }
        };

        // Handle request
        let response = match server.handle_request(request) {
            Ok(resp) => resp,
            Err(e) => {
                let error_response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "error": {
                        "code": -32603,
                        "message": format!("Internal error: {e}")
                    },
                    "id": null
                });
                let _ = writeln!(stdout, "{}", error_response);
                let _ = stdout.flush();
                continue;
            }
        };
        match serde_json::to_string(&response) {
            Ok(json) => {
                let _ = writeln!(stdout, "{json}");
                let _ = stdout.flush();
            }
            Err(e) => {
                eprintln!("serialization error: {e}");
            }
        }
    }
}

/// Interactive REPL: read goals from stdin, execute, print results.
fn run_repl(cli: &DefaultCliRunner) {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    println!("soma-next REPL (type 'quit' to exit)");

    loop {
        print!("soma> ");
        let _ = stdout.flush();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => break,
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "quit" || trimmed == "exit" {
            break;
        }

        // Parse as CLI args
        let args: Vec<String> = shell_split(trimmed);
        match cli.parse_args(args) {
            Ok(cmd) => match cli.execute(cmd) {
                Ok(output) => println!("{output}"),
                Err(e) => eprintln!("error: {e}"),
            },
            Err(e) => eprintln!("error: {e}"),
        }
    }
}

/// Simple shell-like splitting (handles quoted strings).
fn shell_split(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut quote_char = '"';

    for ch in input.chars() {
        if in_quotes {
            if ch == quote_char {
                in_quotes = false;
            } else {
                current.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            in_quotes = true;
            quote_char = ch;
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}
