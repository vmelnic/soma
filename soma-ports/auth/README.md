# soma-port-auth

`soma-port-auth` is a `cdylib` SOMA port that provides local authentication primitives: OTP, sessions, TOTP, and bearer tokens.

- Port ID: `auth`
- Kind: `Custom`
- Trust level: `Trusted`
- Remote exposure: `false`
- State model: in-memory only

## Capabilities

- `otp_generate`: generate a 6-digit OTP for `phone`
- `otp_verify`: verify `phone` + `code`
- `session_create`: create a session token for `user_id`, with optional `device_info` and `ttl_hours`
- `session_validate`: validate a session `token`
- `session_revoke`: revoke a session `token`
- `totp_generate`: generate a base32 TOTP secret and provisioning URI for `user_id`
- `totp_verify`: verify a TOTP `secret` + `code`
- `token_generate`: create a bearer token for `user_id`, with optional `ttl_hours`
- `token_validate`: validate a bearer `token`
- `token_refresh`: extend a bearer `token` expiry, with optional `ttl_hours`

## Runtime Behavior

- OTP, session, and bearer-token state is stored in process memory behind mutexes.
- `otp_generate` returns the OTP as `debug_code`. That is useful for local testing and unacceptable for a real user-facing delivery flow.
- OTP entries expire after 5 minutes, become invalid after more than 5 verification attempts, and cannot be reused after successful verification.
- `session_create` defaults to a 720 hour TTL. `token_generate` and `token_refresh` default to 24 hours.
- Session and token values are stored as SHA-256 hashes, not plaintext.
- TOTP uses SHA-1, 6 digits, 30 second steps, and returns a provisioning URI suitable for authenticator apps.

## Production Notes

- This port is self-contained. It does not send SMS, email, or push messages.
- All auth state is lost on process restart and is not shared across instances.
- There is no persistent audit trail, device binding policy, rate limiting beyond OTP attempt count, or external identity integration.
- Use it as a local auth primitive or test double. For multi-instance production auth, pair the same API shape with durable storage and delivery infrastructure.

## Build

```bash
cargo build
cargo test
```
