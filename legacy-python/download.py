import os
import sys
import time
from pathlib import Path
import warnings
warnings.filterwarnings("ignore")

os.environ["HF_XET_HIGH_PERFORMANCE"] = "1"
os.environ["HF_HUB_ENABLE_HF_TRANSFER"] = "1"

from huggingface_hub import hf_hub_download

repo_id = "prism-ml/Ternary-Bonsai-2-27B-gguf"
local_dir = Path(__file__).resolve().parent / "models"

filename = sys.argv[1] if len(sys.argv) > 1 else "Ternary-Bonsai-2-27B-PTQ1_0.gguf"

print(f"==> Starting high-speed download of {filename} ({repo_id})...")
start_time = time.time()

file_path = hf_hub_download(
    repo_id=repo_id,
    filename=filename,
    local_dir=local_dir
)

elapsed = time.time() - start_time
size_mb = os.path.getsize(file_path) / (1024 * 1024)
size_gb = size_mb / 1024
speed_mbps = size_mb / elapsed if elapsed > 0 else 0

print(f"==> Download complete: {file_path}")
print(f"==> Size: {size_gb:.2f} GB ({size_mb:.2f} MB) in {elapsed:.1f}s ({speed_mbps:.2f} MB/s)")
