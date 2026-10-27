#!/usr/bin/env bash
# Optional local cloned-voice runtime. Kokoro needs none of this.
set -euo pipefail
root="${XDG_DATA_HOME:-$HOME/.local/share}/nyx-assistant/voice-engines/chatterbox"
mkdir -p "$root"
uv venv --python 3.11 "$root/.venv"
uv pip install --python "$root/.venv/bin/python" chatterbox-tts==0.1.7
# Chatterbox's pinned Torch 2.6 lacks kernels for this machine's Blackwell GPU.
uv pip install --python "$root/.venv/bin/python" --torch-backend cu128 --upgrade torch==2.11.0 torchaudio==2.11.0
uvx --from huggingface_hub hf download ResembleAI/chatterbox-turbo
install -m 0644 "$(dirname "$0")/../voice-workers/chatterbox.py" "$root/worker.py"
