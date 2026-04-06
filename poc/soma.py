"""
SOMA Instance — v0.3: Live Trace + Confidence + Error Recovery.

The Mind generates multi-step programs with cross-operation data flow.
The Body executes them step by step with live trace output.

Usage:
    python -m poc.soma
"""

import platform

import torch

from poc.body import ThinBody, Prim, OPCODE_NAMES
from poc.mind import SomaMind
from poc.tokenizer import Tokenizer, NULL_IDX


CONFIDENCE_THRESHOLD = 0.3


class Soma:

    def __init__(self, model_path: str, vocab_path: str):
        self.body = ThinBody()

        self.tokenizer = Tokenizer()
        self.tokenizer.load(vocab_path)

        self.mind = SomaMind(vocab_size=self.tokenizer.vocab_size)
        state = torch.load(model_path, map_location="cpu", weights_only=True)
        self.mind.load_state_dict(state)
        self.mind.eval()

        self.execution_count = 0
        self.error_count = 0

    def process_intent(self, intent_text: str) -> dict:
        intent_text = intent_text.strip()
        if not intent_text:
            return {"type": "empty"}

        lower = intent_text.lower()
        if any(kw in lower for kw in [
            "what can you do", "your capabilities", "what operations",
            "about yourself", "what are you",
        ]) or lower in ("help", "?"):
            return self._proprioception()

        tokens = self.tokenizer.tokenize(intent_text)
        token_ids = [NULL_IDX] + self.tokenizer.encode(intent_text)

        input_tensor = torch.tensor([token_ids], dtype=torch.long)
        length_tensor = torch.tensor([len(token_ids)], dtype=torch.long)

        steps, confidence = self.mind.predict(input_tensor, length_tensor, tokens)

        if confidence < CONFIDENCE_THRESHOLD:
            return {
                "type": "ambiguous",
                "confidence": confidence,
            }

        return {
            "type": "execution",
            "steps": steps,
            "confidence": confidence,
        }

    def execute_and_display(self, result: dict):
        """Execute program and display with live trace."""
        if result["type"] == "empty":
            return

        elif result["type"] == "proprioception":
            p = result
            print(f"\n  [Proprioception]")
            print(f"  Mind: {p['mind']['parameters']:,} params, {p['mind']['architecture']}")
            print(f"  Stats: {p['stats']['executions']} executions, {p['stats']['errors']} errors")
            print(f"\n  Primitives ({len(p['primitives'])}):")
            for prim in p["primitives"]:
                a0, a1 = prim["args"]
                args = ", ".join(a for a in [a0, a1] if a != "none")
                print(f"    [{prim['opcode']:2d}] {prim['name']}{'(' + args + ')' if args else '()'}")

        elif result["type"] == "ambiguous":
            print(f"\n  [Mind] Confidence too low ({result['confidence']:.0%})")
            print(f"         I'm not sure what you want. Could you rephrase?")

        elif result["type"] == "execution":
            steps = result["steps"]
            confidence = result["confidence"]
            real_steps = [s for s in steps if s.opcode != Prim.STOP]

            print(f"\n  [Mind] Program ({len(real_steps)} steps, {confidence:.0%} confidence):")
            for i, step in enumerate(steps):
                print(f"    {step.format(i)}")
                if step.opcode == Prim.STOP:
                    break

            # Execute with live trace
            print()
            def on_step(idx, op_name, summary):
                if op_name == "STOP":
                    return
                if op_name == "EMIT":
                    return  # we'll display output separately
                status = f"... {summary}" if summary else ""
                print(f"    [{idx}] {op_name} {status}")

            exec_result = self.body.execute_program(steps, on_step=on_step)

            self.execution_count += 1
            if not exec_result["success"]:
                self.error_count += 1
                print(f"\n  [Body] Error: {exec_result['error']}")
                # Show which steps succeeded
                ok_steps = [t for t in exec_result["trace"] if t.get("ok")]
                if ok_steps:
                    print(f"         ({len(ok_steps)} steps completed before failure)")
            else:
                out = exec_result["output"]
                if out is None:
                    print(f"  [Body] Done.")
                elif isinstance(out, list):
                    print(f"  [Body] Output ({len(out)} items):")
                    for item in out[:20]:
                        print(f"    {item}")
                    if len(out) > 20:
                        print(f"    ... and {len(out) - 20} more")
                elif isinstance(out, dict):
                    print(f"  [Body] Output:")
                    for k, v in out.items():
                        print(f"    {k}: {v}")
                elif isinstance(out, bool):
                    print(f"  [Body] {out}")
                else:
                    print(f"  [Body] {out}")

    def _proprioception(self) -> dict:
        return {
            "type": "proprioception",
            "primitives": self.body.capabilities(),
            "mind": {
                "parameters": sum(p.numel() for p in self.mind.parameters()),
                "vocabulary": self.tokenizer.vocab_size,
                "architecture": "BiLSTM encoder + GRU decoder (dynamic arg types)",
            },
            "stats": {
                "executions": self.execution_count,
                "errors": self.error_count,
            },
        }

    def repl(self):
        total_params = sum(p.numel() for p in self.mind.parameters())
        print("=" * 60)
        print("  SOMA Proof of Concept v0.3")
        print("  The Neural Network IS the Program")
        print("=" * 60)
        print(f"  Body:       macOS {platform.machine()} (19 primitives)")
        print(f"  Mind:       {total_params:,} params (BiLSTM + GRU)")
        print(f"  Features:   compositional programs, cross-op data flow")
        print(f"  Vocabulary: {self.tokenizer.vocab_size} tokens")
        print("=" * 60)
        print("  Type intent. The mind generates a program. The body executes it.")
        print("  Type 'quit' to exit, 'help' for capabilities.")
        print()

        while True:
            try:
                intent = input("intent> ").strip()
                if not intent:
                    continue
                if intent.lower() in ("quit", "exit", "q"):
                    print("\nSOMA shutting down.")
                    break

                result = self.process_intent(intent)
                self.execute_and_display(result)
                print()

            except KeyboardInterrupt:
                print("\n\nSOMA shutting down.")
                break
            except Exception as e:
                print(f"  [Error] {e}\n")


def main():
    soma = Soma("poc/artifacts/soma_mind.pt", "poc/artifacts/vocab.json")
    soma.repl()


if __name__ == "__main__":
    main()
