//! Dynamic port loading from `.so`/`.dylib` shared libraries.
//!
//! Enables the runtime to load external port packs at runtime without
//! recompilation. Each port pack is compiled as a `cdylib` crate and must
//! export a C-ABI init symbol:
//!
//! ```text
//! #[no_mangle]
//! pub extern "C" fn soma_port_init() -> *mut dyn Port {
//!     Box::into_raw(Box::new(MyPort::new()))
//! }
//! ```
//!
//! The loader searches configured directories for libraries matching the
//! platform naming convention (`lib<name>.dylib` on macOS, `lib<name>.so`
//! on Linux), loads the symbol, and returns a boxed `Port` trait object.

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
    /// the `soma_port_init` symbol, calls it to obtain a `Box<dyn Port>`, and
    /// retains the library handle so the code stays mapped.
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

        let init_fn: libloading::Symbol<unsafe extern "C" fn() -> *mut dyn Port> = unsafe {
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

        let port = unsafe { Box::from_raw(port_ptr) };

        tracing::info!(
            port_id = %port.spec().port_id,
            path = %lib_path.display(),
            "dynamic port loaded"
        );

        // Keep the library alive — the port holds references to its symbols.
        self.loaded_libs.push(lib);

        Ok(port)
    }

    /// Return the number of libraries currently held open.
    pub fn loaded_count(&self) -> usize {
        self.loaded_libs.len()
    }

    /// Return the configured search paths.
    pub fn search_paths(&self) -> &[PathBuf] {
        &self.search_paths
    }
}

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
