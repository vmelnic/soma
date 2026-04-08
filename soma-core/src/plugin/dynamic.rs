//! Dynamic plugin loading from `.so`/`.dylib` shared libraries (Section 5.3).
//!
//! Each plugin crate is compiled as `cdylib` and must export:
//! ```ignore
//! #[no_mangle]
//! pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
//!     Box::into_raw(Box::new(MyPlugin::new()))
//! }
//! ```
//!
//! Loading supports optional Ed25519 signature verification via sidecar `.sig`/`.pub` files,
//! manifest-based metadata parsing, and platform compatibility filtering.

use anyhow::{Context, Result};
use std::path::Path;

use super::interface::SomaPlugin;

/// Verify Ed25519 signature of a plugin binary against a known public key.
pub fn verify_plugin_signature(
    plugin_path: &Path,
    signature: &[u8; 64],
    public_key: &[u8; 32],
) -> bool {
    let Ok(plugin_bytes) = std::fs::read(plugin_path) else { return false };
    crate::protocol::encryption::verify(public_key, &plugin_bytes, signature)
}

/// Load a plugin from a shared library, calling its `soma_plugin_init` symbol.
///
/// When sidecar signature files exist (`<path>.sig` + `<path>.pub`), Ed25519 verification
/// is mandatory -- a failed check returns an error. Without sidecar files, the plugin
/// loads unverified. The library is intentionally leaked (`mem::forget`) because the
/// plugin holds references to symbols within it.
pub fn load_plugin_from_path(path: &Path) -> Result<Box<dyn SomaPlugin>> {
    let ext_str = path.extension().unwrap_or_default().to_str().unwrap_or("");
    let sig_path = path.with_extension(format!("{ext_str}.sig"));
    let pub_path = path.with_extension(format!("{ext_str}.pub"));

    if sig_path.exists() && pub_path.exists() {
        let sig_bytes = std::fs::read(&sig_path)
            .with_context(|| format!("Failed to read signature file: {}", sig_path.display()))?;
        let pub_bytes = std::fs::read(&pub_path)
            .with_context(|| format!("Failed to read public key file: {}", pub_path.display()))?;
        if sig_bytes.len() == 64 && pub_bytes.len() == 32 {
            let sig: [u8; 64] = sig_bytes.try_into().unwrap();
            let pubk: [u8; 32] = pub_bytes.try_into().unwrap();
            if !verify_plugin_signature(path, &sig, &pubk) {
                anyhow::bail!("Plugin signature verification FAILED: {}", path.display());
            }
            tracing::info!(path = %path.display(), "Plugin signature verified");
        } else {
            tracing::warn!(
                path = %path.display(),
                sig_len = sig_bytes.len(),
                pub_len = pub_bytes.len(),
                "Signature/key files have invalid sizes (expected 64/32 bytes), loading without verification"
            );
        }
    } else {
        tracing::debug!(path = %path.display(), "No signature file found — loading without verification");
    }

    let lib = unsafe {
        libloading::Library::new(path)
            .with_context(|| format!("Failed to load plugin library: {}", path.display()))?
    };

    let init_fn: libloading::Symbol<unsafe extern "C" fn() -> *mut dyn SomaPlugin> = unsafe {
        lib.get(b"soma_plugin_init")
            .with_context(|| format!("Plugin missing soma_plugin_init symbol: {}", path.display()))?
    };

    let plugin_ptr = unsafe { init_fn() };
    if plugin_ptr.is_null() {
        anyhow::bail!("soma_plugin_init returned null for: {}", path.display());
    }

    let plugin = unsafe { Box::from_raw(plugin_ptr) };

    // Leak the library -- plugin holds live references to its symbols
    std::mem::forget(lib);

    tracing::info!(
        plugin = plugin.name(),
        version = plugin.version(),
        path = %path.display(),
        "Dynamic plugin loaded"
    );

    Ok(plugin)
}

/// Find plugin libraries in a directory: both top-level `.so`/`.dylib` files and
/// subdirectories with a `manifest.json`/`.toml` containing `lib<dirname>.<ext>`.
pub fn scan_plugin_directory(dir: &Path) -> Vec<std::path::PathBuf> {
    let ext = if cfg!(target_os = "macos") { "dylib" }
              else if cfg!(target_os = "windows") { "dll" }
              else { "so" };

    let mut results: Vec<std::path::PathBuf> = Vec::new();

    match std::fs::read_dir(dir) {
        Ok(entries) => {
            let entries: Vec<_> = entries.filter_map(std::result::Result::ok).collect();

            for entry in &entries {
                let path = entry.path();
                if path.extension().is_some_and(|x| x == ext) {
                    results.push(path);
                }
            }

            // Subdirectories with manifest: expect lib<dirname>.<ext> inside
            for entry in &entries {
                let path = entry.path();
                if path.is_dir() {
                    let manifest_json = path.join("manifest.json");
                    let manifest_toml = path.join("manifest.toml");
                    if (manifest_json.exists() || manifest_toml.exists())
                        && let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                            let lib_name = format!("lib{dir_name}.{ext}");
                            let lib_path = path.join(&lib_name);
                            if lib_path.exists() {
                                results.push(lib_path);
                            }
                        }
                }
            }
        }
        Err(_) => {
            tracing::debug!(dir = %dir.display(), "Plugin directory not found, skipping");
        }
    }

    results
}

/// Parsed plugin metadata from `manifest.toml` (Plugin Catalog spec Section 4).
#[derive(Debug, Clone)]
#[allow(dead_code)] // Spec feature: manifest fields for plugin catalog
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub platforms: Vec<String>,
    pub conventions_count: usize,
    pub lora_included: bool,
    pub dependencies: Vec<String>,
}

/// Parse `manifest.toml` from a plugin directory, returning `None` if absent or malformed.
pub fn parse_manifest(plugin_dir: &Path) -> Option<PluginManifest> {
    let manifest_path = plugin_dir.join("manifest.toml");
    if !manifest_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&manifest_path).ok()?;
    let table: toml::Table = content.parse().ok()?;

    let plugin = table.get("plugin")?;
    let name = plugin.get("name")?.as_str()?.to_string();
    let version = plugin.get("version")?.as_str().unwrap_or("0.1.0").to_string();
    let description = plugin.get("description")?.as_str().unwrap_or("").to_string();

    let compat = table.get("compatibility");
    let platforms = compat
        .and_then(|c| c.get("platforms"))
        .and_then(|p| p.as_array()).map_or_else(|| vec!["*".to_string()], |arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect());

    // Convention count from TOML is a small non-negative integer
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let conv_count = table.get("conventions")
        .and_then(|c| c.get("count"))
        .and_then(toml::Value::as_integer)
        .unwrap_or(0) as usize;

    let lora = table.get("lora")
        .and_then(|l| l.get("included"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);

    let deps = table.get("dependencies")
        .and_then(|d| d.get("required"))
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| {
            v.get("name").and_then(|n| n.as_str()).map(String::from)
        }).collect())
        .unwrap_or_default();

    Some(PluginManifest {
        name,
        version,
        description,
        platforms,
        conventions_count: conv_count,
        lora_included: lora,
        dependencies: deps,
    })
}

/// Check if a manifest's declared platforms include the current target.
pub fn is_platform_compatible(manifest: &PluginManifest) -> bool {
    if manifest.platforms.contains(&"*".to_string()) {
        return true;
    }
    let current = if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "aarch64-macos"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "x86_64-macos"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "x86_64-linux"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "aarch64") {
        "aarch64-linux"
    } else {
        "unknown"
    };
    manifest.platforms.iter().any(|p| p == current)
}

/// Primary entry point for plugin discovery: scans a directory, parses manifests,
/// and filters out platform-incompatible plugins. Plugins without manifests are always included.
pub fn discover_plugins(dir: &Path) -> Vec<(std::path::PathBuf, Option<PluginManifest>)> {
    let mut result = Vec::new();

    for path in scan_plugin_directory(dir) {
        let parent = path.parent().unwrap_or(dir);
        let manifest = parse_manifest(parent);

        if let Some(ref m) = manifest
            && !is_platform_compatible(m) {
                tracing::debug!(
                    plugin = %m.name,
                    platforms = ?m.platforms,
                    path = %path.display(),
                    "Plugin not compatible with current platform, skipping"
                );
                continue;
            }

        result.push((path, manifest));
    }

    result
}
