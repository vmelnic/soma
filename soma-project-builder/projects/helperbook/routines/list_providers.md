---
id: list_providers
match:
  request: list_providers
---

step soma.ports.postgres.query
  bind: sql="SELECT * FROM users WHERE role = 'provider' ORDER BY name ASC LIMIT 50"
  on_success: complete
  on_failure: abandon
