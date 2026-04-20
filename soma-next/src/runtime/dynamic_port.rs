//! Dynamic port loading from `.so`/`.dylib` shared libraries.
//!
//! Enables the runtime to load external port packs at runtime without
//! recompilation. Each port pack is compiled as a `cdylib` crate and must
//! export a C-ABI init symbol:
//!
//! ```text
//! #[no_mangle]
//! pub extern "C" fn soma_port_init() -> *mut dyn soma_port_sdk::Port {
//!     Box::into_raw(Box::new(MyPort::new()))
//! }
//! ```
//!
//! The loader searches configured directories for libraries matching the
//! platform naming convention (`lib<name>.dylib` on macOS, `lib<name>.so`
//! on Linux), loads the symbol, and returns a boxed `Port` trait object.
//!
//! Because `soma_port_sdk::Port` and `crate::runtime::port::Port` are
//! separate traits (even though they mirror each other), the loader wraps
//! the SDK port in an `SdkPortAdapter` that bridges between them via JSON
//! serialization of `PortSpec` and `PortCallRecord`.

use std::path::PathBuf;

use crate::errors::{Result, SomaError};
use crate::runtime::port::Port;
use crate::runtime::port_verify;

/// Loads port adapters from shared libraries at runtime.
///
/// `search_paths` lists directories that may contain `.dylib`/`.so` files.
/// `loaded_libs` keeps `Library` handles alive for the lifetime of the
/// loader — dropping a library while its code is in use is undefined behavior.
/// `require_signatures` controls whether Ed25519 signature verification is
/// mandatory before loading a library.
pub struct DynamicPortLoader {
    search_paths: Vec<PathBuf>,
    loaded_libs: Vec<libloading::Library>,
    require_signatures: bool,
}

impl DynamicPortLoader {
    /// Create a loader that will search the given directories for port libraries.
    pub fn new(search_paths: Vec<PathBuf>) -> Self {
        Self {
            search_paths,
            loaded_libs: Vec::new(),
            require_signatures: false,
        }
    }

    /// Create a loader with explicit signature verification policy.
    pub fn with_signature_policy(search_paths: Vec<PathBuf>, require_signatures: bool) -> Self {
        Self {
            search_paths,
            loaded_libs: Vec::new(),
            require_signatures,
        }
    }

    /// Load a port adapter from a shared library.
    ///
    /// Searches `search_paths` for a file named `lib{library_name}.dylib`
    /// (macOS) or `lib{library_name}.so` (Linux). Loads the library, resolves
    /// the `soma_port_init` symbol, calls it to obtain a `Box<dyn soma_port_sdk::Port>`,
    /// wraps it in an `SdkPortAdapter`, and retains the library handle so the
    /// code stays mapped.
    pub fn load_port(&mut self, library_name: &str) -> Result<Box<dyn Port>> {
        let ext = if cfg!(target_os = "macos") {
            "dylib"
        } else if cfg!(target_os = "windows") {
            "dll"
        } else {
            "so"
        };

        let filename = format!("lib{library_name}.{ext}");

        let lib_path = self
            .search_paths
            .iter()
            .map(|dir| dir.join(&filename))
            .find(|p| p.exists())
            .ok_or_else(|| {
                SomaError::Port(format!(
                    "port library '{}' not found in search paths: {:?}",
                    filename, self.search_paths
                ))
            })?;

        tracing::debug!(path = %lib_path.display(), "loading dynamic port library");

        // Verify Ed25519 signature before loading the library into the process.
        port_verify::check_port_signature(&lib_path, self.require_signatures)?;

        let lib = unsafe {
            libloading::Library::new(&lib_path).map_err(|e| {
                SomaError::Port(format!(
                    "failed to load port library '{}': {}",
                    lib_path.display(),
                    e
                ))
            })?
        };

        // The port library exports soma_port_init() returning *mut dyn soma_port_sdk::Port.
        // We use the SDK trait here so the vtable is correct.
        let init_fn: libloading::Symbol<unsafe extern "C" fn() -> *mut dyn soma_port_sdk::Port> =
            unsafe {
                lib.get(b"soma_port_init").map_err(|e| {
                    SomaError::Port(format!(
                        "port library '{}' missing soma_port_init symbol: {}",
                        lib_path.display(),
                        e
                    ))
                })?
            };

        let port_ptr = unsafe { init_fn() };
        if port_ptr.is_null() {
            return Err(SomaError::Port(format!(
                "soma_port_init returned null for '{}'",
                lib_path.display()
            )));
        }

        let sdk_port = unsafe { Box::from_raw(port_ptr) };

        tracing::info!(
            port_id = %sdk_port.spec().port_id,
            path = %lib_path.display(),
            "dynamic port loaded"
        );

        // Keep the library alive — the port holds references to its symbols.
        self.loaded_libs.push(lib);

        // Wrap in adapter that bridges SDK Port → runtime Port.
        let adapter = SdkPortAdapter::new(sdk_port)?;
        Ok(Box::new(adapter))
    }

    /// Return the number of libraries currently held open.
    pub fn loaded_count(&self) -> usize {
        self.loaded_libs.len()
    }

    /// Return the configured search paths.
    pub fn search_paths(&self) -> &[PathBuf] {
        &self.search_paths
    }

    /// Add a directory to the search paths if not already present.
    pub fn add_search_path(&mut self, path: PathBuf) {
        if !self.search_paths.contains(&path) {
            self.search_paths.push(path);
        }
    }

    /// Discover and load all port libraries found in the search paths.
    ///
    /// Scans every directory in `search_paths` for files matching the platform
    /// naming convention `libsoma_port_*.{dylib,so,dll}`. For each library
    /// found, attempts to load it via `load_port`. Libraries that fail to load
    /// (missing symbol, init error, signature failure) are skipped with a
    /// warning — the runtime never hardcodes which ports should exist.
    ///
    /// Returns a vec of `(library_name, Box<dyn Port>)` pairs for each
    /// successfully loaded port.
    pub fn discover_all(&mut self) -> Vec<(String, Box<dyn Port>)> {
        let ext = if cfg!(target_os = "macos") {
            "dylib"
        } else if cfg!(target_os = "windows") {
            "dll"
        } else {
            "so"
        };

        let prefix = "libsoma_port_";
        let suffix = format!(".{ext}");

        // Collect unique library names across all search paths.
        let mut seen = std::collections::HashSet::new();
        let mut candidates: Vec<String> = Vec::new();

        for dir in &self.search_paths {
            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with(prefix) && name.ends_with(&suffix) {
                    // Extract the library name: libsoma_port_postgres.dylib → soma_port_postgres
                    let lib_name = name
                        .strip_prefix("lib")
                        .unwrap_or(&name)
                        .strip_suffix(&suffix)
                        .unwrap_or(&name)
                        .to_string();
                    if seen.insert(lib_name.clone()) {
                        candidates.push(lib_name);
                    }
                }
            }
        }

        let mut loaded = Vec::new();
        for lib_name in candidates {
            match self.load_port(&lib_name) {
                Ok(port) => {
                    eprintln!("auto: loaded {}", lib_name);
                    loaded.push((lib_name, port));
                }
                Err(e) => {
                    eprintln!("auto: skipped {} ({})", lib_name, e);
                }
            }
        }

        loaded
    }
}

// ---------------------------------------------------------------------------
// SdkPortAdapter — bridges soma_port_sdk::Port to crate::runtime::port::Port
// ---------------------------------------------------------------------------

/// Adapter that wraps a `soma_port_sdk::Port` and presents it as a
/// `crate::runtime::port::Port`.
///
/// Type conversion between SDK and runtime types is done via JSON
/// serialization — both sides implement `Serialize`/`Deserialize` with
/// identical schemas.
struct SdkPortAdapter {
    inner: Box<dyn soma_port_sdk::Port>,
    /// Cached spec converted from SDK format to runtime format.
    spec: crate::types::port::PortSpec,
}

impl SdkPortAdapter {
    fn new(inner: Box<dyn soma_port_sdk::Port>) -> Result<Self> {
        let sdk_spec = inner.spec();
        let spec_json = serde_json::to_value(sdk_spec).map_err(|e| {
            SomaError::Port(format!("failed to serialize SDK port spec: {e}"))
        })?;
        let spec: crate::types::port::PortSpec = serde_json::from_value(spec_json).map_err(|e| {
            SomaError::Port(format!("failed to convert SDK port spec to runtime format: {e}"))
        })?;
        Ok(Self { inner, spec })
    }
}

impl Port for SdkPortAdapter {
    fn spec(&self) -> &crate::types::port::PortSpec {
        &self.spec
    }

    fn invoke(
        &self,
        capability_id: &str,
        input: serde_json::Value,
    ) -> Result<crate::types::observation::PortCallRecord> {
        match self.inner.invoke(capability_id, input) {
            Ok(sdk_record) => {
                let json = serde_json::to_value(&sdk_record).map_err(|e| {
                    SomaError::Port(format!("failed to serialize SDK PortCallRecord: {e}"))
                })?;
                let record: crate::types::observation::PortCallRecord =
                    serde_json::from_value(json).map_err(|e| {
                        SomaError::Port(format!(
                            "failed to convert SDK PortCallRecord to runtime format: {e}"
                        ))
                    })?;
                Ok(record)
            }
            Err(sdk_err) => Err(SomaError::Port(sdk_err.to_string())),
        }
    }

    fn validate_input(
        &self,
        capability_id: &str,
        input: &serde_json::Value,
    ) -> Result<()> {
        self.inner
            .validate_input(capability_id, input)
            .map_err(|e| SomaError::Port(e.to_string()))
    }

    fn lifecycle_state(&self) -> crate::types::port::PortLifecycleState {
        let sdk_state = self.inner.lifecycle_state();
        let json = serde_json::to_value(sdk_state).unwrap_or_default();
        serde_json::from_value(json).unwrap_or(crate::types::port::PortLifecycleState::Loaded)
    }
}

// Mark SdkPortAdapter as thread-safe — the inner SDK port is Send + Sync.
unsafe impl Send for SdkPortAdapter {}
unsafe impl Sync for SdkPortAdapter {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_loader_starts_empty() {
        let loader = DynamicPortLoader::new(vec![PathBuf::from("/tmp/soma_ports")]);
        assert_eq!(loader.loaded_count(), 0);
        assert_eq!(loader.search_paths().len(), 1);
    }

    #[test]
    fn load_nonexistent_library_returns_error() {
        let mut loader = DynamicPortLoader::new(vec![PathBuf::from("/tmp/soma_nonexistent_dir")]);
        let result = loader.load_port("nonexistent_port");
        match result {
            Err(SomaError::Port(msg)) => {
                assert!(msg.contains("not found in search paths"), "unexpected msg: {msg}");
            }
            Err(other) => panic!("expected SomaError::Port, got: {other}"),
            Ok(_) => panic!("expected error but got Ok"),
        }
    }

    #[test]
    fn load_from_empty_search_paths_returns_error() {
        let mut loader = DynamicPortLoader::new(vec![]);
        let result = loader.load_port("some_port");
        assert!(result.is_err());
    }

    #[test]
    fn multiple_search_paths_are_preserved() {
        let paths = vec![
            PathBuf::from("/usr/local/lib/soma/ports"),
            PathBuf::from("/opt/soma/ports"),
            PathBuf::from("./ports"),
        ];
        let loader = DynamicPortLoader::new(paths.clone());
        assert_eq!(loader.search_paths(), &paths);
    }

    #[test]
    fn load_invalid_library_returns_error() {
        // Create a temp directory with a file that is not a valid shared library
        let dir = std::env::temp_dir().join("soma_dynamic_port_test_invalid");
        let _ = std::fs::create_dir_all(&dir);

        let ext = if cfg!(target_os = "macos") {
            "dylib"
        } else if cfg!(target_os = "windows") {
            "dll"
        } else {
            "so"
        };
        let fake_lib = dir.join(format!("libfake_port.{ext}"));
        std::fs::write(&fake_lib, b"not a real library").unwrap();

        let mut loader = DynamicPortLoader::new(vec![dir.clone()]);
        let result = loader.load_port("fake_port");

        let _ = std::fs::remove_dir_all(&dir);

        match result {
            Err(SomaError::Port(msg)) => {
                assert!(msg.contains("failed to load"), "unexpected msg: {msg}");
            }
            Err(other) => panic!("expected SomaError::Port, got: {other}"),
            Ok(_) => panic!("expected error but got Ok"),
        }
    }
}
