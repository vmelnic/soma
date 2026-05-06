import torch
from collections import Counter

td = torch.load("data/train_qa.pt", map_location="cpu", weights_only=False)
answers = td["answers"]
ac = Counter(answers)
total = len(answers)

print(f"unique answers: {len(ac)}")
print(f"total QA pairs: {total}")
print()

for n in [1000, 2000, 5000, 10000, 20000]:
    top = set(a for a, _ in ac.most_common(n))
    c = sum(1 for a in answers if a in top)
    print(f"  top {n}: {c}/{total} = {c/total:.1%}")

print()
print("top 15 answers:")
for a, c in ac.most_common(15):
    print(f"  '{a}': {c}")
