# soma-port-stripe

`soma-port-stripe` is a `cdylib` SOMA port that provides payment processing via the Stripe REST API.

- Port ID: `soma.stripe`
- Kind: `Payment`
- Trust level: `Verified`
- Remote exposure: `false`
- Network access: required

## Capabilities

- `create_charge`: `amount`, `currency`, `source`, `description`
- `create_customer`: `email`, `name`, `description`
- `list_charges`: `limit`, `starting_after`
- `create_payment_intent`: `amount`, `currency`, `payment_method_types`
- `get_balance`: *(no parameters)*

## Configuration

| Env var | Description |
|---|---|
| `SOMA_STRIPE_SECRET_KEY` | Stripe secret API key (primary) |
| `STRIPE_SECRET_KEY` | Stripe secret API key (fallback) |

## Build

```bash
cargo build
cargo test
```
