---
id: cancel_appointment
match:
  request: cancel_appointment
---

step soma.ports.auth.session_validate
  bind: token=$token
  on_failure: abandon
  condition:
    match: {valid: false}
    next: abandon

step soma.ports.postgres.find
  bind: table="appointments", id=$appointment_id
  on_failure: abandon
  condition:
    match: {found: false}
    next: abandon

step soma.ports.postgres.update
  bind: table="appointments", set={"status":"cancelled"}, where={"id":"$appointment_id"}
  on_success: complete
  on_failure: abandon
