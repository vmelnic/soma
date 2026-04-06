"""
SOMA Instance — Phase 2: Program Execution.

The Mind generates multi-step programs. The Body executes them.
The program trace is visible — proving the model IS the program.

Usage:
    python -m poc.soma
"""

import platform

import torch

from poc.body import ThinBody, Prim, OPCODE_NAMES
from poc.mind import SomaMind
from poc.tokenizer import Tokenizer, NULL_IDX


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

        # Layer 1: Intent reception
        tokens = self.tokenizer.tokenize(intent_text)
        token_ids = [NULL_IDX] + self.tokenizer.encode(intent_text)

        input_tensor = torch.tensor([token_ids], dtype=torch.long)
        length_tensor = torch.tensor([len(token_ids)], dtype=torch.long)

        # Layers 2+3: Mind generates program
        steps = self.mind.predict(input_tensor, length_tensor, tokens)

        # Body executes program
        result = self.body.execute_program(steps)

        self.execution_count += 1
        if not result["success"]:
            self.error_count += 1

        return {
            "type": "execution",
            "steps": steps,
            "result": result,
        }

    def _proprioception(self) -> dict:
        return {
            "type": "proprioception",
            "primitives": self.body.capabilities(),
            "mind": {
                "parameters": sum(p.numel() for p in self.mind.parameters()),
                "vocabulary": self.tokenizer.vocab_size,
                "architecture": "BiLSTM encoder + GRU decoder",
            },
            "stats": {
                "executions": self.execution_count,
                "errors": self.error_count,
            },
        }

    def repl(self):
        total_params = sum(p.numel() for p in self.mind.parameters())
        print("=" * 60)
        print("  SOMA Proof of Concept v0.2")
        print("  The Neural Network IS the Program")
        print("=" * 60)
        print(f"  Body:       macOS {platform.machine()} (19 primitives)")
        print(f"  Mind:       {total_params:,} params (BiLSTM + GRU)")
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

                if result["type"] == "empty":
                    continue

                elif result["type"] == "proprioception":
                    p = result
                    print(f"\n  [Proprioception]")
                    print(f"  Mind: {p['mind']['parameters']:,} params, "
                          f"{p['mind']['architecture']}")
                    print(f"  Stats: {p['stats']['executions']} executions, "
                          f"{p['stats']['errors']} errors")
                    print(f"\n  Primitives ({len(p['primitives'])}):")
                    for prim in p["primitives"]:
                        a0, a1 = prim["args"]
                        args = ", ".join(a for a in [a0, a1] if a != "none")
                        print(f"    [{prim['opcode']:2d}] {prim['name']}"
                              f"{'(' + args + ')' if args else '()'}")

                elif result["type"] == "execution":
                    steps = result["steps"]
                    exec_result = result["result"]

                    # Show the program the mind generated
                    real_steps = [s for s in steps if s.opcode != Prim.STOP]
                    print(f"\n  [Mind] Program ({len(real_steps)} steps):")
                    for i, step in enumerate(steps):
                        print(f"    {step.format(i)}")
                        if step.opcode == Prim.STOP:
                            break

                    # Show execution result
                    if exec_result["success"]:
                        out = exec_result["output"]
                        if isinstance(out, list):
                            print(f"\n  [Body] Output:")
                            for item in out[:20]:
                                print(f"    {item}")
                            if len(out) > 20:
                                print(f"    ... ({len(out)} total)")
                        elif isinstance(out, dict):
                            print(f"\n  [Body] Output:")
                            for k, v in out.items():
                                print(f"    {k}: {v}")
                        elif isinstance(out, bool):
                            print(f"\n  [Body] {out}")
                        elif out is not None:
                            print(f"\n  [Body] {out}")
                        else:
                            print(f"\n  [Body] Done.")
                    else:
                        print(f"\n  [Body] Error: {exec_result['error']}")

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
