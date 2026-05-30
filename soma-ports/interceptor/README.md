# soma-port-interceptor

`soma-port-interceptor` is a `cdylib` SOMA port that simulates autonomous drone interception with 3D kinematics, sensor fusion, and guidance laws.

- Port ID: `interceptor`
- Kind: `Custom`
- Trust level: `Trusted`
- Remote exposure: `false`
- Network access: not required

## Capabilities

- Sensors: `rf_scan`, `visual_detect`, `ir_detect`, `acoustic_detect`, `imu_read`, `gps_read`
- Fusion: `fuse_target_state`
- Flight control: `set_heading`, `set_throttle`, `set_altitude`, `set_waypoint`
- Guidance: `compute_intercept_vector`, `proportional_navigation`, `lead_pursuit`, `pure_pursuit`
- Engagement: `arm`, `disarm`, `detonate_proximity`, `abort_engagement`
- Reporting: `report_kill`, `report_miss`
- C2: `beacon_status`, `receive_tasking`, `share_target`, `upload_episode`

## Build

```bash
cargo build
cargo test
```
