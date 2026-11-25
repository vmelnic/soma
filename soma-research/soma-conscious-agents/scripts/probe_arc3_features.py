#!/usr/bin/env python3
"""Probe published ARC-AGI-3 environments through their official runtime boundary.

The probe records only observable transitions from the seed-0 initial state. It
does not inspect private state and does not claim to solve an environment.
Run with the Python environment from the official ARC-AGI toolkit.
"""
from __future__ import annotations
import hashlib, json, logging, sys, uuid
from collections import deque
from itertools import combinations
from pathlib import Path
import numpy as np

ROOT=Path(__file__).resolve().parents[1]
ARC3=ROOT/'data/external/ARC-AGI-3'
TOOLKIT=Path('/tmp/arc-agi-official')
sys.path.insert(0,str(TOOLKIT))
from arc_agi.local_wrapper import LocalEnvironmentWrapper
from arc_agi.models import EnvironmentInfo
from arcengine import GameAction

OUT=ROOT/'artifacts/arc3-observable-probes.json'
LOG=logging.getLogger('arc3-probe'); LOG.addHandler(logging.NullHandler()); LOG.setLevel(logging.CRITICAL)

def metadata(folder): return json.loads((folder/'metadata.json').read_text())
def make(folder):
 m=metadata(folder); info=EnvironmentInfo(**m,local_dir=str(folder))
 return LocalEnvironmentWrapper(info,LOG,'probe-'+uuid.uuid4().hex,seed=0)
def last_frame(obs):
 return np.asarray(obs.frame[-1],dtype=np.int8) if obs is not None and obs.frame else None
def digest(frame): return hashlib.sha256(frame.tobytes()).hexdigest() if frame is not None else None
def delta(a,b):
 if a is None or b is None: return None
 ys,xs=np.where(a!=b)
 return {'changed_cells':int(len(xs)),'bbox':None if not len(xs) else [int(xs.min()),int(ys.min()),int(xs.max()),int(ys.max())],
         'colors_added':sorted(int(x) for x in set(np.unique(b))-set(np.unique(a))),
         'colors_removed':sorted(int(x) for x in set(np.unique(a))-set(np.unique(b)))}
def run(folder,sequence):
 env=make(folder); initial=env.observation_space; first=last_frame(initial); observations=[]
 for action,data in sequence:
  obs=env.step(action,data=data); observations.append(obs)
  if obs is None: break
 final=last_frame(observations[-1]) if observations else first
 obs=observations[-1] if observations else initial
 return {'digest':digest(final),'delta':delta(first,final),'frames':sum(len(x.frame) for x in observations if x is not None),
         'state':str(obs.state.value),'levels_completed':int(obs.levels_completed),
         'available_actions':[GameAction.from_id(x).name for x in obs.available_actions],
         'step_digests':[digest(last_frame(x)) for x in observations if x is not None]}
def component_clicks(frame):
 h,w=frame.shape; seen=set(); pts={(x,y) for y in range(4,h,8) for x in range(4,w,8)}
 for y in range(h):
  for x in range(w):
   if (y,x) in seen: continue
   v=frame[y,x]; q=deque([(y,x)]); seen.add((y,x)); cells=[]
   while q:
    cy,cx=q.popleft(); cells.append((cy,cx))
    for ny,nx in ((cy-1,cx),(cy+1,cx),(cy,cx-1),(cy,cx+1)):
     if 0<=ny<h and 0<=nx<w and (ny,nx) not in seen and frame[ny,nx]==v:
      seen.add((ny,nx)); q.append((ny,nx))
   if len(cells)<h*w//2:
    ys=[p[0] for p in cells]; xs=[p[1] for p in cells]
    pts.add((round(sum(xs)/len(xs)),round(sum(ys)/len(ys))))
    pts.add((min(xs),min(ys))); pts.add((max(xs),max(ys)))
 return sorted((max(0,min(w-1,x)),max(0,min(h-1,y))) for x,y in pts)[:160]
def analyze(folder):
 env=make(folder); initial=env.observation_space; frame=last_frame(initial)
 actions=list(env.action_space); simple=[a for a in actions if a != GameAction.ACTION6]
 one={}
 for action in simple: one[action.name]=run(folder,[(action,{})])
 repeat={}
 for action in simple:
  r=run(folder,[(action,{}),(action,{})]); a=one[action.name]
  repeat[action.name]={'second_step_changes_state':len(r['step_digests'])==2 and r['step_digests'][1]!=r['step_digests'][0],
                       'two_step':r}
 noncomm=[]
 for a,b in combinations([x for x in simple if x != GameAction.ACTION7],2):
  ab=run(folder,[(a,{}),(b,{})]); ba=run(folder,[(b,{}),(a,{})])
  if ab['digest']!=ba['digest']:
   noncomm.append([a.name,b.name])
 click_summary=None
 if GameAction.ACTION6 in actions:
  probes=[]
  for x,y in component_clicks(frame):
   r=run(folder,[(GameAction.ACTION6,{'x':x,'y':y})])
   if r['delta'] and r['delta']['changed_cells']:
    probes.append({'x':x,'y':y,**r})
  click_summary={'points_tested':len(component_clicks(frame)),'responsive_points':len(probes),
                 'unique_outcomes':len({p['digest'] for p in probes}),'examples':probes[:12]}
 return {'game_id':metadata(folder)['game_id'],'initial_digest':digest(frame),'initial_actions':[a.name for a in actions],
         'one_step':one,'repeat':repeat,'noncommuting_action_pairs':noncomm,'click_probe':click_summary}
def main():
 folders=sorted(p for p in ARC3.iterdir() if p.is_dir())
 result=[]
 for i,f in enumerate(folders,1):
  item=analyze(f); result.append(item); print(f"probed {i}/{len(folders)} {item['game_id']}",flush=True)
 OUT.write_text(json.dumps({'schema':'soma.arc3-observable-probes.v1','method':'seed-0 reset; one/two-step actions; component-centroid and 8-pixel click lattice','environment_count':len(result),'environments':result},indent=2)+'\n')
 print('wrote',OUT)
if __name__=='__main__': main()
