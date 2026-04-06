"""
SOMA Instance — The running organism.

This ties the Mind (neural architecture) and Body (OS interface) together
into a living SOMA that receives human intent and directly executes it.

Usage:
    python -m poc.soma
"""

import platform
import sys

import torch

from poc.body import Body
from poc.mind import SomaMind
from poc.tokenizer import Tokenizer, NULL_IDX


CONFIDENCE_THRESHOLD = 0.4


class Soma:
    """A running SOMA instance — the organism."""

    def __init__(self, model_path: str, vocab_path: str):
        self.body = Body()

        self.tokenizer = Tokenizer()
        self.tokenizer.load(vocab_path)

        self.mind = SomaMind(vocab_size=self.tokenizer.vocab_size)
        state = torch.load(model_path, map_location="cpu", weights_only=True)
        self.mind.load_state_dict(state)
        self.mind.eval()

        self.execution_count = 0
        self.error_count = 0
        self.success_count = 0
        self.last_op = None

    def process_intent(self, intent_text: str) -> dict:
        """The core SOMA loop: intent -> mind -> body -> result."""
        intent_text = intent_text.strip()
        if not intent_text:
            return {"type": "empty"}

        lower = intent_text.lower()
        if any(kw in lower for kw in [
            "what can you do", "your capabilities", "what operations",
            "what do you know", "about yourself", "what are you",
        ]) or lower in ("help", "?"):
            return self._proprioception_report()

        tokens = self.tokenizer.tokenize(intent_text)
        token_ids = [NULL_IDX] + self.tokenizer.encode(intent_text)
        length = len(token_ids)

        input_tensor = torch.tensor([token_ids], dtype=torch.long)
        length_tensor = torch.tensor([length], dtype=torch.long)

        predicted_op, spans, confidence = self.mind.predict(input_tensor, length_tensor)

        opcode = predicted_op.item()
        conf = confidence.item()

        if conf < CONFIDENCE_THRESHOLD:
            op_logits, _ = self.mind(input_tensor, length_tensor)
            probs = torch.softmax(op_logits, dim=-1)[0]
            top3 = probs.topk(3)
            guesses = [
                (self.body.operations[i.item()].name, p.item())
                for i, p in zip(top3.indices, top3.values)
            ]
            return {
                "type": "ambiguous",
                "message": "I'm not confident about what you want. Could you rephrase?",
                "top_guesses": guesses,
            }

        op_schema = self.body.operations[opcode]

        params = []
        for slot_idx, (start, end) in enumerate(spans):
            s, e = start.item(), end.item()
            if s == 0 and e == 0:
                params.append(None)
            else:
                param_tokens = tokens[s - 1 : e]
                params.append(" ".join(param_tokens))

        result = self.body.dispatch(opcode, params)

        self.execution_count += 1
        self.last_op = op_schema.name
        if result["success"]:
            self.success_count += 1
        else:
            self.error_count += 1

        param_dict = {}
        for i, p in enumerate(params):
            if p is not None and i < len(op_schema.params):
                param_dict[op_schema.params[i].name] = p

        return {
            "type": "execution",
            "operation": op_schema.name,
            "confidence": conf,
            "params": param_dict,
            "result": result,
        }

    def _proprioception_report(self) -> dict:
        return {
            "type": "proprioception",
            "capabilities": self.body.capabilities(),
            "stats": {
                "total_executions": self.execution_count,
                "successes": self.success_count,
                "errors": self.error_count,
                "last_operation": self.last_op,
            },
            "mind": {
                "parameters": sum(p.numel() for p in self.mind.parameters()),
                "vocabulary": self.tokenizer.vocab_size,
            },
        }

    def repl(self):
        """Interactive REPL — the intent interface."""
        total_params = sum(p.numel() for p in self.mind.parameters())
        print("=" * 60)
        print("  SOMA Proof of Concept v0.1")
        print("  Embodied Neural Computing")
        print("=" * 60)
        print(f"  Body:       macOS {platform.machine()}")
        print(f"  Mind:       {total_params:,} parameters (BiLSTM)")
        print(f"  Operations: {len(self.body.operations)}")
        print(f"  Vocabulary: {self.tokenizer.vocab_size} tokens")
        print("=" * 60)
        print("  Type natural language intent. No code. Just say what you want.")
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
                    print("\n  [Proprioception]")
                    print(f"  Mind: {result['mind']['parameters']:,} params, "
                          f"{result['mind']['vocabulary']} tokens")
                    print(f"  Stats: {result['stats']['total_executions']} executions, "
                          f"{result['stats']['errors']} errors")
                    if result["stats"]["last_operation"]:
                        print(f"  Last op: {result['stats']['last_operation']}")
                    print()
                    print("  Capabilities:")
                    for cap in result["capabilities"]:
                        params_str = ", ".join(
                            p["name"] for p in cap["params"]
                        ) or "none"
                        print(f"    [{cap['opcode']:2d}] {cap['name']}: "
                              f"{cap['description']} ({params_str})")

                elif result["type"] == "ambiguous":
                    print(f"\n  [Mind] {result['message']}")
                    print("  Top guesses:")
                    for name, prob in result["top_guesses"]:
                        print(f"    {name}: {prob:.1%}")

                elif result["type"] == "execution":
                    op = result["operation"]
                    conf = result["confidence"]
                    params = result["params"]
                    exec_result = result["result"]

                    print(f"\n  [Mind] {op} (confidence: {conf:.1%})")
                    if params:
                        for k, v in params.items():
                            print(f"         {k}: {v}")

                    if exec_result["success"]:
                        r = exec_result["result"]
                        if isinstance(r, list):
                            print("  [Body] Result:")
                            for item in r:
                                print(f"         {item}")
                        elif isinstance(r, dict):
                            print("  [Body] Result:")
                            for k, v in r.items():
                                print(f"         {k}: {v}")
                        else:
                            print(f"  [Body] {r}")
                    else:
                        print(f"  [Body] Error: {exec_result['error']}")

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
