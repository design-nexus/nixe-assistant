# Assistant benchmarks

Primary metric: **speech end → first audible response word**.

Times are milliseconds from VAD/key-up (`VAD end` = 0). Copy the daemon timing
trace into the results table. Do not invent numbers.

## Trace format

Logged at info after every successful turn:

```
Assistant timing
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
RUST_LOG=info journalctl --user -u nixe-assistantd -f
# or run in a terminal:
RUST_LOG=info nixe-assistantd
```

Hold Alt+Space, speak one short sentence, release. Paste the `Assistant timing` block
and fill a row below.

Also record, when practical:

- `nvidia-smi` VRAM of `nixe-assistantd` at idle (models loaded)
- GGUF path / name
- Voxtype variant (`voxtype info`)
- whether Kokoro ran on CUDA or CPU

## Results

| Date | Host | GGUF | STT | TTS | VAD→STT | STT→route | route→TTFT | TTFT→TTS | TTS→audio | TOTAL | VRAM idle | Notes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-10-05 | Hades RTX 5080 | Gemma 4 E4B Q4 | voxtype whisper `base.en` CPU | Kokoro CPU | 861 | 0 | 139 | 237 | 0 | **1237** | ~3860 MiB | `nixe-assistant demo`: Kokoro “What is two plus two?” → Voxtype “What is 2 plus 2?” → LLM “Four.” → speakers. No mic/VAD. Probe synth excluded from the clock. |
| 2026-10-06 | Hades RTX 5080 | Gemma 4 E4B Q4 | warm Parakeet (`nixe-assistant-stt`, `voxtype-onnx-avx2`) | Kokoro CPU (CUDA skipped, no sm_120 ORT) | 198 | 0 | 164 | 426 | 0 | **788** | 3848 MiB | PTT mic: “Hello.” → “Hello there.” Key-up is VAD end. Same daemon as the 643 ms Kokoro / 1329 ms GGUF load below. |

## Audio microbenchmarks

Targets (not gates):

| Check | Target | How | Measured |
|---|---|---|---|
| Capture start after `ptt.start` | small tens of ms | time `StartCapture` → first PCM frame | **23 ms** (`capture first frame`, 2026-10-06, 512-sample / 32 ms frame) |
| Playback start after first TTS PCM | small tens of ms | TTS begins → audio playback | **0 ms** in the timing log (both stamps set in `speak_phrase` before `play()`) |
| Playback cancel | < 100 ms | `cancel` while speaking; queue flush | **7 µs** queued + **1 µs** flushed (`playback cancel queued/flushed`, 2026-10-06) |

## Streaming speech (Phase 5, 2026-10-14)

Nemotron streaming 0.6B, 560 ms chunks, int8, through sherpa-onnx 1.13.8
(`sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25`), Core Ultra 9 285H.

| Check | Result | How |
|---|---|---|
| Speed idle | RTF 0.2 (1 thread), 0.1 (4 threads) | sherpa CLI, 6.6 s LibriSpeech clip |
| Speed under load | RTF 0.2 (1), 0.14 (2), 0.1 (4) | same, while a long `say` turn ran (LLM + Kokoro), load average 10 |
| CUDA (ORT 1.28.2, CUDA 13) | runs on sm_120, no faster than CPU (≈ RTF 0.12) | int8 ops gain nothing on the GPU; the 2-thread CPU path is used |
| WER, 20 Kokoro command clips | Nemotron 6.3 % (≈ 4 % without spelling variants), Parakeet 3.1 % | `voxtype transcribe --engine parakeet` for Parakeet |
| WER, 10 of Ken's recordings | Nemotron 8.3 % (4.2 % without "Rip Grep"), Parakeet 6.2 % | 48 words, echo-cancelled source |
| Final after speech end | ≈ 140 ms (worker), 144 ms in a live PTT turn | `final … decode_ms` / timing trace |
| Short words | "stop" needs ~1 s of trailing audio; the worker pads 1 s (and 0.25 s before) | |

Known limit: some very quick single words ("stop" in 200 ms) come back empty from Nemotron
where Parakeet hears them. Adding Parakeet as a second opinion was offered and declined;
a missed "stop" lets the reply carry on (say it again, or Escape).

## Duplex (Phase 5, 2026-10-14)

`nixe-assistant bench duplex [--echo-only]`; numbers from the daemon's `interruption` and
`timing` events.

| Check | Target (§4.6) | Measured |
|---|---|---|
| False onsets / interruptions, 3 long replies, nobody talking | 0 | **0 / 0** |
| Pause or duck after speech onset | < 150 ms | **2–14 µs** after the onset is detected (Silero confirms onset after 2 × 32 ms frames) |
| Stale audio dropped (cancel generation) | < 100 ms | **2–5 µs** (`generation cancelled` log, 4 live interruptions) |
| Redirect decided | — | 1.0–1.9 s after onset (waits for three content words or the end of speech) |
| First audio, text turn incl. routing, under its own LLM + TTS load | < 900 ms | 607 / 980 / 1293 ms (median 980) |

Live trials (`bench duplex`, 2026-10-14, Ken speaking at normal volume while she counts):

| Said | Verdicts | Paused after onset | Decided after onset |
|---|---|---|---|
| mm-hm | noise “”, noise “” | 0.01–0.02 ms | 840 ms |
| stop | noise “”, noise “” | 0.00–0.01 ms | 805–826 ms |
| actually, what's the weather in Paris? | redirect “Actually, what's the weather in Paris” | 0.01 ms | 1897 ms |

Echo in the same run: 0 onsets over two short replies; first audio 425 / 954 ms.
"mm-hm" comes back without words, which has the right effect (she carries on). "stop" also
comes back without words, so it does not stop the reply: the recognizer limit above. Escape,
Alt+Space or a longer request still stop or redirect her.

The first-audio figure includes a routing step that took 2.4 s in one earlier long-prompt
turn; for short replies it is in the 0.3–0.5 s range (demo turns: 906–1180 ms total).

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
