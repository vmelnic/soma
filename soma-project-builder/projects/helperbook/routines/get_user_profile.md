---
id: get_user_profile
match:
  request: get_user_profile
---

step soma.ports.auth.session_validate
  bind: token=$token
  on_failure: abandon
  condition:
    match: {valid: false}
    next: abandon

step soma.ports.postgres.find
  bind: table="users", id=$id
  on_success: complete
  on_failure: abandon
