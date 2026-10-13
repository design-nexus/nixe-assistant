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
| 2026-10-06 | Hades RTX 5080 | Gemma 4 E4B Q4 | warm Parakeet (`nyx-parakeet`, `voxtype-onnx-avx2`) | Kokoro CPU (CUDA skipped, no sm_120 ORT) | 198 | 0 | 164 | 426 | 0 | **788** | 3848 MiB | PTT mic: “Hello.” → “Hello there.” Key-up is VAD end. Same daemon as the 643 ms Kokoro / 1329 ms GGUF load below. |

## Audio microbenchmarks

Targets (not gates):

| Check | Target | How | Measured |
|---|---|---|---|
| Capture start after `ptt.start` | small tens of ms | time `StartCapture` → first PCM frame | **23 ms** (`capture first frame`, 2026-10-06, 512-sample / 32 ms frame) |
| Playback start after first TTS PCM | small tens of ms | TTS begins → audio playback | **0 ms** in the timing log (both stamps set in `speak_phrase` before `play()`) |
| Playback cancel | < 100 ms | `cancel` while speaking; queue flush | **7 µs** queued + **1 µs** flushed (`playback cancel queued/flushed`, 2026-10-06) |

## Model load (one-shot)

Record at daemon startup from the log lines `GGUF loaded` and `Kokoro ready`.

| Date | Model | Load ms | Notes |
|---|---|---|---|
| 2026-10-05 | gemma-4-E4B-it Q4_K_M | 1817 (warm) / 3738 (first) | llama.cpp CUDA, n_gpu_layers=1000, sm_120 `120a-real` |
| 2026-10-05 | Kokoro-82M ONNX af_heart | 3447 including failed CUDA attempt | CPU EP. CUDA EP failed: missing `libcudnn.so` |
| 2026-10-06 | gemma-4-E4B-it Q4_K_M | 1329 | llama.cpp CUDA, n_gpu_layers=1000, sm_120 `120a-real` |
| 2026-10-06 | Kokoro-82M ONNX af_heart | 643 | CPU EP. CUDA skipped up front (`compute_cap` 12.* / skip stamp); no failed CUDA load |

## Known expected bottlenecks (measure before changing)

1. Warm Parakeet STT is ~175–260 ms on this host. A cold take this morning logged STT 3227 ms.
2. First Kokoro phrase waits for a sentence boundary or 12 words. On the “Hello.” turn, TTS was 426 ms of the 788 ms total.
3. Chat TTS runs on a phrase queue so llama.cpp keeps emitting tokens while Kokoro synthesizes. Playback was already non-blocking; the stall was the token loop awaiting TTS.
4. Kokoro stays on CPU on Blackwell (ORT has no sm_120 kernels). llama.cpp GGUF still uses CUDA.
