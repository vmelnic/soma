//! Dynamic plugin loading from .so/.dylib files (Section 5.3).
//!
//! Plugins are compiled as cdylib crates that export a C ABI init function.
//! The function signature is:
//!   extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin
//!
//! Example plugin crate:
//!   #[no_mangle]
//!   pub extern "C" fn soma_plugin_init() -> *mut dyn SomaPlugin {
//!       Box::into_raw(Box::new(MyPlugin::new()))
//!   }

use anyhow::{Context, Result};
use std::path::Path;

use super::interface::SomaPlugin;

/// Verify Ed25519 signature of a plugin file (Section 20.4).
/// `signature` contains 64-byte raw Ed25519 signature.
/// `public_key` is the signer's 32-byte Ed25519 public key.
pub fn verify_plugin_signature(
    plugin_path: &Path,
    signature: &[u8; 64],
    public_key: &[u8; 32],
) -> bool {
    let Ok(plugin_bytes) = std::fs::read(plugin_path) else { return false };
    crate::protocol::encryption::verify(public_key, &plugin_bytes, signature)
}

/// Load a plugin from a shared library file.
/// The library must export `soma_plugin_init` returning a raw pointer to SomaPlugin.
pub fn load_plugin_from_path(path: &Path) -> Result<Box<dyn SomaPlugin>> {
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

    // Keep the library alive — leak it intentionally.
    // The plugin uses symbols from the library, so it must stay loaded.
    std::mem::forget(lib);

    tracing::info!(
        plugin = plugin.name(),
        version = plugin.version(),
        path = %path.display(),
        "Dynamic plugin loaded"
    );

    Ok(plugin)
}

/// Scan a directory for .so/.dylib plugin files.
pub fn scan_plugin_directory(dir: &Path) -> Vec<std::path::PathBuf> {
    let ext = if cfg!(target_os = "macos") { "dylib" }
              else if cfg!(target_os = "windows") { "dll" }
              else { "so" };

    match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().extension()
                    .map(|x| x == ext)
                    .unwrap_or(false)
            })
            .map(|e| e.path())
            .collect(),
        Err(_) => {
            tracing::debug!(dir = %dir.display(), "Plugin directory not found, skipping");
            Vec::new()
        }
    }
}

/// Plugin manifest parsed from manifest.toml (05_PLUGIN_CATALOG.md Section 4).
#[derive(Debug, Clone)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub platforms: Vec<String>,
    pub conventions_count: usize,
    pub lora_included: bool,
    pub dependencies: Vec<String>,
}

/// Parse a plugin manifest.toml file.
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
        .and_then(|p| p.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_else(|| vec!["*".to_string()]);

    let conv_count = table.get("conventions")
        .and_then(|c| c.get("count"))
        .and_then(|c| c.as_integer())
        .unwrap_or(0) as usize;

    let lora = table.get("lora")
        .and_then(|l| l.get("included"))
        .and_then(|i| i.as_bool())
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

/// Check if a plugin manifest is compatible with the current platform.
pub fn is_platform_compatible(manifest: &PluginManifest) -> bool {
    if manifest.platforms.contains(&"*".to_string()) {
        return true;
    }
    let current = if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "aarch64-macos"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "x86_64-linux"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "aarch64") {
        "aarch64-linux"
    } else {
        "unknown"
    };
    manifest.platforms.iter().any(|p| p == current)
}

/// Discover plugins from a directory, reading manifests for metadata.
pub fn discover_plugins(dir: &Path) -> Vec<(std::path::PathBuf, Option<PluginManifest>)> {
    let mut result = Vec::new();

    // Check for .so/.dylib files
    for path in scan_plugin_directory(dir) {
        let parent = path.parent().unwrap_or(dir);
        let manifest = parse_manifest(parent);
        result.push((path, manifest));
    }

    // Also check subdirectories for manifest.toml + binary
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let subdir = entry.path();
            if subdir.is_dir() {
                if let Some(manifest) = parse_manifest(&subdir) {
                    let ext = if cfg!(target_os = "macos") { "dylib" }
                              else if cfg!(target_os = "windows") { "dll" }
                              else { "so" };
                    let binary = subdir.join(format!("plugin.{}", ext));
                    if binary.exists() {
                        result.push((binary, Some(manifest)));
                    }
                }
            }
        }
    }

    result
}
