"""T5 staging probes: comfyui pre-torch leaves + torch import wedge capture.

Run under antipythony with the local comfyui + site-packages staged:
  antipythony run tests/t5-probe/torch_stage_probes.py

Stage 1: import comfy.options          -> RC=0 on antipythony (green)
Stage 2: import folder_paths           -> RC=0 on antipythony (green)
Stage 3: import torch                  -> HANGS pre-dlopen (Wall 1/2), never
          reaches the 186-symbol dlopen wall (predicted, not byte-captured).
"""
import sys
import time

COMFY = "/Users/caimlas/git/ai-infra/services/comfyui"
SITE = "/opt/homebrew/lib/python3.14/site-packages"

for p in (COMFY, SITE):
    if p not in sys.path:
        sys.path.insert(0, p)

t0 = time.monotonic()
import comfy.options  # noqa: E402
comfy.options.enable_args_parsing()
print("STAGE1 comfy.options OK", flush=True)

import folder_paths  # noqa: E402
print("STAGE2 folder_paths OK", flush=True)

print("STAGE3 import torch (the wall)", flush=True)
import torch  # noqa: E402

print("STAGE3 torch OK", torch.__version__, time.monotonic() - t0, flush=True)
print("DONE", flush=True)
