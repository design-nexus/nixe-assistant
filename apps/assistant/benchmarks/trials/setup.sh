#!/usr/bin/env bash
# Get the speech-model trials ready to run: weights and one Python environment per model,
# all under $TRIALS (default ~/.cache/nixe-assistant/trials), apart from the assistant.
# Runs nothing on the GPU. Safe to run again: finished steps are skipped.
#
#   benchmarks/trials/setup.sh [step…]     steps: weights envs (default: both)
#

set -uo pipefail

TRIALS=${TRIALS:-$HOME/.cache/nixe-assistant/trials}
LOGS=$TRIALS/logs
export UV_HTTP_TIMEOUT=${UV_HTTP_TIMEOUT:-600}
mkdir -p "$LOGS"
steps=("$@")
[[ ${#steps[@]} -eq 0 ]] && steps=(weights envs)

ok() { printf '  \033[32m✓\033[0m %s\n' "$*"; }
bad() { printf '  \033[31m✗\033[0m %s (log: %s)\n' "$1" "$2"; failed=1; }
failed=0

# hf download REPO DIR [patterns…]: skipped when DIR/.done exists.
fetch() {
    local repo=$1 dir=$TRIALS/$2 log=$LOGS/dl-${2//\//-}.log
    shift 2
    if [[ -f $dir/.done ]]; then ok "$repo (already here)"; return; fi
    mkdir -p "$dir"
    local include=()
    for p in "$@"; do include+=(--include "$p"); done
    if uvx --from huggingface_hub hf download "$repo" --local-dir "$dir" "${include[@]}" >"$log" 2>&1; then
        touch "$dir/.done"; ok "$repo → $dir"
    else
        bad "$repo" "$log"
    fi
}

# venv DIR PYTHON-ARGS…: a Python 3.12 environment at DIR with the packages installed.
venv() {
    local dir=$TRIALS/$1 log=$LOGS/env-${1//\//-}.log
    shift
    if [[ -f $dir/.done ]]; then ok "$dir (already set up)"; return; fi
    {
        uv venv --clear --python 3.12 "$dir" && uv pip install --python "$dir/bin/python" "$@"
    } >"$log" 2>&1 && touch "$dir/.done" && ok "environment $dir" || bad "environment $dir" "$log"
}

weights() {
    echo "Weights"
    fetch Qwen/Qwen3-TTS-12Hz-0.6B-CustomVoice qwen3-tts/weights/0.6B-CustomVoice
    fetch Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice qwen3-tts/weights/1.7B-CustomVoice
    fetch nineninesix/tyger tyger/weights/tyger
    fetch nineninesix/gepard-1.1 gepard/weights/gepard-1.1
    # Gepard's checkpoint names its codec by repo id; NeMo looks for it in the Hugging Face cache.
    if uvx --from huggingface_hub hf download nvidia/nemo-nano-codec-22khz-1.89kbps-21.5fps >"$LOGS/dl-nano-codec.log" 2>&1; then
        ok "nvidia/nemo-nano-codec-22khz-1.89kbps-21.5fps → Hugging Face cache"
    else
        bad "nvidia/nemo-nano-codec-22khz-1.89kbps-21.5fps" "$LOGS/dl-nano-codec.log"
    fi
    fetch microsoft/VibeVoice-Realtime-0.5B vibevoice/weights/Realtime-0.5B
    # Its text tokenizer, by repo name: Qwen2.5-0.5B's tokenizer files only, not its weights.
    if uvx --from huggingface_hub hf download Qwen/Qwen2.5-0.5B tokenizer.json tokenizer_config.json vocab.json merges.txt >"$LOGS/dl-qwen2.5-tokenizer.log" 2>&1; then
        ok "Qwen/Qwen2.5-0.5B tokenizer files → Hugging Face cache"
    else
        bad "Qwen/Qwen2.5-0.5B tokenizer files" "$LOGS/dl-qwen2.5-tokenizer.log"
    fi
    # Ternary Bonsai 2 (Prism's packings, which only Prism's llama.cpp loads).
    fetch prism-ml/Ternary-Bonsai-2-9B-gguf bonsai/weights/9B 'Ternary-Bonsai-2-9B-PQ2_0.gguf'
    fetch prism-ml/Ternary-Bonsai-2-27B-gguf bonsai/weights/27B-pq2 'Ternary-Bonsai-2-27B-PQ2_0.gguf'
    # The 27B: the PTQ1_0 LM Studio already has, linked rather than downloaded again.
    local lms=$HOME/.lmstudio/models/prism-ml/Ternary-Bonsai-2-27B-gguf/Ternary-Bonsai-2-27B-PTQ1_0.gguf
    mkdir -p "$TRIALS/bonsai/weights/27B"
    if [[ -f $lms ]]; then
        ln -sf "$lms" "$TRIALS/bonsai/weights/27B/Ternary-Bonsai-2-27B-PTQ1_0.gguf" && ok "27B PTQ1_0 (LM Studio's copy, linked)"
    else
        fetch prism-ml/Ternary-Bonsai-2-27B-gguf bonsai/weights/27B-ptq1 'Ternary-Bonsai-2-27B-PTQ1_0.gguf'
    fi
}

# Prism's llama.cpp fork, its CUDA 13 release build (PQ2_0/PTQ1_0 and the Hadamard transform).
PRISM_TAG=${PRISM_TAG:-prism-b10777-e3de715}
prism() {
    local dir=$TRIALS/bonsai/llama.cpp log=$LOGS/prism-llama.log
    if [[ -f $dir/.done ]]; then ok "Prism llama.cpp $(cat "$dir/.done") (already here)"; return; fi
    mkdir -p "$dir"
    local url=https://github.com/PrismML-Eng/llama.cpp/releases/download/$PRISM_TAG/llama-$PRISM_TAG-bin-linux-cuda-13.3-x64.tar.gz
    if curl -fsSL "$url" -o "$dir/build.tar.gz" 2>"$log" && tar -xzf "$dir/build.tar.gz" -C "$dir" --strip-components=1 2>>"$log"; then
        rm -f "$dir/build.tar.gz"; echo "$PRISM_TAG" >"$dir/.done"; ok "Prism llama.cpp $PRISM_TAG → $dir"
    else
        bad "Prism llama.cpp $PRISM_TAG" "$log"
    fi
}

envs() {
    echo "Environments"
    prism
    # cu128 wheels carry sm_120 (Blackwell) kernels; torch comes with each package so a
    # package's own torch requirement can't pull a CPU build.
    venv qwen3-tts/.venv --torch-backend cu128 torch torchaudio qwen-tts soundfile
    venv vibevoice/.venv --torch-backend cu128 torch soundfile -e "$TRIALS/vibevoice/VibeVoice[streamingtts]"
    # Only TYGER's EARS engine runs here, not tyger-s2s's full agent (vLLM, VoxCPM2), so only
    # its packages, at the torch and transformers versions tyger-s2s is tested with.
    venv tyger/.venv --torch-backend auto torch==2.13.0 transformers==5.14.1 safetensors==0.8.0 \
        huggingface_hub soundfile==0.14.0 librosa==1.0.0
    # Gepard: its scripts/setup.sh picks a CPU torch on CUDA 13.4 and needs a torchcodec that
    # can't load Arch's FFmpeg 9, so the same stack in one resolve: CUDA torch, NeMo's codec,
    # the package. torchcodec is only for transformers' speech recognition, which Gepard
    # doesn't use, so it's removed.
    local g=$TRIALS/gepard/gepard-inference
    if [[ -f $g/venv/.done ]]; then
        ok "environment $g/venv (already set up)"
    elif {
        uv venv --clear --python 3.12 "$g/venv" &&
            # pyarrow 21+ needs NumPy 2, which NeMo 2.4 doesn't allow; Gepard only gets it via datasets.
            printf 'transformers==5.3.0\npyarrow<21\n' >"$TRIALS/gepard/overrides.txt" &&
            # NeMo 2.4 caps transformers at 4.52; Gepard needs 5.3 and its own script installs it
            # over NeMo's. The override does that in the one resolve.
            uv pip install --python "$g/venv/bin/python" --torch-backend cu128 --override "$TRIALS/gepard/overrides.txt" torch torchaudio "nemo-toolkit[tts]==2.4.0" -e "$g[inference]" &&
            uv pip uninstall --python "$g/venv/bin/python" torchcodec
    } >"$LOGS/env-gepard.log" 2>&1; then
        touch "$g/venv/.done"; ok "environment $g/venv"
    else
        bad "environment $g/venv" "$LOGS/env-gepard.log"
    fi
}

clone() {
    local url=$1 dir=$TRIALS/$2
    [[ -d $dir/.git ]] || git clone -q --depth 1 "$url" "$dir"
}
clone https://github.com/nineninesix-ai/tyger-s2s tyger/tyger-s2s
clone https://github.com/nineninesix-ai/gepard-inference gepard/gepard-inference
clone https://github.com/microsoft/VibeVoice vibevoice/VibeVoice

for s in "${steps[@]}"; do
    case $s in
        weights) weights ;;
        envs) envs ;;
        *) echo "unknown step $s (weights, envs)"; exit 2 ;;
    esac
done
exit $failed
