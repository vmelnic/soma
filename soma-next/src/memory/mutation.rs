//! Mutation operators — heritable variation over routines.
//!
//! A routine is SOMA's unit of selection: a serializable program with a niche
//! (match conditions), a phenotype (compiled steps), and a fitness (confidence
//! and model evidence). The reactive monitor already provides selection —
//! routines that keep failing are decayed and invalidated. What it lacks is
//! variation: every routine comes from experience or human authoring, never
//! from perturbing an existing one. These operators supply that missing piece.
//!
//! Domain-agnostic by construction. Operators transform routine *structure*,
//! never domain content. The skill "alphabet" used for substitution and
//! insertion is supplied by the caller — the gene pool's own skills — so this
//! module knows nothing about any specific domain.
//!
//! Containment: a mutant never alters `policy_scope`. The sandbox a routine
//! runs inside is not a gene; mutation cannot widen the policy that contains
//! it. Every operator inherits `policy_scope` from the parent verbatim.

use crate::types::routine::{CompiledStep, NextStep, Routine, RoutineOrigin};

/// Marker embedded in a mutant's id to record lineage in the id itself, e.g.
/// `dna.orient~mp1a3f`. Recoverable with [`parent_of`].
const MUTATION_MARKER: &str = "~m";

/// Deterministic PRNG (SplitMix64). Mutation is reproducible given the same
/// seed, which keeps tests stable and makes a run's lineage replayable. Avoids
/// pulling in a `rand` dependency.
struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform index in `0..n`. Returns 0 when `n == 0`.
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }
}

/// Tunables for a mutation pass.
#[derive(Debug, Clone)]
pub struct MutationConfig {
    /// Confidence assigned to every mutant. A mutant must earn its fitness
    /// through selection; it never inherits the parent's standing.
    pub probation_confidence: f64,
    /// Maximum mutants returned from a single [`mutate`] call.
    pub max_offspring: usize,
}

impl Default for MutationConfig {
    fn default() -> Self {
        Self {
            probation_confidence: 0.5,
            max_offspring: 4,
        }
    }
}

/// Recover the immediate parent id of a mutant, or `None` if `routine_id` is
/// not a mutant id. Lineage chains (`a~mp1~mi2`) resolve to the direct parent.
pub fn parent_of(routine_id: &str) -> Option<&str> {
    routine_id
        .rfind(MUTATION_MARKER)
        .map(|i| &routine_id[..i])
}

/// True if a mutant is structurally usable: at least one step, and every step
/// references a non-empty target. Whether the referenced skills actually exist
/// in the live registry is the breeding loop's check, not this module's.
pub fn is_structurally_valid(r: &Routine) -> bool {
    let steps = r.effective_steps();
    if steps.is_empty() {
        return false;
    }
    steps.iter().all(|s| match s {
        CompiledStep::Skill { skill_id, .. } => !skill_id.is_empty(),
        CompiledStep::SubRoutine { routine_id, .. } => !routine_id.is_empty(),
    })
}

/// Build a fresh mutant id under the parent, recording lineage in the id.
fn child_id(parent_id: &str, tag: char, rng: &mut Rng) -> String {
    format!("{parent_id}{MUTATION_MARKER}{tag}{:04x}", rng.next_u64() & 0xffff)
}

/// Clone the parent into a probation mutant: fresh id, `Mutated` origin, reset
/// confidence and model evidence, bumped version. `policy_scope` is inherited
/// verbatim through the clone and is never touched — containment.
fn spawn(parent: &Routine, id: String, cfg: &MutationConfig) -> Routine {
    let mut child = parent.clone();
    child.routine_id = id;
    child.origin = RoutineOrigin::Mutated;
    child.confidence = cfg.probation_confidence;
    child.model_evidence = 0.0;
    child.version = parent.version.saturating_add(1);
    child
}

/// Normalize a parent's phenotype to explicit steps, so all mutants are
/// expressed in `compiled_steps` form regardless of the parent's encoding.
fn express(child: &mut Routine, steps: Vec<CompiledStep>) {
    child.compiled_steps = steps;
    // compiled_steps takes precedence; clear the legacy path so it can't drift.
    child.compiled_skill_path = Vec::new();
}

/// Swap one skill step's `skill_id` for a different skill drawn from the
/// alphabet (the gene pool's available skills).
fn point_mutate_skill(
    parent: &Routine,
    alphabet: &[String],
    cfg: &MutationConfig,
    rng: &mut Rng,
) -> Option<Routine> {
    if alphabet.is_empty() {
        return None;
    }
    let mut steps = parent.effective_steps();
    let skill_idxs: Vec<usize> = steps
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s, CompiledStep::Skill { .. }))
        .map(|(i, _)| i)
        .collect();
    if skill_idxs.is_empty() {
        return None;
    }
    let pick = skill_idxs[rng.below(skill_idxs.len())];
    let current = match &steps[pick] {
        CompiledStep::Skill { skill_id, .. } => skill_id.clone(),
        _ => return None,
    };
    let candidates: Vec<&String> = alphabet.iter().filter(|s| **s != current).collect();
    if candidates.is_empty() {
        return None;
    }
    let new_skill = candidates[rng.below(candidates.len())].clone();
    if let CompiledStep::Skill { skill_id, .. } = &mut steps[pick] {
        *skill_id = new_skill;
    }
    let mut child = spawn(parent, child_id(&parent.routine_id, 'p', rng), cfg);
    express(&mut child, steps);
    Some(child)
}

/// Insert a new skill step (drawn from the alphabet) at a random position.
fn insert_step(
    parent: &Routine,
    alphabet: &[String],
    cfg: &MutationConfig,
    rng: &mut Rng,
) -> Option<Routine> {
    if alphabet.is_empty() {
        return None;
    }
    let mut steps = parent.effective_steps();
    let skill = alphabet[rng.below(alphabet.len())].clone();
    let pos = rng.below(steps.len() + 1);
    steps.insert(
        pos,
        CompiledStep::Skill {
            skill_id: skill,
            on_success: NextStep::Continue,
            on_failure: NextStep::Abandon,
            conditions: vec![],
            input_overrides: Default::default(),
        },
    );
    let mut child = spawn(parent, child_id(&parent.routine_id, 'i', rng), cfg);
    express(&mut child, steps);
    Some(child)
}

/// Delete a random step. No-op when one or zero steps remain — a routine must
/// keep a phenotype.
fn delete_step(
    parent: &Routine,
    _alphabet: &[String],
    cfg: &MutationConfig,
    rng: &mut Rng,
) -> Option<Routine> {
    let mut steps = parent.effective_steps();
    if steps.len() <= 1 {
        return None;
    }
    let pos = rng.below(steps.len());
    steps.remove(pos);
    let mut child = spawn(parent, child_id(&parent.routine_id, 'd', rng), cfg);
    express(&mut child, steps);
    Some(child)
}

/// Perturb one input-override value on a skill step: numbers nudge by ±1,
/// bools flip. Strings and structured values are left alone — there is no
/// domain-agnostic way to edit them meaningfully.
fn perturb_input(
    parent: &Routine,
    _alphabet: &[String],
    cfg: &MutationConfig,
    rng: &mut Rng,
) -> Option<Routine> {
    let mut steps = parent.effective_steps();
    let mut targets: Vec<(usize, String)> = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        if let CompiledStep::Skill { input_overrides, .. } = s {
            for k in input_overrides.keys() {
                targets.push((i, k.clone()));
            }
        }
    }
    if targets.is_empty() {
        return None;
    }
    let (idx, key) = targets[rng.below(targets.len())].clone();
    let flip_down = (rng.next_u64() & 1) == 0;
    if let CompiledStep::Skill { input_overrides, .. } = &mut steps[idx] {
        match input_overrides.get_mut(&key) {
            Some(serde_json::Value::Number(n)) => {
                // Preserve the JSON number type: integers stay integers.
                let new_num = if let Some(i) = n.as_i64() {
                    let delta = if flip_down { -1 } else { 1 };
                    serde_json::Value::Number((i + delta).into())
                } else {
                    let f = n.as_f64()?;
                    let delta = if flip_down { -1.0 } else { 1.0 };
                    serde_json::Value::Number(serde_json::Number::from_f64(f + delta)?)
                };
                input_overrides.insert(key, new_num);
            }
            Some(serde_json::Value::Bool(b)) => {
                let flipped = !*b;
                input_overrides.insert(key, serde_json::Value::Bool(flipped));
            }
            _ => return None,
        }
    }
    let mut child = spawn(parent, child_id(&parent.routine_id, 'v', rng), cfg);
    express(&mut child, steps);
    Some(child)
}

/// Widen the niche: drop one match condition so the routine fires more
/// broadly. Only when more than one condition remains — never produce a
/// fire-on-everything reflex. Narrowing is intentionally omitted: inventing a
/// new condition would require domain knowledge this module must not hold.
fn niche_widen(
    parent: &Routine,
    _alphabet: &[String],
    cfg: &MutationConfig,
    rng: &mut Rng,
) -> Option<Routine> {
    if parent.match_conditions.len() <= 1 {
        return None;
    }
    let mut conds = parent.match_conditions.clone();
    let pos = rng.below(conds.len());
    conds.remove(pos);
    let mut child = spawn(parent, child_id(&parent.routine_id, 'n', rng), cfg);
    child.match_conditions = conds;
    Some(child)
}

/// Operator signature. `alphabet` is ignored by operators that don't need it.
type Operator = fn(&Routine, &[String], &MutationConfig, &mut Rng) -> Option<Routine>;

const OPERATORS: [Operator; 5] = [
    point_mutate_skill,
    insert_step,
    delete_step,
    perturb_input,
    niche_widen,
];

/// Produce up to `cfg.max_offspring` distinct, structurally valid mutants of
/// `parent`. Each operator is applied once and skipped when inapplicable. The
/// `alphabet` is the set of skill ids the gene pool already uses, supplied by
/// the caller so this stays domain-agnostic.
pub fn mutate(
    parent: &Routine,
    alphabet: &[String],
    cfg: &MutationConfig,
    seed: u64,
) -> Vec<Routine> {
    let mut rng = Rng::new(seed);
    let mut out: Vec<Routine> = Vec::new();
    for op in OPERATORS {
        if out.len() >= cfg.max_offspring {
            break;
        }
        if let Some(child) = op(parent, alphabet, cfg, &mut rng)
            && is_structurally_valid(&child)
            && !out.iter().any(|r| r.routine_id == child.routine_id)
        {
            out.push(child);
        }
    }
    out
}

/// Recombine two parents: first half of A's steps followed by the second half
/// of B's. The child inherits A's niche, namespace, and `policy_scope`. Both
/// parents must have a phenotype. Returns `None` if the result is empty.
pub fn crossover(
    a: &Routine,
    b: &Routine,
    cfg: &MutationConfig,
    seed: u64,
) -> Option<Routine> {
    let mut rng = Rng::new(seed);
    let a_steps = a.effective_steps();
    let b_steps = b.effective_steps();
    if a_steps.is_empty() || b_steps.is_empty() {
        return None;
    }
    let a_cut = a_steps.len() / 2;
    let b_cut = b_steps.len() / 2;
    let mut steps: Vec<CompiledStep> = a_steps[..a_cut].to_vec();
    steps.extend_from_slice(&b_steps[b_cut..]);
    if steps.is_empty() {
        return None;
    }
    let id = format!("{}{}x{:04x}", a.routine_id, MUTATION_MARKER, rng.next_u64() & 0xffff);
    let mut child = spawn(a, id, cfg);
    express(&mut child, steps);
    is_structurally_valid(&child).then_some(child)
}

/// Failure-directed mutation: replace the specific skill that failed with a
/// different one from the alphabet, instead of perturbing a random part of the
/// routine. Used to *rescue* a routine before it is invalidated — directed
/// variation aimed at the observed failure, still subject to selection (each
/// variant must earn its fitness). Returns one variant per alternative skill,
/// up to `max_offspring`. Empty when the routine doesn't use `failed_skill` or
/// no alternative exists.
///
/// This is the heuristic precursor to brain-directed (Lamarckian) mutation: the
/// direction comes from which skill failed, not from a reasoning model.
pub fn guided_mutate(
    parent: &Routine,
    failed_skill: &str,
    alphabet: &[String],
    cfg: &MutationConfig,
    seed: u64,
) -> Vec<Routine> {
    let steps = parent.effective_steps();
    let uses_failed = steps.iter().any(|s| {
        matches!(s, CompiledStep::Skill { skill_id, .. } if skill_id == failed_skill)
    });
    if !uses_failed {
        return Vec::new();
    }
    let mut rng = Rng::new(seed);
    let mut out: Vec<Routine> = Vec::new();
    for replacement in alphabet.iter().filter(|s| s.as_str() != failed_skill) {
        if out.len() >= cfg.max_offspring {
            break;
        }
        let new_steps: Vec<CompiledStep> = steps
            .iter()
            .cloned()
            .map(|s| match s {
                CompiledStep::Skill {
                    skill_id,
                    on_success,
                    on_failure,
                    conditions,
                    input_overrides,
                } if skill_id == failed_skill => CompiledStep::Skill {
                    skill_id: replacement.clone(),
                    on_success,
                    on_failure,
                    conditions,
                    input_overrides,
                },
                other => other,
            })
            .collect();
        let mut child = spawn(parent, child_id(&parent.routine_id, 'r', &mut rng), cfg);
        express(&mut child, new_steps);
        if is_structurally_valid(&child) {
            out.push(child);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Breeding — selection-for. The reactive monitor already decays confidence on
// failure; these are the pieces of the symmetric half: reinforce on success,
// and breed offspring from routines that have earned it.
// ---------------------------------------------------------------------------

/// Reinforce a routine's confidence on success, capped at a ceiling. The
/// opposite of the monitor's failure decay.
pub fn reinforced_confidence(confidence: f64, factor: f64, ceiling: f64) -> f64 {
    (confidence * factor).min(ceiling)
}

/// Build the mutation alphabet from a routine population: every distinct skill
/// id appearing in any routine's steps. Domain-agnostic — the gene pool
/// supplies its own vocabulary, so the operators never need to know a domain.
pub fn skill_alphabet(routines: &[&Routine]) -> Vec<String> {
    let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for r in routines {
        for step in r.effective_steps() {
            if let CompiledStep::Skill { skill_id, .. } = step
                && !skill_id.is_empty()
            {
                set.insert(skill_id);
            }
        }
    }
    set.into_iter().collect()
}

/// Deterministic per-event seed from a routine id and a monotonic tick, so a
/// breeding run is replayable without a global RNG.
pub fn seed_for(routine_id: &str, tick: u64) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
    for b in routine_id.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^ tick.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// When a routine is allowed to breed.
#[derive(Debug, Clone)]
pub struct BreedingPolicy {
    /// Consecutive successes required before a routine spawns offspring.
    pub breed_threshold: u32,
    /// Minimum confidence to breed — reproduction proportional to fitness.
    pub confidence_breed_floor: f64,
    /// Hard cap on the autonomous-routine population; breeding stops above it.
    pub population_cap: usize,
    /// How offspring are produced.
    pub mutation: MutationConfig,
}

impl Default for BreedingPolicy {
    fn default() -> Self {
        Self {
            breed_threshold: 3,
            confidence_breed_floor: 0.7,
            population_cap: 256,
            mutation: MutationConfig::default(),
        }
    }
}

/// Whether a proven mutant should be shared with peers (horizontal gene
/// transfer). Only evolved genes (`Mutated` origin) that have climbed to the
/// fitness floor are worth broadcasting, and each only once.
pub fn should_broadcast(routine: &Routine, floor: f64, already_sent: bool) -> bool {
    routine.origin == RoutineOrigin::Mutated && routine.confidence >= floor && !already_sent
}

/// Whether a routine that just succeeded should breed now. Only autonomous,
/// high-confidence routines that have sustained success breed, and only while
/// the population is under the cap.
pub fn should_breed(
    policy: &BreedingPolicy,
    parent: &Routine,
    consecutive_successes: u32,
    population: usize,
) -> bool {
    parent.autonomous
        && consecutive_successes >= policy.breed_threshold
        && parent.confidence >= policy.confidence_breed_floor
        && population < policy.population_cap
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::common::Precondition;

    fn skill(id: &str) -> CompiledStep {
        CompiledStep::Skill {
            skill_id: id.to_string(),
            on_success: NextStep::Continue,
            on_failure: NextStep::Abandon,
            conditions: vec![],
            input_overrides: Default::default(),
        }
    }

    fn cond(key: &str) -> Precondition {
        Precondition {
            condition_type: "world_state".to_string(),
            expression: serde_json::json!({ key: true }),
            description: format!("fires on {key}"),
        }
    }

    fn parent() -> Routine {
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("max_count".to_string(), serde_json::json!(3));
        overrides.insert("recurse".to_string(), serde_json::json!(true));
        Routine {
            routine_id: "dna.probe".to_string(),
            namespace: "soma.dna".to_string(),
            origin: RoutineOrigin::PackAuthored,
            match_conditions: vec![cond("event.detected"), cond("novelty.detected")],
            compiled_skill_path: Vec::new(),
            compiled_steps: vec![
                CompiledStep::Skill {
                    skill_id: "git.status".to_string(),
                    on_success: NextStep::Continue,
                    on_failure: NextStep::Abandon,
                    conditions: vec![],
                    input_overrides: overrides,
                },
                skill("git.diff"),
                skill("git.log"),
            ],
            guard_conditions: Vec::new(),
            expected_cost: 0.1,
            expected_effect: Vec::new(),
            confidence: 0.9,
            autonomous: true,
            priority: 10,
            exclusive: false,
            policy_scope: Some("sandbox".to_string()),
            version: 2,
            model_evidence: 0.4,
        }
    }

    const ALPHABET: &[&str] = &["git.status", "git.diff", "git.log", "git.branch_list", "search.grep"];

    fn alphabet() -> Vec<String> {
        ALPHABET.iter().map(|s| s.to_string()).collect()
    }

    /// Every mutant, regardless of operator, must carry probation confidence,
    /// Mutated origin, a bumped version, an id under the parent — and must
    /// never alter the parent's policy_scope (containment).
    fn assert_probation(child: &Routine, p: &Routine) {
        assert_eq!(child.origin, RoutineOrigin::Mutated);
        assert_eq!(child.confidence, 0.5);
        assert_eq!(child.model_evidence, 0.0);
        assert_eq!(child.version, p.version + 1);
        assert_eq!(parent_of(&child.routine_id), Some(p.routine_id.as_str()));
        assert_eq!(child.policy_scope, p.policy_scope, "policy_scope must be inherited, never mutated");
        assert!(is_structurally_valid(child));
    }

    #[test]
    fn point_mutation_swaps_a_skill_for_a_different_alphabet_skill() {
        let p = parent();
        let cfg = MutationConfig::default();
        let mut rng = Rng::new(7);
        let child = point_mutate_skill(&p, &alphabet(), &cfg, &mut rng).expect("applicable");
        assert_probation(&child, &p);

        let before: Vec<String> = step_skills(&p);
        let after: Vec<String> = step_skills(&child);
        assert_eq!(before.len(), after.len(), "point mutation preserves step count");
        let changed: Vec<_> = before.iter().zip(&after).filter(|(a, b)| a != b).collect();
        assert_eq!(changed.len(), 1, "exactly one skill changed");
        assert!(alphabet().contains(changed[0].1), "new skill comes from alphabet");
    }

    #[test]
    fn insert_grows_and_delete_shrinks_the_phenotype() {
        let p = parent();
        let cfg = MutationConfig::default();
        let mut rng = Rng::new(11);
        let inserted = insert_step(&p, &alphabet(), &cfg, &mut rng).expect("applicable");
        assert_eq!(step_skills(&inserted).len(), step_skills(&p).len() + 1);
        assert_probation(&inserted, &p);

        let deleted = delete_step(&p, &alphabet(), &cfg, &mut rng).expect("applicable");
        assert_eq!(step_skills(&deleted).len(), step_skills(&p).len() - 1);
        assert_probation(&deleted, &p);
    }

    #[test]
    fn delete_is_noop_on_single_step_routine() {
        let mut p = parent();
        p.compiled_steps = vec![skill("git.status")];
        let cfg = MutationConfig::default();
        let mut rng = Rng::new(3);
        assert!(delete_step(&p, &alphabet(), &cfg, &mut rng).is_none());
    }

    #[test]
    fn perturb_changes_a_number_or_flips_a_bool() {
        let p = parent();
        let cfg = MutationConfig::default();
        // Sweep seeds until both override keys have been exercised.
        let mut saw_number = false;
        let mut saw_bool = false;
        for seed in 0..50u64 {
            let mut rng = Rng::new(seed);
            if let Some(child) = perturb_input(&p, &alphabet(), &cfg, &mut rng) {
                assert_probation(&child, &p);
                let ov = first_overrides(&child);
                let max_count = ov.get("max_count").unwrap();
                let recurse = ov.get("recurse").unwrap();
                if max_count != &serde_json::json!(3) {
                    saw_number = true;
                    assert!(max_count == &serde_json::json!(2) || max_count == &serde_json::json!(4));
                }
                if recurse != &serde_json::json!(true) {
                    saw_bool = true;
                    assert_eq!(recurse, &serde_json::json!(false));
                }
            }
        }
        assert!(saw_number, "number perturbation never observed");
        assert!(saw_bool, "bool flip never observed");
    }

    #[test]
    fn niche_widen_drops_a_condition_only_when_more_than_one() {
        let p = parent();
        let cfg = MutationConfig::default();
        let mut rng = Rng::new(5);
        let child = niche_widen(&p, &alphabet(), &cfg, &mut rng).expect("two conditions → applicable");
        assert_eq!(child.match_conditions.len(), p.match_conditions.len() - 1);
        assert_probation(&child, &p);

        let mut single = parent();
        single.match_conditions = vec![cond("event.detected")];
        assert!(niche_widen(&single, &alphabet(), &cfg, &mut rng).is_none());
    }

    #[test]
    fn crossover_joins_a_prefix_and_b_suffix_under_a_niche() {
        let a = parent(); // 3 steps: status, diff, log
        let mut b = parent();
        b.routine_id = "dna.scan".to_string();
        b.compiled_steps = vec![skill("search.grep"), skill("git.branch_list")];
        b.match_conditions = vec![cond("anomaly.detected")];

        let cfg = MutationConfig::default();
        let child = crossover(&a, &b, &cfg, 9).expect("both have phenotype");
        // a_cut = 3/2 = 1 → ["git.status"]; b_cut = 2/2 = 1 → ["git.branch_list"].
        assert_eq!(step_skills(&child), vec!["git.status", "git.branch_list"]);
        // Child inherits A's niche and policy_scope.
        assert_eq!(child.match_conditions, a.match_conditions);
        assert_probation(&child, &a);
    }

    #[test]
    fn mutation_is_deterministic_for_a_fixed_seed() {
        let p = parent();
        let cfg = MutationConfig::default();
        let first = mutate(&p, &alphabet(), &cfg, 42);
        let second = mutate(&p, &alphabet(), &cfg, 42);
        let ids = |v: &[Routine]| v.iter().map(|r| r.routine_id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&first), ids(&second));
        let steps = |v: &[Routine]| v.iter().map(step_skills).collect::<Vec<_>>();
        assert_eq!(steps(&first), steps(&second));
    }

    #[test]
    fn mutate_respects_offspring_cap_and_yields_distinct_ids() {
        let p = parent();
        let cfg = MutationConfig { probation_confidence: 0.5, max_offspring: 3 };
        let kids = mutate(&p, &alphabet(), &cfg, 1);
        assert!(kids.len() <= 3);
        assert!(!kids.is_empty(), "at least one operator should apply to this parent");
        let mut ids: Vec<&String> = kids.iter().map(|r| &r.routine_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), kids.len(), "all offspring ids are distinct");
        for k in &kids {
            assert_probation(k, &p);
        }
    }

    #[test]
    fn parent_of_recovers_lineage_and_ignores_non_mutants() {
        assert_eq!(parent_of("dna.orient~mp1a3f"), Some("dna.orient"));
        assert_eq!(parent_of("dna.orient~mp1a3f~mi00c2"), Some("dna.orient~mp1a3f"));
        assert_eq!(parent_of("dna.orient"), None);
    }

    #[test]
    fn invalid_routine_is_rejected() {
        let mut empty = parent();
        empty.compiled_steps = Vec::new();
        empty.compiled_skill_path = Vec::new();
        assert!(!is_structurally_valid(&empty));
    }

    #[test]
    fn reinforcement_multiplies_below_ceiling_and_clamps_at_it() {
        assert!((reinforced_confidence(0.5, 1.15, 0.95) - 0.575).abs() < 1e-9);
        // Already at/above ceiling stays at ceiling — no runaway.
        assert_eq!(reinforced_confidence(0.95, 1.15, 0.95), 0.95);
        assert_eq!(reinforced_confidence(0.9, 1.15, 0.95), 0.95);
    }

    #[test]
    fn skill_alphabet_is_deduped_sorted_and_skips_empties() {
        let a = parent(); // git.status, git.diff, git.log
        let mut b = parent();
        b.compiled_steps = vec![skill("git.diff"), skill("search.grep"), skill("")];
        let pop = vec![&a, &b];
        let alpha = skill_alphabet(&pop);
        assert_eq!(alpha, vec!["git.diff", "git.log", "git.status", "search.grep"]);
    }

    #[test]
    fn seed_is_deterministic_per_id_and_tick() {
        assert_eq!(seed_for("dna.orient", 1), seed_for("dna.orient", 1));
        assert_ne!(seed_for("dna.orient", 1), seed_for("dna.orient", 2));
        assert_ne!(seed_for("dna.orient", 1), seed_for("dna.explore", 1));
    }

    #[test]
    fn guided_mutate_replaces_the_failed_skill_with_alternatives() {
        let mut p = parent();
        p.compiled_steps = vec![skill("git.diff")]; // a routine that uses git.diff
        let cfg = MutationConfig { probation_confidence: 0.5, max_offspring: 4 };
        let kids = guided_mutate(&p, "git.diff", &alphabet(), &cfg, 1);

        assert!(!kids.is_empty(), "should produce rescue variants");
        for k in &kids {
            assert_probation(k, &p);
            let skills = step_skills(k);
            assert!(
                !skills.iter().any(|s| s == "git.diff"),
                "rescue variant must not keep the failed skill"
            );
            assert!(
                skills.iter().all(|s| alphabet().contains(s)),
                "replacement comes from the alphabet"
            );
        }
        // Distinct replacements.
        let mut reps: Vec<String> = kids.iter().filter_map(|k| step_skills(k).into_iter().next()).collect();
        reps.sort();
        reps.dedup();
        assert_eq!(reps.len(), kids.len(), "each variant uses a distinct replacement");
    }

    #[test]
    fn guided_mutate_is_noop_when_routine_does_not_use_the_failed_skill() {
        let p = parent(); // uses git.status/diff/log, not search.grep
        let cfg = MutationConfig::default();
        assert!(guided_mutate(&p, "search.grep", &alphabet(), &cfg, 1).is_empty());
    }

    #[test]
    fn should_broadcast_only_proven_mutants_once() {
        let mut m = parent(); // PackAuthored, confidence 0.9
        // A pack-authored routine is never broadcast, however fit.
        assert!(!should_broadcast(&m, 0.7, false));

        m.origin = RoutineOrigin::Mutated;
        assert!(should_broadcast(&m, 0.7, false), "proven mutant, not yet sent");
        assert!(!should_broadcast(&m, 0.7, true), "already sent — only once");

        m.confidence = 0.6;
        assert!(!should_broadcast(&m, 0.7, false), "below fitness floor");
    }

    #[test]
    fn should_breed_gates_on_threshold_fitness_population_and_autonomy() {
        let policy = BreedingPolicy::default(); // threshold 3, floor 0.7, cap 256
        let mut p = parent(); // autonomous, confidence 0.9
        assert!(should_breed(&policy, &p, 3, 10), "all conditions met");

        assert!(!should_breed(&policy, &p, 2, 10), "below success threshold");
        assert!(!should_breed(&policy, &p, 3, 256), "population at cap");

        p.confidence = 0.6;
        assert!(!should_breed(&policy, &p, 3, 10), "below fitness floor");

        p.confidence = 0.9;
        p.autonomous = false;
        assert!(!should_breed(&policy, &p, 3, 10), "non-autonomous routines do not breed");
    }

    // --- helpers ---

    fn step_skills(r: &Routine) -> Vec<String> {
        r.effective_steps()
            .iter()
            .filter_map(|s| match s {
                CompiledStep::Skill { skill_id, .. } => Some(skill_id.clone()),
                _ => None,
            })
            .collect()
    }

    fn first_overrides(r: &Routine) -> std::collections::HashMap<String, serde_json::Value> {
        for s in r.effective_steps() {
            if let CompiledStep::Skill { input_overrides, .. } = s {
                if !input_overrides.is_empty() {
                    return input_overrides;
                }
            }
        }
        std::collections::HashMap::new()
    }
}
