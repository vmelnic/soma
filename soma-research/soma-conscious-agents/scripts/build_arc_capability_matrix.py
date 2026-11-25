#!/usr/bin/env python3
"""Map the reviewed ARC semantic analyses onto a fixed domain-general ontology."""
from __future__ import annotations
import concurrent.futures, importlib.util, json
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
SOURCE=ROOT/'artifacts/arc2-semantic-analysis.json'
CACHE=ROOT/'artifacts/arc-semantic-analysis/capabilities'
OUT=ROOT/'artifacts/arc2-capability-matrix.json'
BATCH=20
ONTOLOGY={
'C1':'partition a carrier into panels, bands, tiles, or regions',
'C2':'select a subobject or project/crop a carrier',
'C3':'infer coordinate charts, axes, orientation, and relative coordinates',
'C4':'form quotients/equivalence classes from repetition, symmetry, or roles',
'C5':'construct an output carrier, extent, canvas, or layout',
'C6':'represent multiple scales: values, motifs, objects, regions, scenes',
'C7':'infer contextual roles such as background, marker, guide, key, or container',
'E1':'construct entities by connectivity with inferred adjacency/value policy',
'E2':'represent shape, boundary, interior, holes, and negative space',
'E3':'discover motifs, periods, lattices, and repeated structures',
'E4':'construct lines, rays, paths, traces, and endpoints',
'E5':'construct frames, containers, compartments, and nested regions',
'R1':'test equality, congruence, isomorphism, or transformed matching',
'R2':'reason over incidence, adjacency, contact, containment, alignment, and betweenness',
'R3':'infer order, sequence, reading order, rank, and extrema',
'R4':'infer correspondence, assignment, and matching across entities or carriers',
'R5':'count and compare cardinality, size, distance, and other measures',
'R6':'infer and apply symmetry/group actions',
'R7':'reason over connectivity, paths, enclosures, and topology',
'T1':'select/filter entities by relational predicates or extrema',
'T2':'perform local rewrite, recolor, substitution, insertion, or deletion',
'T3':'apply geometric maps: translate, rotate, reflect, transpose, or scale',
'T4':'crop, project, embed, reframe, or change coordinates',
'T5':'copy, repeat, tile, extrapolate, or periodically extend',
'T6':'overlay, mask, merge, and resolve collisions/conflicts',
'T7':'compose, pack, concatenate, sort, or arrange structures',
'T8':'propagate, draw paths/rays, flood, or fill bounded regions',
'T9':'complete or repair a structure under inferred invariants',
'T10':'split, merge, or reconstruct entities and scenes',
'T11':'encode/decode between values, counts, shapes, and layouts',
'T12':'iterate, scan, fold, recurse, or compute a fixed point',
'I1':'synthesize typed executable hypotheses rather than choose task handlers',
'I2':'infer carrier/headset, ontology, correspondence, and program jointly',
'I3':'maintain competing hypotheses, uncertainty, and underdetermination',
'I4':'rank by evidence plus a complexity/description-length prior',
'I5':'validate exact predictions across all demonstrations and held-out observations',
'I6':'abstract reusable subprograms and transfer them between worlds',
'I7':'adapt search resources and invent conservative typed extensions',
'A1':'track latent state and persistent entities through observations',
'A2':'infer action semantics and affordances from interventions',
'A3':'infer goals, terminal conditions, and progress variables',
'A4':'choose information-gathering experiments',
'A5':'plan and replan in belief state over long horizons',
'A6':'assign causal credit from sparse feedback and outcomes',
'A7':'reason about risk, reversibility, and action cost',
'A8':'consolidate episodic/semantic/procedural memory and transfer across levels',
}

def load_api():
 spec=importlib.util.spec_from_file_location('sem',ROOT/'scripts/analyze_arc_semantics.py')
 sem=importlib.util.module_from_spec(spec); spec.loader.exec_module(sem); sem.load_env(); return sem

def prompt(items):
 compact=[{k:x[k] for k in ('task_id','semantic_family','hypothesis','carrier','entities','relations','program','alternatives','confidence')} for x in items]
 return '''Map each reviewed task analysis to the fixed capability ontology below.
Return only a JSON array in task order. Each item must contain task_id,
capability_ids (unique array of ontology IDs), and architecture_stressor (one
sentence naming what a too-narrow inducer would miss). Assign every capability
actually required by the described generative program, including I1, I2, I3,
I4, or I5 when ambiguity or joint inference makes it relevant. Do not create
new IDs and do not infer task facts beyond the reviewed analysis.\n\nONTOLOGY:\n'''+json.dumps(ONTOLOGY)+"\n\nTASKS:\n"+json.dumps(compact,separators=(',',':'))

def run(index,items,sem):
 CACHE.mkdir(parents=True,exist_ok=True); out=CACHE/f'batch-{index:02d}.json'
 expected=[x['task_id'] for x in items]
 if out.exists(): raw=json.loads(out.read_text())
 else:
  raw=sem.call_model(prompt(items)); out.write_text(json.dumps(raw,indent=2)+'\n')
 if isinstance(raw,dict): raw=raw.get('tasks',raw.get('items'))
 if not isinstance(raw,list): raise ValueError(f'batch {index}: not a list')
 by={x['task_id']:x for x in raw}
 if set(by)!=set(expected): raise ValueError(f'batch {index}: ID mismatch')
 for x in by.values():
  bad=set(x['capability_ids'])-set(ONTOLOGY)
  if bad: raise ValueError(f"{x['task_id']}: invalid IDs {bad}")
 return [by[x] for x in expected]

def main():
 tasks=json.loads(SOURCE.read_text())['tasks']; sem=load_api()
 chunks=[tasks[i:i+BATCH] for i in range(0,len(tasks),BATCH)]
 result=[None]*len(chunks)
 with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
  fs={pool.submit(run,i,c,sem):i for i,c in enumerate(chunks)}
  for f in concurrent.futures.as_completed(fs):
   i=fs[f]; result[i]=f.result(); print('capability batch',i,'complete',flush=True)
 flat=[x for batch in result for x in batch]
 OUT.write_text(json.dumps({'schema':'soma.arc2-capability-matrix.v1','ontology':ONTOLOGY,'tasks':flat},indent=2)+'\n')
 print('wrote',OUT,'tasks',len(flat))
if __name__=='__main__': main()
