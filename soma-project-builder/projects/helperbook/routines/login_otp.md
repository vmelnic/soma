---
id: login_otp
match:
  request: login_otp
---

step soma.ports.auth.otp_generate
  bind: phone=$phone
  on_failure: abandon

step soma.smtp.send_plain
  bind: to=$email, subject="Your HelperBook verification code", body=$debug_code
  on_success: complete
  on_failure: complete
