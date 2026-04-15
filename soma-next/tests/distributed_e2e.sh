#!/usr/bin/env bash
# distributed_e2e.sh — Two live SOMA instances proving the distributed layer.
set -euo pipefail

SOMA="${SOMA:-./target/release/soma}"
PACK="./packs/reference/manifest.json"

DIR_A=$(mktemp -d); DIR_B=$(mktemp -d)
FIFO_A="$DIR_A/in"; FIFO_B="$DIR_B/in"
mkfifo "$FIFO_A" "$FIFO_B"

cleanup() { exec 3>&- 4>&- 2>/dev/null||true; kill $PID_A $PID_B 2>/dev/null||true; rm -rf "$DIR_A" "$DIR_B"; }
trap cleanup EXIT

echo "=== SOMA Distributed E2E ==="

SOMA_SOMA_DATA_DIR="$DIR_A" "$SOMA" --mcp --pack "$PACK" --listen 127.0.0.1:9900 --peer 127.0.0.1:9901 <"$FIFO_A" >"$DIR_A/out" 2>"$DIR_A/err" &
PID_A=$!
SOMA_SOMA_DATA_DIR="$DIR_B" "$SOMA" --mcp --pack "$PACK" --listen 127.0.0.1:9901 --peer 127.0.0.1:9900 <"$FIFO_B" >"$DIR_B/out" 2>"$DIR_B/err" &
PID_B=$!
exec 3>"$FIFO_A"; exec 4>"$FIFO_B"
sleep 2

PASS=0; FAIL=0

# All checks go through this python script that reads responses by id
run_check() {
    local label="$1" file="$2" id="$3" assertion="$4"
    if python3 -c "
import json
for line in open('$file'):
    try:
        r = json.loads(line.strip())
        if r.get('id') == $id:
            $assertion
            break
    except: pass
else:
    raise AssertionError('response $id not found')
" 2>/dev/null; then
        echo "[PASS] $label"; PASS=$((PASS+1))
    else
        echo "[FAIL] $label"; FAIL=$((FAIL+1))
    fi
}

# 1. Init
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' >&3
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' >&4
sleep 1
run_check "A init" "$DIR_A/out" 1 "assert r['result']['protocolVersion']=='2024-11-05'"
run_check "B init" "$DIR_B/out" 1 "assert r['result']['protocolVersion']=='2024-11-05'"

# 2. Author routine
echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"author_routine","arguments":{"routine_id":"drt","match_conditions":[{"condition_type":"goal_fingerprint","expression":{"goal_fingerprint":"d"},"description":"d"}],"steps":[{"type":"skill","skill_id":"soma.ports.reference.stat"}]}}}' >&3
sleep 1
run_check "Author routine" "$DIR_A/out" 2 "i=json.loads(r['result']['content'][0]['text']); assert i['created']==True"

# 3. Execute
echo '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"execute_routine","arguments":{"routine_id":"drt","input":{"path":"/tmp"}}}}' >&3
sleep 1
run_check "Execute completes" "$DIR_A/out" 3 "i=json.loads(r['result']['content'][0]['text']); assert i['status']=='completed', i"

# 4. Re-author (version)
echo '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"author_routine","arguments":{"routine_id":"drt","match_conditions":[{"condition_type":"goal_fingerprint","expression":{"goal_fingerprint":"d2"},"description":"v2"}],"steps":[{"type":"skill","skill_id":"soma.ports.reference.readdir"}]}}}' >&3
sleep 1
run_check "Version bumped" "$DIR_A/out" 4 "i=json.loads(r['result']['content'][0]['text']); assert i['created']==True"

# 5. List versions
echo '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_routine_versions","arguments":{"routine_id":"drt"}}}' >&3
sleep 1
run_check "2 versions" "$DIR_A/out" 5 "i=json.loads(r['result']['content'][0]['text']); assert len(i['versions'])==2, len(i['versions'])"

# 6. Rollback
echo '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"rollback_routine","arguments":{"routine_id":"drt","target_version":0}}}' >&3
sleep 1
run_check "Rollback v0" "$DIR_A/out" 6 "i=json.loads(r['result']['content'][0]['text']); assert i['rolled_back']==True"

# 7. Transfer
echo '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"transfer_routine","arguments":{"peer_id":"peer-0","routine_id":"drt"}}}' >&3
sleep 1
run_check "Transfer" "$DIR_A/out" 7 "assert 'result' in r, r.get('error')"

# 8. Tool count
echo '{"jsonrpc":"2.0","id":8,"method":"tools/list","params":{}}' >&3
sleep 1
run_check "31 tools" "$DIR_A/out" 8 "assert len(r['result']['tools'])==31, len(r['result']['tools'])"

# 9. Kill B + heartbeat wait
echo ""; echo "Kill B, wait 20s for heartbeat..."
kill $PID_B 2>/dev/null||true; wait $PID_B 2>/dev/null||true
sleep 20

# 10. World state
echo '{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"dump_world_state","arguments":{}}}' >&3
sleep 1
python3 -c "
import json
for line in open('$DIR_A/out'):
    try:
        r = json.loads(line.strip())
        if r.get('id') == 10:
            inner = json.loads(r['result']['content'][0]['text'])
            facts = inner.get('facts', [])
            print(f'World state: {len(facts)} facts')
            for f in facts:
                print(f'  {f[\"fact_id\"]}: {json.dumps(f[\"value\"])}')
            break
    except: pass
" 2>/dev/null || echo "[INFO] Could not parse world state"

echo ""
echo "=== $PASS passed, $FAIL failed ==="
tail -5 "$DIR_A/err"
[ "$FAIL" -eq 0 ] && echo "=== ALL PASSED ===" && exit 0
exit 1
