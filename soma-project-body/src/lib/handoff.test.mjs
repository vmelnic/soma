import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  handoffSubject,
  isHandoffFact,
  isClaimFact,
  buildHandoffPatch,
  buildClaimPatch,
  selectHandoffs,
  DEFAULT_HANDOFF_TTL_MS,
} from './handoff.js';

test('handoff subject + fact predicate', () => {
  assert.equal(handoffSubject('sess-1'), 'session:sess-1');
  assert.equal(isHandoffFact({ subject: 'session:x', predicate: 'handoff' }), true);
  assert.equal(isHandoffFact({ subject: 'other', predicate: 'handoff' }), false);
  assert.equal(isClaimFact({ subject: 'session:x', predicate: 'claimed_by' }), true);
});

test('buildHandoffPatch shape', () => {
  const p = buildHandoffPatch({
    sessionId: 's1',
    fromDevice: 'phone-a',
    toDevice: 'laptop-b',
    objective: 'reconcile invoices',
    now: 1000,
  });
  assert.equal(p.add_facts.length, 1);
  const f = p.add_facts[0];
  assert.equal(f.subject, 'session:s1');
  assert.equal(f.predicate, 'handoff');
  assert.equal(f.value.from_device, 'phone-a');
  assert.equal(f.value.to_device, 'laptop-b');
  assert.equal(f.value.objective, 'reconcile invoices');
  assert.equal(f.ttl_ms, DEFAULT_HANDOFF_TTL_MS);
});

test('buildHandoffPatch validates', () => {
  assert.throws(() => buildHandoffPatch({ fromDevice: 'x' }), /sessionId required/);
  assert.throws(() => buildHandoffPatch({ sessionId: 's' }), /fromDevice required/);
});

test('buildClaimPatch shape', () => {
  const p = buildClaimPatch({ sessionId: 's1', deviceId: 'laptop-b', now: 5000 });
  const f = p.add_facts[0];
  assert.equal(f.subject, 'session:s1');
  assert.equal(f.predicate, 'claimed_by');
  assert.equal(f.value.device_id, 'laptop-b');
  assert.equal(f.value.ts, 5000);
});

test('selectHandoffs partitions into open / mine / others', () => {
  const facts = [
    // Addressed to laptop-b; unclaimed
    {
      subject: 'session:s1',
      predicate: 'handoff',
      value: { from_device: 'phone-a', to_device: 'laptop-b', session_id: 's1', ts: 10 },
    },
    // Broadcast (no to_device); claimed by me
    {
      subject: 'session:s2',
      predicate: 'handoff',
      value: { from_device: 'phone-a', session_id: 's2', ts: 20 },
    },
    {
      subject: 'session:s2',
      predicate: 'claimed_by',
      value: { device_id: 'laptop-b', ts: 21 },
    },
    // Claimed by someone else
    {
      subject: 'session:s3',
      predicate: 'handoff',
      value: { from_device: 'tablet-c', session_id: 's3', ts: 30 },
    },
    {
      subject: 'session:s3',
      predicate: 'claimed_by',
      value: { device_id: 'desktop-d', ts: 31 },
    },
    // Handed off BY me — should not appear as openHandoff on my side
    {
      subject: 'session:s4',
      predicate: 'handoff',
      value: { from_device: 'laptop-b', session_id: 's4', ts: 40 },
    },
  ];

  const { openHandoffs, myClaims, othersClaimed } = selectHandoffs(facts, 'laptop-b');
  assert.deepEqual(openHandoffs.map((x) => x.session_id), ['s1']);
  assert.deepEqual(myClaims.map((x) => x.session_id), ['s2']);
  assert.deepEqual(othersClaimed.map((x) => x.session_id), ['s3']);
});

test('selectHandoffs picks the latest claim when multiple', () => {
  const facts = [
    {
      subject: 'session:s1',
      predicate: 'handoff',
      value: { from_device: 'p1', session_id: 's1', ts: 1 },
    },
    {
      subject: 'session:s1',
      predicate: 'claimed_by',
      value: { device_id: 'me', ts: 5 },
    },
    {
      subject: 'session:s1',
      predicate: 'claimed_by',
      value: { device_id: 'other', ts: 10 },
    },
  ];
  const r = selectHandoffs(facts, 'me');
  assert.equal(r.myClaims.length, 0);
  assert.equal(r.othersClaimed.length, 1);
  assert.equal(r.othersClaimed[0].claim.value.device_id, 'other');
});
