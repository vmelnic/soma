use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use soma_rtl::generative::GenerativeLimits;
use soma_rtl::instrument::{observe, InstrumentOutput};
use soma_rtl::interaction::{run_world_interaction, InteractionLimits};
use soma_rtl::runtime::run_suite;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "soma-rtl")]
#[command(about = "Reflective Markov observation and induction runtime")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Verify and decode one external instrument without running cognition.
    Observe { manifest: PathBuf },
    /// Learn a causal Markov world model from measured traces and plan in it.
    Plan {
        manifest: PathBuf,
        #[arg(long)]
        state: Option<String>,
        #[arg(long, default_value_t = 8)]
        horizon: usize,
    },
    /// Run the complete, predeclared integrated validation once.
    Validate { suite: PathBuf },
    /// Learn and act through a domain-neutral operational world protocol.
    Interact {
        manifest: PathBuf,
        #[arg(long)]
        report: PathBuf,
        #[arg(long, default_value_t = 64)]
        maximum_candidates: usize,
        #[arg(long, default_value_t = 2048)]
        maximum_period_models: usize,
        #[arg(long, default_value_t = 16)]
        maximum_recursion_depth: usize,
        #[arg(long, default_value_t = 16)]
        trace_duration_horizon: usize,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Observe { manifest } => match observe(&manifest)? {
            InstrumentOutput::Sequences(batch) => {
                println!("{}", serde_json::to_string_pretty(&batch.audit)?);
            }
            InstrumentOutput::Matrix(batch) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "source": &batch.source.id,
                        "events": batch.events.len(),
                        "digest": batch.document_digest,
                    }))?
                );
            }
            InstrumentOutput::Traces(batch) => {
                let model = soma_rtl::active::ActiveWorldModel::learn(&batch.events)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "source": batch.source.id,
                        "events": batch.events.len(),
                        "states": model.states.len(),
                        "actions": model.actions.len(),
                        "model_digest": model.model_digest,
                    }))?
                );
            }
        },
        Command::Validate { suite } => {
            let report = run_suite(&suite)?;
            let report_digest = soma_rtl::memory::digest_json(&report)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "suite": &report.suite_id,
                    "complete": report.complete,
                    "ledger": &report.ledger,
                    "report_digest": report_digest,
                }))?
            );
            if !report.complete {
                bail!("integrated validation completed with unresolved required checks");
            }
        }
        Command::Plan {
            manifest,
            state,
            horizon,
        } => {
            let InstrumentOutput::Traces(batch) = observe(&manifest)? else {
                bail!("planning requires a trace_json instrument");
            };
            let model = soma_rtl::active::ActiveWorldModel::learn(&batch.events)?;
            let held_out = soma_rtl::active::evaluate_withheld_transitions(&batch.events)?;
            let initial = state
                .or_else(|| batch.events.first().map(|event| event.state.clone()))
                .ok_or_else(|| anyhow::anyhow!("trace instrument has no initial state"))?;
            let plan = model.plan(&initial, horizon)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "model_digest": model.model_digest,
                    "goal_posterior": model.goal_posterior,
                    "held_out_transition_evaluation": held_out,
                    "plan": plan,
                }))?
            );
        }
        Command::Interact {
            manifest,
            report,
            maximum_candidates,
            maximum_period_models,
            maximum_recursion_depth,
            trace_duration_horizon,
        } => {
            let result = run_world_interaction(
                &manifest,
                &report,
                InteractionLimits {
                    generative: GenerativeLimits {
                        maximum_candidates,
                        maximum_period_models,
                        maximum_recursion_depth,
                    },
                    trace_duration_horizon,
                },
            )?;
            let selected = result
                .induction
                .hypotheses
                .iter()
                .find(|candidate| candidate.id == result.induction.selected_hypothesis)
                .ok_or_else(|| {
                    anyhow::anyhow!("selected hypothesis is absent from the posterior")
                })?;
            let report_digest = soma_rtl::memory::digest_json(&result)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "integration_complete": result.integration_complete,
                    "external_task_solved": result.external_task_solved,
                    "public_view_digest": &result.public_view_digest,
                    "selected_hypothesis": &result.induction.selected_hypothesis,
                    "selected_program": &selected.program,
                    "selected_replay": &selected.replay,
                    "selected_leave_one_out": &selected.leave_one_out,
                    "selected_transferred_from_memory": selected.transferred_from_memory,
                    "selected_posterior_probability": selected.posterior.to_f64()?,
                    "posterior_normalized": result.induction.posterior_normalized,
                    "detailed_balance": result.induction.detailed_balance,
                    "candidate_count": result.induction.hypotheses.len(),
                    "query_diagnostics": result.query_actions.iter().map(|item| serde_json::json!({
                        "episode": item.episode_id,
                        "derived_cells": item.execution.derived_cells,
                        "unresolved_cells": item.execution.unresolved_cells,
                        "ambiguous_cells": item.execution.ambiguous_cells,
                    })).collect::<Vec<_>>(),
                    "evaluation_all_exact": result.evaluation.all_exact,
                    "actions_committed_before_reveal": result.evaluation.actions_committed_before_reveal,
                    "evaluation_cases": &result.evaluation.cases,
                    "report_digest": report_digest,
                }))?
            );
            if !result.integration_complete {
                bail!("world interaction completed with an incomplete architecture checklist");
            }
        }
    }
    Ok(())
}
