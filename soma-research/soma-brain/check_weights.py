import torch

data = torch.load("checkpoints/brain_base.pt", map_location="cpu", weights_only=False)
sd = data["state_dict"]
eye1024 = torch.eye(1024)

for k in sorted(sd.keys()):
    v = sd[k]
    if "proj" not in k:
        continue
    if v.dim() == 2 and v.shape[0] == v.shape[1] == 1024:
        is_eye = torch.allclose(v, eye1024, atol=1e-5)
        print(f"{k}: {list(v.shape)} is_identity={is_eye} diag_mean={v.diag().mean():.4f}")
    elif v.dim() == 1:
        print(f"{k}: {list(v.shape)} all_zero={torch.allclose(v, torch.zeros_like(v), atol=1e-5)} max={v.abs().max():.6f}")
    else:
        print(f"{k}: {list(v.shape)}")
