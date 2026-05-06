---
id: list_contacts
match:
  request: list_contacts
---

step soma.ports.auth.session_validate
  bind: token=$token
  on_failure: abandon
  condition:
    match: {valid: false}
    next: abandon

step soma.ports.postgres.query
  bind: sql="SELECT c.*, u.name, u.phone, u.bio, u.photo_url FROM connections c JOIN users u ON u.id = CASE WHEN c.requester_id::text = $1 THEN c.recipient_id ELSE c.requester_id END WHERE (c.requester_id::text = $1 OR c.recipient_id::text = $1) AND c.status = 'accepted' ORDER BY u.name ASC LIMIT 50", params=[$user_id]
  on_success: complete
  on_failure: abandon
