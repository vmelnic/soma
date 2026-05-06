---
id: book_appointment
match:
  request: book_appointment
---

step soma.ports.auth.session_validate
  bind: token=$token
  on_failure: abandon
  condition:
    match: {valid: false}
    next: abandon

step soma.ports.postgres.find
  bind: table="users", id=$provider_id
  on_failure: abandon
  condition:
    match: {found: false}
    next: abandon

step soma.ports.postgres.count
  bind: table="appointments", where={"provider_id":"$provider_id","status":"confirmed"}
  on_success: abandon
  on_failure: abandon
  condition:
    match: {count: 0}
    description: no conflicts
    next: continue

step soma.ports.postgres.insert
  bind: table="appointments", values={"creator_id":"$user_id","client_id":"$user_id","provider_id":"$provider_id","service":"$service","start_time":"$start_time","end_time":"$end_time","location":"$location","rate_amount":"$rate_amount","rate_type":"$rate_type","notes":"$notes","status":"proposed"}
  on_success: complete
  on_failure: abandon
