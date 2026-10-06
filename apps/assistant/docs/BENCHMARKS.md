# Nyx benchmarks

Primary metric: **speech end → first audible response word**.

Times are milliseconds from VAD/key-up (`VAD end` = 0). Copy the daemon timing
trace into the results table. Do not invent numbers.

## Trace format

Logged at info after every successful turn:

```
Nyx timing
  VAD end                 0 ms
  STT complete          +… ms
  routing               +… ms
  LLM first token       +… ms
  TTS begins            +… ms
  audio playback        +… ms
  TOTAL                   … ms
```

`routing` is the intent matcher. High-confidence desktop phrases skip the LLM
(LLM first token stays 0). `audio playback` is when PCM is queued to PipeWire,
not when the speaker cone moves.

## How to capture

```bash
RUST_LOG=info journalctl --user -u omarchy-assistantd -f
# or run in a terminal:
RUST_LOG=info omarchy-assistantd
```

Hold Alt+Space, speak one short sentence, release. Paste the `Nyx timing` block
and fill a row below.

Also record, when practical:

- `nvidia-smi` VRAM of `omarchy-assistantd` at idle (models loaded)
- GGUF path / name
- Voxtype variant (`voxtype info`)
- whether Kokoro ran on CUDA or CPU

## Results

| Date | Host | GGUF | STT | TTS | VAD→STT | STT→route | route→TTFT | TTFT→TTS | TTS→audio | TOTAL | VRAM idle | Notes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-10-05 | Hades RTX 5080 | Gemma 4 E4B Q4 | voxtype whisper `base.en` CPU | Kokoro CPU | 861 | 0 | 139 | 237 | 0 | **1237** | ~3860 MiB | `omarchy-assistant demo`: Kokoro “What is two plus two?” → Voxtype “What is 2 plus 2?” → LLM “Four.” → speakers. No mic/VAD. Probe synth excluded from the clock. |

## Audio microbenchmarks

Targets (not gates):

| Check | Target | How |
|---|---|---|
| Capture start after `ptt.start` | small tens of ms | time `StartCapture` → first PCM frame |
| Playback start after first TTS PCM | small tens of ms | TTS begins → audio playback |
| Playback cancel | < 100 ms | `cancel` while speaking; queue flush |

Fill after the first spoken turn works.

## Model load (one-shot)

Record at daemon startup from the log lines `GGUF loaded` and `Kokoro ready`.

| Date | Model | Load ms | Notes |
|---|---|---|---|
| 2026-10-05 | gemma-4-E4B-it Q4_K_M | 1817 (warm) / 3738 (first) | llama.cpp CUDA, n_gpu_layers=1000, sm_120 `120a-real` |
| 2026-10-05 | Kokoro-82M ONNX af_heart | 3447 including failed CUDA attempt | CPU EP. CUDA EP failed: missing `libcudnn.so` |

## Known expected bottlenecks (measure before changing)

1. `voxtype transcribe` may cold-start Whisper `base.en` on CPU each utterance.
2. First Kokoro phrase waits for a sentence boundary or 12 words.
3. Sequential TTS-then-play (no overlap with the next phrase yet).
