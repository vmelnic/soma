---
id: list_appointments
match:
  request: list_appointments
---

step soma.ports.auth.session_validate
  bind: token=$token
  on_failure: abandon
  condition:
    match: {valid: false}
    next: abandon

step soma.ports.postgres.query
  bind: sql="SELECT * FROM appointments WHERE client_id::text = $1 OR provider_id::text = $1 ORDER BY start_time DESC LIMIT 50", params=[$user_id]
  on_success: complete
  on_failure: abandon
