//! Ed25519 signature verification for dynamic port libraries.
//!
//! Each port library (`.dylib`/`.so`) may have a sidecar `.sig` file containing
//! a 64-byte Ed25519 signature over the library bytes. Verification uses a
//! trusted public key (32 bytes) provided by the caller or read from a `.pub`
//! sidecar file.
//!
//! When `[ports] require_signatures = true` in config, the dynamic loader
//! rejects any library that lacks a valid signature.

use std::path::Path;

use ed25519_dalek::{Signature, VerifyingKey, Verifier};

use crate::errors::{Result, SomaError};

/// Verify an Ed25519 signature over a port shared library.
///
/// Reads the library bytes from `dylib_path`, reads the 64-byte signature from
/// `signature_path`, and checks the signature against `public_key`.
///
/// Returns `Ok(true)` if the signature is valid, `Ok(false)` if the signature
/// is well-formed but does not match. Returns `Err` on I/O failures or if the
/// signature/key bytes have the wrong length.
pub fn verify_port_signature(
    dylib_path: &Path,
    signature_path: &Path,
    public_key: &[u8],
) -> Result<bool> {
    let key_bytes: [u8; 32] = public_key.try_into().map_err(|_| {
        SomaError::Port(format!(
            "public key must be exactly 32 bytes, got {}",
            public_key.len()
        ))
    })?;

    let verifying_key = VerifyingKey::from_bytes(&key_bytes).map_err(|e| {
        SomaError::Port(format!("invalid Ed25519 public key: {e}"))
    })?;

    let sig_bytes = std::fs::read(signature_path).map_err(|e| {
        SomaError::Port(format!(
            "failed to read signature file '{}': {e}",
            signature_path.display()
        ))
    })?;

    let sig_array: [u8; 64] = sig_bytes.try_into().map_err(|v: Vec<u8>| {
        SomaError::Port(format!(
            "signature file must be exactly 64 bytes, got {}",
            v.len()
        ))
    })?;

    let signature = Signature::from_bytes(&sig_array);

    let dylib_bytes = std::fs::read(dylib_path).map_err(|e| {
        SomaError::Port(format!(
            "failed to read port library '{}': {e}",
            dylib_path.display()
        ))
    })?;

    match verifying_key.verify(&dylib_bytes, &signature) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Check whether a `.sig` sidecar file exists for a given library path.
///
/// The sidecar is expected at `<path>.sig` — e.g. `libfoo.dylib.sig` for
/// `libfoo.dylib`.
pub fn signature_path_for(dylib_path: &Path) -> std::path::PathBuf {
    let mut sig = dylib_path.as_os_str().to_owned();
    sig.push(".sig");
    std::path::PathBuf::from(sig)
}

/// Check whether a `.pub` sidecar file exists for a given library path.
pub fn public_key_path_for(dylib_path: &Path) -> std::path::PathBuf {
    let mut pk = dylib_path.as_os_str().to_owned();
    pk.push(".pub");
    std::path::PathBuf::from(pk)
}

/// Attempt to verify a port library using sidecar `.sig` and `.pub` files.
///
/// If both sidecar files exist, reads them and verifies the signature.
/// If neither exists and `require_signatures` is false, returns `Ok(())`.
/// If signatures are required but sidecars are missing, returns an error.
pub fn check_port_signature(dylib_path: &Path, require_signatures: bool) -> Result<()> {
    let sig_path = signature_path_for(dylib_path);
    let pub_path = public_key_path_for(dylib_path);

    if sig_path.exists() && pub_path.exists() {
        let pub_key_bytes = std::fs::read(&pub_path).map_err(|e| {
            SomaError::Port(format!(
                "failed to read public key file '{}': {e}",
                pub_path.display()
            ))
        })?;

        let valid = verify_port_signature(dylib_path, &sig_path, &pub_key_bytes)?;
        if !valid {
            return Err(SomaError::Port(format!(
                "Ed25519 signature verification FAILED for '{}'",
                dylib_path.display()
            )));
        }

        tracing::info!(
            path = %dylib_path.display(),
            "port signature verified"
        );
        Ok(())
    } else if require_signatures {
        Err(SomaError::Port(format!(
            "port library '{}' has no signature (.sig/.pub sidecar files) but require_signatures is enabled",
            dylib_path.display()
        )))
    } else {
        tracing::debug!(
            path = %dylib_path.display(),
            "no signature files found, loading without verification"
        );
        Ok(())
    }
}

/// Format a human-readable verification report for a port library.
///
/// Used by the `verify-port` CLI command. Returns a multi-line string
/// describing whether the signature is valid, missing, or failed.
pub fn verify_port_report(dylib_path: &Path) -> String {
    let sig_path = signature_path_for(dylib_path);
    let pub_path = public_key_path_for(dylib_path);

    if !dylib_path.exists() {
        return format!("ERROR: port library not found: {}", dylib_path.display());
    }

    if !sig_path.exists() {
        return format!(
            "NO SIGNATURE: {}\n  expected signature at: {}\n  status: unsigned",
            dylib_path.display(),
            sig_path.display()
        );
    }

    if !pub_path.exists() {
        return format!(
            "NO PUBLIC KEY: {}\n  signature found at: {}\n  expected public key at: {}\n  status: cannot verify (missing public key)",
            dylib_path.display(),
            sig_path.display(),
            pub_path.display()
        );
    }

    match std::fs::read(&pub_path) {
        Ok(pub_key_bytes) => match verify_port_signature(dylib_path, &sig_path, &pub_key_bytes) {
            Ok(true) => format!(
                "VERIFIED: {}\n  signature: {}\n  public key: {}\n  status: valid Ed25519 signature",
                dylib_path.display(),
                sig_path.display(),
                pub_path.display()
            ),
            Ok(false) => format!(
                "FAILED: {}\n  signature: {}\n  public key: {}\n  status: Ed25519 signature does NOT match",
                dylib_path.display(),
                sig_path.display(),
                pub_path.display()
            ),
            Err(e) => format!(
                "ERROR: {}\n  {e}",
                dylib_path.display()
            ),
        },
        Err(e) => format!(
            "ERROR: failed to read public key '{}': {e}",
            pub_path.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{SigningKey, Signer};

    /// Helper: create a test keypair, sign some bytes, write the .sig and .pub files.
    fn create_signed_port(dir: &Path, lib_name: &str, content: &[u8]) -> (std::path::PathBuf, SigningKey) {
        let signing_key = SigningKey::from_bytes(&[
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
            17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
        ]);
        let verifying_key = signing_key.verifying_key();

        let lib_path = dir.join(lib_name);
        std::fs::write(&lib_path, content).unwrap();

        let signature = signing_key.sign(content);
        let sig_path = signature_path_for(&lib_path);
        std::fs::write(&sig_path, signature.to_bytes()).unwrap();

        let pub_path = public_key_path_for(&lib_path);
        std::fs::write(&pub_path, verifying_key.to_bytes()).unwrap();

        (lib_path, signing_key)
    }

    #[test]
    fn verify_valid_signature() {
        let dir = std::env::temp_dir().join("soma_port_verify_valid");
        let _ = std::fs::create_dir_all(&dir);

        let content = b"fake port library content for testing";
        let (lib_path, signing_key) = create_signed_port(&dir, "libtest_port.dylib", content);

        let sig_path = signature_path_for(&lib_path);
        let pub_key = signing_key.verifying_key().to_bytes();

        let result = verify_port_signature(&lib_path, &sig_path, &pub_key);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.unwrap());
    }

    #[test]
    fn verify_invalid_signature_returns_false() {
        let dir = std::env::temp_dir().join("soma_port_verify_invalid");
        let _ = std::fs::create_dir_all(&dir);

        let content = b"original content";
        let (lib_path, signing_key) = create_signed_port(&dir, "libtest_port.dylib", content);

        // Tamper with the library content after signing
        std::fs::write(&lib_path, b"tampered content").unwrap();

        let sig_path = signature_path_for(&lib_path);
        let pub_key = signing_key.verifying_key().to_bytes();

        let result = verify_port_signature(&lib_path, &sig_path, &pub_key);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(!result.unwrap());
    }

    #[test]
    fn verify_wrong_key_returns_false() {
        let dir = std::env::temp_dir().join("soma_port_verify_wrongkey");
        let _ = std::fs::create_dir_all(&dir);

        let content = b"some library bytes";
        let (lib_path, _signing_key) = create_signed_port(&dir, "libtest_port.dylib", content);

        // Use a different public key
        let other_key = SigningKey::from_bytes(&[
            99, 98, 97, 96, 95, 94, 93, 92, 91, 90, 89, 88, 87, 86, 85, 84,
            83, 82, 81, 80, 79, 78, 77, 76, 75, 74, 73, 72, 71, 70, 69, 68,
        ]);
        let wrong_pub = other_key.verifying_key().to_bytes();

        let sig_path = signature_path_for(&lib_path);
        let result = verify_port_signature(&lib_path, &sig_path, &wrong_pub);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(!result.unwrap());
    }

    #[test]
    fn verify_bad_key_length_returns_error() {
        let dir = std::env::temp_dir().join("soma_port_verify_badkey");
        let _ = std::fs::create_dir_all(&dir);

        let lib_path = dir.join("libtest.dylib");
        std::fs::write(&lib_path, b"content").unwrap();
        let sig_path = dir.join("libtest.dylib.sig");
        std::fs::write(&sig_path, vec![0u8; 64]).unwrap();

        let result = verify_port_signature(&lib_path, &sig_path, &[0u8; 16]);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("32 bytes"));
    }

    #[test]
    fn verify_bad_signature_length_returns_error() {
        let dir = std::env::temp_dir().join("soma_port_verify_badsig");
        let _ = std::fs::create_dir_all(&dir);

        let lib_path = dir.join("libtest.dylib");
        std::fs::write(&lib_path, b"content").unwrap();
        let sig_path = dir.join("libtest.dylib.sig");
        std::fs::write(&sig_path, vec![0u8; 32]).unwrap(); // wrong length

        let pub_key = [0u8; 32];
        let result = verify_port_signature(&lib_path, &sig_path, &pub_key);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("64 bytes"));
    }

    #[test]
    fn check_port_signature_passes_with_valid_sidecar() {
        let dir = std::env::temp_dir().join("soma_port_check_valid");
        let _ = std::fs::create_dir_all(&dir);

        let content = b"valid port bytes";
        let (lib_path, _) = create_signed_port(&dir, "libgood.dylib", content);

        let result = check_port_signature(&lib_path, true);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_ok());
    }

    #[test]
    fn check_port_signature_rejects_tampered_library() {
        let dir = std::env::temp_dir().join("soma_port_check_tampered");
        let _ = std::fs::create_dir_all(&dir);

        let content = b"original bytes";
        let (lib_path, _) = create_signed_port(&dir, "libbad.dylib", content);

        // Tamper
        std::fs::write(&lib_path, b"different bytes").unwrap();

        let result = check_port_signature(&lib_path, true);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("FAILED"));
    }

    #[test]
    fn check_port_no_sidecar_allowed_when_not_required() {
        let dir = std::env::temp_dir().join("soma_port_check_nosig_ok");
        let _ = std::fs::create_dir_all(&dir);

        let lib_path = dir.join("libunsigned.dylib");
        std::fs::write(&lib_path, b"unsigned library").unwrap();

        let result = check_port_signature(&lib_path, false);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_ok());
    }

    #[test]
    fn check_port_no_sidecar_rejected_when_required() {
        let dir = std::env::temp_dir().join("soma_port_check_nosig_err");
        let _ = std::fs::create_dir_all(&dir);

        let lib_path = dir.join("libunsigned.dylib");
        std::fs::write(&lib_path, b"unsigned library").unwrap();

        let result = check_port_signature(&lib_path, true);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("require_signatures"));
    }

    #[test]
    fn signature_path_for_appends_sig() {
        let p = Path::new("/tmp/libfoo.dylib");
        assert_eq!(signature_path_for(p), Path::new("/tmp/libfoo.dylib.sig"));
    }

    #[test]
    fn public_key_path_for_appends_pub() {
        let p = Path::new("/tmp/libfoo.so");
        assert_eq!(public_key_path_for(p), Path::new("/tmp/libfoo.so.pub"));
    }

    #[test]
    fn verify_report_missing_library() {
        let report = verify_port_report(Path::new("/tmp/nonexistent_soma_port.dylib"));
        assert!(report.starts_with("ERROR: port library not found"));
    }

    #[test]
    fn verify_report_unsigned() {
        let dir = std::env::temp_dir().join("soma_port_report_unsigned");
        let _ = std::fs::create_dir_all(&dir);

        let lib_path = dir.join("libunsigned.dylib");
        std::fs::write(&lib_path, b"unsigned").unwrap();

        let report = verify_port_report(&lib_path);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(report.starts_with("NO SIGNATURE"));
        assert!(report.contains("unsigned"));
    }

    #[test]
    fn verify_report_valid() {
        let dir = std::env::temp_dir().join("soma_port_report_valid");
        let _ = std::fs::create_dir_all(&dir);

        let content = b"signed library for report test";
        let (lib_path, _) = create_signed_port(&dir, "libsigned.dylib", content);

        let report = verify_port_report(&lib_path);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(report.starts_with("VERIFIED"));
        assert!(report.contains("valid Ed25519"));
    }

    #[test]
    fn verify_report_failed() {
        let dir = std::env::temp_dir().join("soma_port_report_failed");
        let _ = std::fs::create_dir_all(&dir);

        let content = b"original for report";
        let (lib_path, _) = create_signed_port(&dir, "libtampered.dylib", content);
        std::fs::write(&lib_path, b"tampered for report").unwrap();

        let report = verify_port_report(&lib_path);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(report.starts_with("FAILED"));
        assert!(report.contains("does NOT match"));
    }
}
