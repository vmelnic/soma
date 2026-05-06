const BASE = import.meta.env.VITE_BRIDGE_URL || 'http://localhost:3000';

export async function routine(id, input = {}) {
  const res = await fetch(`${BASE}/${id}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(input),
  });
  const data = await res.json();
  if (!res.ok) throw new Error(data.error || `routine ${id} failed`);
  return data.result?.data ?? data;
}

export async function health() {
  const res = await fetch(`${BASE}/health`);
  return res.json();
}
