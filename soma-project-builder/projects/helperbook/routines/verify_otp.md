---
id: verify_otp
match:
  request: verify_otp
---

step soma.ports.auth.otp_verify
  bind: phone=$phone, code=$code
  on_failure: abandon
  condition:
    match: {valid: false}
    next: abandon

step soma.ports.auth.session_create
  bind: user_id=$user_id
  on_success: complete
  on_failure: abandon
