"""Training loop with combined loss, evaluation, and early stopping.

Implements the SOMA Synthesizer training procedure per Spec Section 5:
    - Combined cross-entropy loss (opcode + arg types + masked span/ref/literal)
    - ReduceLROnPlateau scheduler
    - Early stopping with patience
    - Evaluation metrics per Section 5.3: op_accuracy, program_exact_match,
      span_accuracy, ref_accuracy, literal_accuracy, end_to_end, novel_intent

Self-contained: imports only from soma_synthesizer package + stdlib + torch.
"""

import time
from dataclasses import dataclass, field

import torch
import torch.nn as nn
import torch.nn.functional as F
from torch.utils.data import DataLoader

from soma_synthesizer.data import (
    ARG_NONE,
    ARG_SPAN,
    ARG_REF,
    ARG_LITERAL,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _masked_ce(
    logits: torch.Tensor,
    targets: torch.Tensor,
    ignore_index: int = -1,
) -> torch.Tensor:
    """Cross-entropy computed only where ``targets != ignore_index``.

    This is used for conditional losses: span position loss only on steps
    where arg type is span, ref loss only where type is ref, etc.

    Returns a scalar tensor (0.0 if no valid targets exist).
    """
    mask = targets != ignore_index
    if not mask.any():
        return torch.tensor(0.0, device=logits.device, requires_grad=False)
    return F.cross_entropy(logits[mask], targets[mask])


# ---------------------------------------------------------------------------
# Training statistics
# ---------------------------------------------------------------------------

@dataclass
class TrainingStats:
    """Accumulated statistics from a training run."""
    total_examples: int = 0
    best_epoch: int = 0
    best_val_loss: float = float("inf")
    test_op_accuracy: float = 0.0
    test_program_exact: float = 0.0
    test_span_accuracy: float = 0.0
    test_ref_accuracy: float = 0.0
    test_literal_accuracy: float = 0.0
    test_e2e: float = 0.0
    test_novel_intent: float = 0.0
    elapsed: float = 0.0
    epoch_log: list = field(default_factory=list)


# ---------------------------------------------------------------------------
# Trainer
# ---------------------------------------------------------------------------

class SomaTrainer:
    """End-to-end trainer for the SOMA Mind model.

    Handles the combined loss, epoch loops, validation, early stopping,
    and post-training assessment per Spec Sections 5.1-5.3.

    Args:
        model: a ``SomaMind`` (or compatible) model instance.
        config: dict with optional keys:
            lr (float, default 1e-3),
            weight_decay (float, default 1e-2),
            epochs (int, default 200),
            patience (int, default 30),
            grad_clip (float, default 1.0),
            device (str, default "cpu").
    """

    def __init__(self, model: nn.Module, config: dict | None = None):
        config = config or {}
        self.model = model
        self.device = torch.device(config.get("device", "cpu"))
        self.model.to(self.device)

        self.optimizer = torch.optim.AdamW(
            model.parameters(),
            lr=config.get("lr", 1e-3),
            weight_decay=config.get("weight_decay", 1e-2),
        )
        self.scheduler = torch.optim.lr_scheduler.ReduceLROnPlateau(
            self.optimizer,
            patience=config.get("scheduler_patience", 10),
            factor=config.get("scheduler_factor", 0.5),
        )
        self.max_epochs = config.get("epochs", 200)
        self.patience = config.get("patience", 30)
        self.grad_clip = config.get("grad_clip", 1.0)
        self.log_every = config.get("log_every", 10)

    # -- loss ----------------------------------------------------------------

    def compute_loss(
        self,
        model_output: dict[str, torch.Tensor],
        batch: dict[str, torch.Tensor],
    ) -> torch.Tensor:
        """Combined cross-entropy: opcode + arg_types + masked spans + refs + literals.

        Per Spec Section 5.1, the loss sums over all program steps:
            CE(op) + CE(a0_type) + CE(a1_type)
            + masked_CE(span_s0) + masked_CE(span_e0)
            + masked_CE(span_s1) + masked_CE(span_e1)
            + masked_CE(ref0) + masked_CE(ref1)
            + masked_CE(lit0) + masked_CE(lit1)
        """
        B = batch["opcode"].size(0)
        S = batch["opcode"].size(1)
        dev = self.device

        tgt_op = batch["opcode"].to(dev)
        tgt_a0t = batch["a0_type"].to(dev)
        tgt_a1t = batch["a1_type"].to(dev)

        # Flatten (B, S, C) -> (B*S, C) for CE
        loss = F.cross_entropy(
            model_output["op"].reshape(B * S, -1), tgt_op.reshape(B * S)
        )
        loss = loss + F.cross_entropy(
            model_output["a0t"].reshape(B * S, -1), tgt_a0t.reshape(B * S)
        )
        loss = loss + F.cross_entropy(
            model_output["a1t"].reshape(B * S, -1), tgt_a1t.reshape(B * S)
        )

        # Span losses (masked: only where target arg type == span)
        for logit_s, logit_e, tgt_s_key, tgt_e_key in [
            ("s0s", "s0e", "a0_span_s", "a0_span_e"),
            ("s1s", "s1e", "a1_span_s", "a1_span_e"),
        ]:
            loss = loss + _masked_ce(
                model_output[logit_s].reshape(B * S, -1),
                batch[tgt_s_key].to(dev).reshape(B * S),
            )
            loss = loss + _masked_ce(
                model_output[logit_e].reshape(B * S, -1),
                batch[tgt_e_key].to(dev).reshape(B * S),
            )

        # Ref losses (masked: only where target arg type == ref)
        for logit_key, tgt_key in [("r0", "a0_ref"), ("r1", "a1_ref")]:
            loss = loss + _masked_ce(
                model_output[logit_key].reshape(B * S, -1),
                batch[tgt_key].to(dev).reshape(B * S),
            )

        # Literal losses (masked: only where target arg type == literal)
        for logit_key, tgt_key in [("l0", "a0_lit"), ("l1", "a1_lit")]:
            if logit_key in model_output:
                loss = loss + _masked_ce(
                    model_output[logit_key].reshape(B * S, -1),
                    batch[tgt_key].to(dev).reshape(B * S),
                )

        return loss

    # -- single epoch --------------------------------------------------------

    def train_epoch(self, dataloader: DataLoader) -> float:
        """Run one training epoch. Returns average loss."""
        self.model.train()
        total_loss = 0.0
        total_samples = 0

        for batch in dataloader:
            ids = batch["input_ids"].to(self.device)
            lens = batch["lengths"].to(self.device)
            tgt_ops = batch["opcode"].to(self.device)

            output = self.model(ids, lens, tgt_ops)
            loss = self.compute_loss(output, batch)

            self.optimizer.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(self.model.parameters(), self.grad_clip)
            self.optimizer.step()

            B = ids.size(0)
            total_loss += loss.item() * B
            total_samples += B

        return total_loss / max(total_samples, 1)

    # -- metrics computation -------------------------------------------------

    def evaluate(self, dataloader: DataLoader) -> dict[str, float]:
        """Compute metrics on a dataloader (val or test).

        Returns dict with keys per Spec Section 5.3:
            loss, op_accuracy, program_exact_match, span_accuracy,
            ref_accuracy, literal_accuracy, end_to_end
        """
        self.model.eval()
        total_loss = 0.0
        total_samples = 0

        # Counters
        op_correct_steps = 0
        op_total_steps = 0
        program_exact = 0
        span_correct = 0
        span_total = 0
        ref_correct = 0
        ref_total = 0
        lit_correct = 0
        lit_total = 0
        e2e_correct = 0  # fully correct: op + all arg types + all values

        with torch.no_grad():
            for batch in dataloader:
                ids = batch["input_ids"].to(self.device)
                lens = batch["lengths"].to(self.device)
                tgt_ops = batch["opcode"].to(self.device)
                B, S = tgt_ops.shape

                output = self.model(ids, lens, tgt_ops)
                loss = self.compute_loss(output, batch)
                total_loss += loss.item() * B
                total_samples += B

                # --- Per-step opcode accuracy ---
                pred_op = output["op"].argmax(-1)  # (B, S)
                op_match = pred_op == tgt_ops  # (B, S)
                op_correct_steps += op_match.sum().item()
                op_total_steps += B * S

                # --- Program exact match (all opcodes correct) ---
                prog_match = op_match.all(dim=1)  # (B,)
                program_exact += prog_match.sum().item()

                # --- Arg type predictions ---
                pred_a0t = output["a0t"].argmax(-1)  # (B, S)
                pred_a1t = output["a1t"].argmax(-1)  # (B, S)
                tgt_a0t = batch["a0_type"].to(self.device)
                tgt_a1t = batch["a1_type"].to(self.device)
                a0t_match = pred_a0t == tgt_a0t
                a1t_match = pred_a1t == tgt_a1t

                # --- Span accuracy (only where target type == span) ---
                for logit_s, logit_e, tgt_s_key, tgt_e_key, tgt_type_key in [
                    ("s0s", "s0e", "a0_span_s", "a0_span_e", "a0_type"),
                    ("s1s", "s1e", "a1_span_s", "a1_span_e", "a1_type"),
                ]:
                    tgt_type = batch[tgt_type_key].to(self.device)
                    span_mask = tgt_type == ARG_SPAN  # (B, S)
                    if span_mask.any():
                        tgt_s = batch[tgt_s_key].to(self.device)
                        tgt_e = batch[tgt_e_key].to(self.device)
                        pred_s = output[logit_s].argmax(-1)
                        pred_e = output[logit_e].argmax(-1)
                        both_correct = (pred_s == tgt_s) & (pred_e == tgt_e) & span_mask
                        span_correct += both_correct.sum().item()
                        span_total += span_mask.sum().item()

                # --- Ref accuracy (only where target type == ref) ---
                for logit_key, tgt_key, tgt_type_key in [
                    ("r0", "a0_ref", "a0_type"),
                    ("r1", "a1_ref", "a1_type"),
                ]:
                    tgt_type = batch[tgt_type_key].to(self.device)
                    ref_mask = tgt_type == ARG_REF  # (B, S)
                    if ref_mask.any():
                        tgt_r = batch[tgt_key].to(self.device)
                        pred_r = output[logit_key].argmax(-1)
                        ref_match = (pred_r == tgt_r) & ref_mask
                        ref_correct += ref_match.sum().item()
                        ref_total += ref_mask.sum().item()

                # --- Literal accuracy (only where target type == literal) ---
                for logit_key, tgt_key, tgt_type_key in [
                    ("l0", "a0_lit", "a0_type"),
                    ("l1", "a1_lit", "a1_type"),
                ]:
                    if logit_key not in output:
                        continue
                    tgt_type = batch[tgt_type_key].to(self.device)
                    lit_mask = tgt_type == ARG_LITERAL  # (B, S)
                    if lit_mask.any():
                        tgt_l = batch[tgt_key].to(self.device)
                        pred_l = output[logit_key].argmax(-1)
                        lit_match = (pred_l == tgt_l) & lit_mask
                        lit_correct += lit_match.sum().item()
                        lit_total += lit_mask.sum().item()

                # --- End-to-end: per-sample, all steps fully correct ---
                # A sample is e2e correct iff: every step has correct
                # opcode AND correct arg types AND correct values for
                # active arg types.
                sample_ok = op_match & a0t_match & a1t_match  # (B, S)

                # Check span values where target type is span
                for logit_s, logit_e, tgt_s_key, tgt_e_key, tgt_type_key in [
                    ("s0s", "s0e", "a0_span_s", "a0_span_e", "a0_type"),
                    ("s1s", "s1e", "a1_span_s", "a1_span_e", "a1_type"),
                ]:
                    tgt_type = batch[tgt_type_key].to(self.device)
                    span_mask = tgt_type == ARG_SPAN
                    if span_mask.any():
                        tgt_s = batch[tgt_s_key].to(self.device)
                        tgt_e = batch[tgt_e_key].to(self.device)
                        pred_s = output[logit_s].argmax(-1)
                        pred_e = output[logit_e].argmax(-1)
                        span_ok = (~span_mask) | ((pred_s == tgt_s) & (pred_e == tgt_e))
                        sample_ok = sample_ok & span_ok

                # Check ref values where target type is ref
                for logit_key, tgt_key, tgt_type_key in [
                    ("r0", "a0_ref", "a0_type"),
                    ("r1", "a1_ref", "a1_type"),
                ]:
                    tgt_type = batch[tgt_type_key].to(self.device)
                    ref_mask = tgt_type == ARG_REF
                    if ref_mask.any():
                        tgt_r = batch[tgt_key].to(self.device)
                        pred_r = output[logit_key].argmax(-1)
                        ref_ok = (~ref_mask) | (pred_r == tgt_r)
                        sample_ok = sample_ok & ref_ok

                # Check literal values where target type is literal
                for logit_key, tgt_key, tgt_type_key in [
                    ("l0", "a0_lit", "a0_type"),
                    ("l1", "a1_lit", "a1_type"),
                ]:
                    if logit_key not in output:
                        continue
                    tgt_type = batch[tgt_type_key].to(self.device)
                    lit_mask = tgt_type == ARG_LITERAL
                    if lit_mask.any():
                        tgt_l = batch[tgt_key].to(self.device)
                        pred_l = output[logit_key].argmax(-1)
                        lit_ok = (~lit_mask) | (pred_l == tgt_l)
                        sample_ok = sample_ok & lit_ok

                # All steps must be correct for a sample to be e2e correct
                e2e_correct += sample_ok.all(dim=1).sum().item()

        metrics = {
            "loss": total_loss / max(total_samples, 1),
            "op_accuracy": op_correct_steps / max(op_total_steps, 1),
            "program_exact_match": program_exact / max(total_samples, 1),
            "span_accuracy": span_correct / max(span_total, 1),
            "ref_accuracy": ref_correct / max(ref_total, 1),
            "literal_accuracy": lit_correct / max(lit_total, 1),
            "end_to_end": e2e_correct / max(total_samples, 1),
        }
        return metrics

    # -- full training loop --------------------------------------------------

    def train(
        self,
        train_loader: DataLoader,
        val_loader: DataLoader,
        test_loader: DataLoader | None = None,
    ) -> TrainingStats:
        """Full training loop with early stopping per Spec Section 5.2.

        Prints progress in the format shown in Spec Section 5.4.
        Returns ``TrainingStats`` with final metrics.
        """
        stats = TrainingStats()
        best_state: dict | None = None
        no_improve = 0
        t0 = time.time()

        print(f"\n[Synthesis] Training up to {self.max_epochs} epochs "
              f"(patience={self.patience})...")

        for epoch in range(1, self.max_epochs + 1):
            train_loss = self.train_epoch(train_loader)
            val_metrics = self.evaluate(val_loader)
            val_loss = val_metrics["loss"]
            self.scheduler.step(val_loss)

            entry = {
                "epoch": epoch,
                "train_loss": train_loss,
                "val_loss": val_loss,
                "prog": val_metrics["program_exact_match"],
                "e2e": val_metrics["end_to_end"],
            }
            stats.epoch_log.append(entry)

            if epoch == 1 or epoch % self.log_every == 0:
                print(
                    f"  Epoch {epoch:3d} | "
                    f"Train loss={train_loss:.4f} | "
                    f"Val loss={val_loss:.4f} "
                    f"prog={val_metrics['program_exact_match']:.3f} "
                    f"e2e={val_metrics['end_to_end']:.3f}"
                )

            if val_loss < stats.best_val_loss:
                stats.best_val_loss = val_loss
                stats.best_epoch = epoch
                best_state = {
                    k: v.cpu().clone() for k, v in self.model.state_dict().items()
                }
                no_improve = 0
            else:
                no_improve += 1
                if no_improve >= self.patience:
                    print(f"  Early stopping at epoch {epoch} (best={stats.best_epoch})")
                    break

        stats.elapsed = time.time() - t0
        print(f"\n[Synthesis] Done in {stats.elapsed:.1f}s (best epoch: {stats.best_epoch})")

        # Restore best weights
        if best_state is not None:
            self.model.load_state_dict(best_state)

        # --- Test set assessment ---
        if test_loader is not None:
            test_metrics = self.evaluate(test_loader)
            stats.test_op_accuracy = test_metrics["op_accuracy"]
            stats.test_program_exact = test_metrics["program_exact_match"]
            stats.test_span_accuracy = test_metrics["span_accuracy"]
            stats.test_ref_accuracy = test_metrics["ref_accuracy"]
            stats.test_literal_accuracy = test_metrics["literal_accuracy"]
            stats.test_e2e = test_metrics["end_to_end"]

            print(f"\n[Test] Results:")
            print(f"  Op Accuracy:       {stats.test_op_accuracy:.3f}")
            print(f"  Program Exact:     {stats.test_program_exact:.3f}")
            print(f"  Span Accuracy:     {stats.test_span_accuracy:.3f}")
            print(f"  Ref Accuracy:      {stats.test_ref_accuracy:.3f}")
            print(f"  Literal Accuracy:  {stats.test_literal_accuracy:.3f}")
            print(f"  End-to-End:        {stats.test_e2e:.3f}")

        return stats

    # -- novel intent assessment ---------------------------------------------

    def evaluate_novel_intents(
        self,
        novel_loader: DataLoader,
    ) -> float:
        """Measure accuracy on held-out intent phrasings.

        Per Spec Section 5.3, this measures generalization to novel
        wordings of the same operations seen during training.

        Returns the end-to-end accuracy on the novel set.
        """
        metrics = self.evaluate(novel_loader)
        novel_e2e = metrics["end_to_end"]
        print(f"  Novel Intents:     {novel_e2e:.3f}")
        return novel_e2e
