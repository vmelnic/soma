# soma-port-geo

`soma-port-geo` is a `cdylib` SOMA port for simple geospatial math and geospatial filtering.

- Port ID: `geo`
- Kind: `Custom`
- Trust level: `Trusted`
- Remote exposure: `false`
- State model: stateless, local computation

## Capabilities

- `distance`: haversine distance between `lat1`, `lon1`, `lat2`, `lon2`
- `radius_filter`: filter point entries by center point and radius
- `bounds_check`: test whether `lat` and `lon` fall inside `min_lat`, `max_lat`, `min_lon`, `max_lon`
- `geocode`: declared address-to-coordinate capability
- `reverse_geocode`: declared coordinate-to-address capability

## Current Reality

- `distance`, `radius_filter`, and `bounds_check` are fully local and usable today.
- `geocode` and `reverse_geocode` are not wired to any backend.
- Without `api_key`, those capabilities return `DependencyUnavailable`.
- Even with `api_key`, they still return an error because there is no HTTP integration behind them yet.

## Production Notes

- Treat this crate as a reliable local geometry helper, not a full geocoding integration.
- If you need geocoding, the current port surface is a good contract to keep, but the implementation still needs a real provider such as Nominatim or Google Maps.

## Build

```bash
cargo build
cargo test
```
