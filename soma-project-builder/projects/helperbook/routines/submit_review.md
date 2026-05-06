---
id: submit_review
match:
  request: submit_review
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

step soma.ports.postgres.count
  bind: table="reviews", where={"appointment_id":"$appointment_id","reviewer_id":"$user_id"}
  on_success: abandon
  on_failure: abandon
  condition:
    match: {count: 0}
    description: no existing review
    next: continue

step soma.ports.postgres.insert
  bind: table="reviews", values={"appointment_id":"$appointment_id","reviewer_id":"$user_id","reviewed_id":"$reviewed_id","rating":"$rating","feedback":"$feedback"}
  on_success: complete
  on_failure: abandon
