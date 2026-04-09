# soma-port-push

`soma-port-push` is a `cdylib` SOMA messaging port for Firebase Cloud Messaging, Web Push delivery, and in-memory device registration.

- Port ID: `soma.push`
- Kind: `Messaging`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `send_fcm`: send an FCM notification to `device_token` with `title`, `body`, optional `data`, optional `project_id`, and caller-supplied `access_token`
- `send_webpush`: send a Web Push payload using `subscription_json`, `title`, `body`, and optional `vapid_key`
- `register_device`: upsert a device registration for `user_id`, `platform`, and `token`
- `unregister_device`: remove a device registration for `user_id` and `platform`

## Runtime Behavior

- The device registry is stored in memory and keyed by `user_id`.
- Each user can have at most one token per platform. Re-registering the same platform replaces the stored token.
- Supported platforms are `android`, `ios`, and `web`.
- The HTTP client is synchronous `reqwest::blocking` under the hood.

## Important Caveats

- `send_fcm` does not obtain OAuth credentials for you. In practice, `access_token` is required for a successful call even though validation does not force it.
- `send_webpush` currently builds an `Authorization` header with `t=placeholder`. That means the Web Push path is not a complete production VAPID implementation yet.
- Device registrations are not durable and disappear on restart.

## Production Notes

- Use `register_device` and `unregister_device` as a local contract, not as a final device registry.
- For production push delivery, you will likely want persistent token storage, retry policy, provider credential management, and a real VAPID signing flow.

## Build

```bash
cargo build
cargo test
```
