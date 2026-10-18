#!/usr/bin/env bash
# Run one speech-model trial (set up first with setup.sh). Each needs most of the graphics
# card, so the assistant is stopped first and started again afterwards (--with-assistant
# keeps it running, to see whether both fit).
#
#   run.sh qwen [0.6B|1.7B]          per-phrase synthesis, Qwen3-TTS CustomVoice (default 0.6B)
#   run.sh gepard [REFERENCE.wav]    per-phrase synthesis, Gepard 1.1 (voice from the clip)
#   run.sh vibevoice [VOICE]         per-phrase synthesis, VibeVoice Realtime 0.5B (default en-Emma_woman)
#   run.sh tyger NAME                TYGER on the take results/takes/NAME-{mic,playback}.wav
#   run.sh bonsai 9B|27B [PQ2_0|PTQ1_0]  Ternary Bonsai 2 on Prism's llama.cpp: the planner bench and speed
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
TRIALS=${TRIALS:-$HOME/.cache/nixe-assistant/trials}
args=()
keep=0
for a in "$@"; do
    if [[ $a == --with-assistant ]]; then keep=1; else args+=("$a"); fi
done
set -- "${args[@]}"
what=${1:?usage: run.sh qwen|gepard|vibevoice|tyger …}
shift

need() { [[ -f $1/.done || -d $1 ]] || { echo "not set up: $1 (run setup.sh)"; exit 1; }; }

stopped=0
if [[ $keep == 0 ]] && systemctl --user is-active -q nixe-assistantd; then
    echo "Stopping the assistant for the trial (it starts again afterwards)."
    nixe-assistant stop >/dev/null
    stopped=1
fi
restore() { [[ $stopped == 1 ]] && { echo "Starting the assistant again."; nixe-assistant start >/dev/null; }; }
trap restore EXIT
nvidia-smi --query-gpu=memory.used,memory.total --format=csv,noheader | sed 's/^/Graphics memory in use before: /'

case $what in
    qwen)
        size=${1:-0.6B}
        need "$TRIALS/qwen3-tts/.venv"
        "$TRIALS/qwen3-tts/.venv/bin/python" "$here/tts_bench.py" qwen "$TRIALS/qwen3-tts/weights/$size-CustomVoice"
        ;;
    gepard)
        g=$TRIALS/gepard/gepard-inference
        need "$g/venv"
        ref=${1:-$g/ref_audio/$(ls "$g/ref_audio" | grep -m1 '\.wav$')}
        "$g/venv/bin/python" "$here/tts_bench.py" gepard "$TRIALS/gepard/weights/gepard-1.1" --reference "$ref"
        ;;
    vibevoice)
        need "$TRIALS/vibevoice/.venv"
        "$TRIALS/vibevoice/.venv/bin/python" "$here/tts_bench.py" vibevoice "$TRIALS/vibevoice/weights/Realtime-0.5B" ${1:+--speaker "$1"}
        ;;
    tyger)
        name=${1:?run.sh tyger NAME (a take from record_pair.sh)}
        need "$TRIALS/tyger/.venv"
        "$TRIALS/tyger/.venv/bin/python" "$here/tyger_offline.py" "$here/results/takes/$name-mic.wav" "$here/results/takes/$name-playback.wav"
        ;;
    bonsai)
        size=${1:?run.sh bonsai 9B|27B [PQ2_0|PTQ1_0]}
        pack=${2:-PQ2_0}
        bin=$TRIALS/bonsai/llama.cpp
        model=$(ls "$TRIALS"/bonsai/weights/"$size"*/Ternary-Bonsai-2-"$size"-"$pack".gguf 2>/dev/null | head -1)
        need "$bin"
        [[ -e $model ]] || { echo "not downloaded: $model (run setup.sh weights)"; exit 1; }
        export LD_LIBRARY_PATH=$bin${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
        mkdir -p "$here/results"
        log=$here/results/bonsai-$size-$pack-server.log
        # The planner bench's context (12k: about 9k with every app tool offered), all on the GPU.
        "$bin/llama-server" -m "$model" -ngl 99 -c 12288 -fa on --jinja --port 8089 --host 127.0.0.1 >"$log" 2>&1 &
        server=$!
        trap 'kill $server 2>/dev/null; restore' EXIT
        nixe-assistant bench planner --server http://127.0.0.1:8089 --label "Ternary Bonsai 2 $size $pack" | tee "$here/results/bonsai-$size-$pack-planner.md"
        kill $server; wait $server 2>/dev/null || true
        echo; echo "Speed (prompt 512, generate 128):"
        "$bin/llama-bench" -m "$model" -ngl 99 -fa 1 -p 512 -n 128 | tee "$here/results/bonsai-$size-$pack-speed.md"
        ;;
    *)
        echo "unknown trial $what"; exit 2 ;;
esac
