#!/usr/bin/env bash
# Record a take for tyger_offline.py: the microphone (raw, before the assistant's echo
# cancelling) and what the speakers play, at the same time, as two aligned 16 kHz mono files.
#
#   benchmarks/trials/record_pair.sh NAME [SECONDS]     (default 30; Ctrl+C stops early)
#
# Writes results/takes/NAME-mic.wav and NAME-playback.wav. Talk to the assistant while it
# runs (say "mm-hm", "stop" or a new request over a long reply); `bench duplex` logs what the
# assistant decided for the same moments.
set -euo pipefail
name=${1:?usage: record_pair.sh NAME [SECONDS]}
seconds=${2:-30}
dir=$(dirname "$0")/results/takes
mkdir -p "$dir"
mic=$dir/$name-mic.wav
play=$dir/$name-playback.wav
pw-record --rate 16000 --channels 1 "$mic" &
a=$!
pw-record --rate 16000 --channels 1 -P '{ stream.capture.sink = true }' "$play" &
b=$!
trap 'kill $a $b 2>/dev/null; wait 2>/dev/null; echo "→ $mic, $play"' EXIT
echo "Recording $seconds s (Ctrl+C stops)…"
sleep "$seconds"
