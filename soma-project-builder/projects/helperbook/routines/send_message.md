---
id: send_message
match:
  request: send_message
---

step soma.ports.auth.session_validate
  bind: token=$token
  on_failure: abandon
  condition:
    match: {valid: false}
    next: abandon

step soma.ports.postgres.insert
  bind: table="messages", values={"chat_id":"$chat_id","sender_id":"$user_id","content":"$content","type":"text","status":"sent"}
  on_success: complete
  on_failure: abandon
