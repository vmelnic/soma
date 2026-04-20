// Session handoff over world-state facts.
//
// SOMA's world-state store is a cross-device bulletin board that already
// persists through `patch_world_state` + `dump_world_state`. We piggy-back
// on it to signal session handoffs between devices without needing any
// runtime protocol changes.
//
// Fact shape:
//   subject:   "session:<session_id>"
//   predicate: "handoff"
//   value:     { from_device, to_device?, session_id, objective?, ts }
//   ttl_ms:    default 600_000 (10 min); device that claims it overwrites
//              with a claim fact.
//
// A claim is a second fact on the same subject:
//   predicate: "claimed_by"
//   value:     { device_id, ts }

export const DEFAULT_HANDOFF_TTL_MS = 10 * 60 * 1000;

export function handoffSubject(sessionId) {
  return `session:${sessionId}`;
}

export function isHandoffFact(fact) {
  return (
    fact &&
    typeof fact.subject === 'string' &&
    fact.subject.startsWith('session:') &&
    fact.predicate === 'handoff'
  );
}

export function isClaimFact(fact) {
  return (
    fact &&
    typeof fact.subject === 'string' &&
    fact.subject.startsWith('session:') &&
    fact.predicate === 'claimed_by'
  );
}

/** Build a patch payload the caller can send to `patch_world_state`. */
export function buildHandoffPatch({
  sessionId,
  fromDevice,
  toDevice,
  objective,
  ttl_ms = DEFAULT_HANDOFF_TTL_MS,
  now = Date.now(),
}) {
  if (!sessionId) throw new Error('buildHandoffPatch: sessionId required');
  if (!fromDevice) throw new Error('buildHandoffPatch: fromDevice required');
  return {
    add_facts: [
      {
        subject: handoffSubject(sessionId),
        predicate: 'handoff',
        value: {
          session_id: sessionId,
          from_device: fromDevice,
          to_device: toDevice || null,
          objective: objective || null,
          ts: now,
        },
        confidence: 1.0,
        ttl_ms,
      },
    ],
  };
}

export function buildClaimPatch({ sessionId, deviceId, now = Date.now() }) {
  if (!sessionId) throw new Error('buildClaimPatch: sessionId required');
  if (!deviceId) throw new Error('buildClaimPatch: deviceId required');
  return {
    add_facts: [
      {
        subject: handoffSubject(sessionId),
        predicate: 'claimed_by',
        value: { device_id: deviceId, ts: now },
        confidence: 1.0,
        ttl_ms: DEFAULT_HANDOFF_TTL_MS,
      },
    ],
  };
}

/**
 * Select session handoffs relevant to a device from a world-state dump.
 *
 *  - openHandoffs:   broadcast or addressed to me, not yet claimed
 *  - myClaims:       I've already claimed these
 *  - othersClaimed:  claimed by someone else (for display)
 */
export function selectHandoffs(facts, myDeviceId) {
  const handoffs = new Map(); // session_id -> handoff fact
  const claims = new Map();   // session_id -> claim fact (latest wins)
  const sessionIdOf = (subject) => subject.startsWith('session:') ? subject.slice(8) : null;

  for (const f of facts || []) {
    if (isHandoffFact(f)) {
      const sid = sessionIdOf(f.subject);
      if (sid) handoffs.set(sid, f);
    } else if (isClaimFact(f)) {
      const sid = sessionIdOf(f.subject);
      if (!sid) continue;
      const prev = claims.get(sid);
      const prevTs = prev?.value?.ts ?? 0;
      const curTs = f.value?.ts ?? 0;
      if (curTs >= prevTs) claims.set(sid, f);
    }
  }

  const openHandoffs = [];
  const myClaims = [];
  const othersClaimed = [];
  for (const [sid, hf] of handoffs) {
    const claim = claims.get(sid);
    const claimedByMe = claim?.value?.device_id === myDeviceId;
    const claimedByOther = claim && !claimedByMe;
    const addressed = hf.value?.to_device;
    const addressedToMe = !addressed || addressed === myDeviceId;
    const fromMe = hf.value?.from_device === myDeviceId;
    if (claimedByMe) {
      myClaims.push({ session_id: sid, handoff: hf, claim });
    } else if (claimedByOther) {
      othersClaimed.push({ session_id: sid, handoff: hf, claim });
    } else if (addressedToMe && !fromMe) {
      openHandoffs.push({ session_id: sid, handoff: hf });
    }
  }
  return { openHandoffs, myClaims, othersClaimed };
}
