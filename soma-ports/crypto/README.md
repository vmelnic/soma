# soma-port-crypto

`soma-port-crypto` is a stateless `cdylib` SOMA port for hashing, MACs, password hashing, symmetric encryption, RSA signatures, JWT signing, and random data generation.

- Port ID: `crypto`
- Kind: `Custom`
- Trust level: `Trusted`
- Remote exposure: `false`
- State model: stateless, per-call only

## Capabilities

- `sha256`, `sha512`: hash `data` and return hex digests
- `hmac`: HMAC-SHA256 over `data` with `key`
- `bcrypt_hash`, `bcrypt_verify`: password hashing and verification
- `aes_encrypt`, `aes_decrypt`: AES-256-GCM using `plaintext`/`ciphertext` plus `key`
- `rsa_sign`, `rsa_verify`: RSA PKCS#1 v1.5 SHA-256 signatures using PEM keys
- `jwt_sign`, `jwt_verify`: JWT signing and verification with HS256
- `random_bytes`, `random_string`: secure random output

## Input and Output Conventions

- Byte-oriented fields accept either a UTF-8 string or an array of byte values.
- Hashes, MACs, and RSA signatures are returned as hex strings.
- AES requires a 32 byte key. `aes_encrypt` prepends the random 12 byte nonce to the returned ciphertext so `aes_decrypt` can operate on one field.
- RSA expects PKCS#1 PEM input for private and public keys.
- `random_bytes.count` must be between `1` and `65536`.

## Important Semantics

- No secrets are retained between calls. All key material is caller-supplied.
- `jwt_sign` uses `HS256`.
- `jwt_verify` checks signature validity but deliberately clears required standard claims and disables `exp` validation. If you need expiry, issuer, or audience enforcement, enforce those outside this port or extend the implementation.

## Production Notes

- This crate is suitable for local deterministic crypto work and utility-style signing.
- It does not provide key management, KMS integration, secure secret storage, or hardware-backed operations.

## Build

```bash
cargo build
cargo test
```
