use crate::exact::Exact;
use crate::generative::{GenerativeProgram, ProgramExecution};
use crate::kernel::{zeros, MarkovKernel};
use crate::memory::digest_json;
use crate::relational::{RelationalLimits, RelationalProgram};
use crate::structure::IndexedStructure;
use crate::symbolic::{SymbolicMarkovProgram, SymbolicTrace, UniversalSubstrateCertificate};
use crate::transducer::{MachineNode, NodeKind, OpenTransducer};
use crate::world::RelationalFrame;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgramSubstrateCertificate {
    pub relational_input_digest: String,
    pub relational_boundary_digest: String,
    pub generative_ast_digest: String,
    pub generative_prefix_bits: usize,
    pub open_transducer: OpenTransducer,
    pub open_trace_probability_one: bool,
    pub symbolic_search_program: SymbolicMarkovProgram,
    pub symbolic_search_trace: SymbolicTrace,
    pub universal_language: UniversalSubstrateCertificate,
    pub execution_digest_matches: bool,
    pub headset_preceded_program: bool,
    pub causal_execution_replayed: bool,
    pub ast_result_matches_committed_action: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn certify_program_execution(
    observation: &RelationalFrame,
    action: &RelationalFrame,
    structure: &IndexedStructure,
    program: &GenerativeProgram,
    execution: &ProgramExecution,
    proposal_kernel: &MarkovKernel,
    selected_hypothesis: &str,
    _supporting_examples: usize,
    duration_horizon: usize,
) -> Result<ProgramSubstrateCertificate> {
    let relational_program = RelationalProgram {
        id: format!("boundary-relation:{}", structure.schema.data_relation),
        steps: Vec::new(),
        output: structure.schema.data_relation.clone(),
        provenance: observation.provenance.clone(),
    };
    let relational_execution = relational_program.execute(
        &observation.relations,
        &RelationalLimits {
            maximum_rows_per_relation: observation
                .relations
                .values()
                .map(|relation| relation.rows.len())
                .max()
                .unwrap_or(1),
            maximum_program_steps: 1,
        },
    )?;
    let input_node = MachineNode {
        id: format!("input:{}", observation.payload_digest),
        kind: NodeKind::Input {
            port: observation.payload_digest.clone(),
        },
    };
    let hidden_node = MachineNode {
        id: format!("program:{}", digest_json(program)?),
        kind: NodeKind::Hidden,
    };
    let output_node = MachineNode {
        id: format!("output:{}", action.payload_digest),
        kind: NodeKind::Output {
            port: action.payload_digest.clone(),
        },
    };
    let nodes = vec![input_node, hidden_node, output_node];
    let mut transition = zeros(3, 3);
    transition[0][1] = Exact::one();
    transition[1][2] = Exact::one();
    transition[2][2] = Exact::one();
    let open_transducer = OpenTransducer::new(
        format!("execution-transducer:{}", execution.execution_digest),
        nodes.clone(),
        MarkovKernel::new(
            nodes.iter().map(|node| node.id.clone()).collect(),
            transition,
        )?,
        vec![
            observation.payload_digest.clone(),
            execution.execution_digest.clone(),
            action.payload_digest.clone(),
        ],
    )?;
    let semantics = open_transducer.semantics(duration_horizon)?;
    let open_trace_probability_one = semantics.probability.len() == 1
        && semantics.probability[0].len() == 1
        && semantics.probability[0][0].is_one();
    let expanded_bits = program.prefix_description_bits()?;
    let symbolic_search_program = SymbolicMarkovProgram::lift_one_step(proposal_kernel)?;
    let symbolic_search_trace = symbolic_search_program.finite_trace(
        selected_hypothesis,
        2,
        proposal_kernel.states.len().saturating_mul(2).max(1),
    )?;
    let universal_language = symbolic_search_program.universal_substrate_certificate();
    let committed_structure = IndexedStructure::infer(action)?;
    let replayed = program.execute(observation)?;
    Ok(ProgramSubstrateCertificate {
        relational_input_digest: digest_json(&observation.relations)?,
        relational_boundary_digest: relational_execution.database_digest,
        generative_ast_digest: digest_json(program)?,
        generative_prefix_bits: expanded_bits,
        open_transducer,
        open_trace_probability_one,
        symbolic_search_program,
        symbolic_search_trace,
        universal_language,
        execution_digest_matches: execution.observation_digest == observation.payload_digest
            && execution.program_id == program.id,
        headset_preceded_program: !execution.headset_digest.is_empty()
            && !execution.perception_digest.is_empty()
            && !execution.program_execution_digest.is_empty()
            && !execution.action_execution_digest.is_empty(),
        causal_execution_replayed: replayed.execution_digest == execution.execution_digest
            && replayed.result_relations == execution.result_relations,
        ast_result_matches_committed_action: execution.result.domains
            == committed_structure.domains
            && execution.result.values == committed_structure.values,
    })
}
