# Speech-model trials

Models the assistant doesn't use yet, set up apart from it (`~/.cache/nixe-assistant/trials`)
so they can be measured on this computer before anything is wired in.

| Trial | What it is | Compared with | Weights | Licence |
|---|---|---|---|---|
| Qwen3-TTS 0.6B / 1.7B CustomVoice | Speech synthesis, nine built-in voices | Kokoro | 2.5 / 4.5 GB | Apache 2.0 |
| Gepard 1.1 | Speech synthesis made for live dialogue; clones a voice from a clip | Kokoro | 1.1 GB + 0.5 GB codec | Apache 2.0 |
| VibeVoice Realtime 0.5B (Microsoft) | Streaming speech synthesis, English, preset voices | Kokoro | 2.0 GB (+ Qwen2.5's tokenizer files) | MIT |
| TYGER | Turn-taking: when to answer, stop, or say "mm-hm" (Nemotron ASR + a small GRU) | the rules in `duplex.rs` | 2.6 GB | OpenMDW 1.1 |
| Ternary Bonsai 2 9B / 27B | Ternary language models, only on Prism's llama.cpp fork | Gemma 4 E4B as the planner | 9B: 2.4 (PQ2_0) or 2.0 GB (PTQ1_0); 27B: LM Studio's PTQ1_0 (5.9 GB) | Apache 2.0 |

## Set up

```bash
benchmarks/trials/setup.sh            # weights and environments; safe to run again
```

Only what fits this computer's 16 GB graphics card, with the assistant stopped, is set up.

## Run

Each trial stops the assistant first (they need the graphics card) and starts it again
afterwards; add `--with-assistant` to see whether both fit.

```bash
benchmarks/trials/run.sh qwen 0.6B       # then: run.sh qwen 1.7B
benchmarks/trials/run.sh gepard          # a reference clip from gepard-inference/ref_audio, or give one
benchmarks/trials/run.sh vibevoice       # en-Emma_woman; or en-Carter_man, en-Grace_woman, …
benchmarks/trials/record_pair.sh stop1   # talk over a long reply: "stop", "mm-hm", a new request
benchmarks/trials/run.sh tyger stop1
benchmarks/trials/run.sh bonsai 9B PQ2_0 # or PTQ1_0; and: run.sh bonsai 27B PTQ1_0
```

Results land in `results/` (JSON, and each phrase's audio for listening).

- **Speech synthesis** (`tts_bench.py`): a short first phrase (it decides when the first word is
  heard), a typical one and a long one, the median of three after a warm-up: milliseconds,
  real-time factor, peak graphics memory. Both reference stacks return a phrase at a time,
  which is how the assistant feeds Kokoro; Gepard's ~50 ms first-chunk streaming needs its vLLM
  build (`gepard-vllm`), not set up here.
- **TYGER** (`tyger_offline.py`): its decisions on a recorded take (the raw microphone and the
  speakers, recorded together), each with its time, and how long each 320 ms hop took (it must
  stay under 320 ms). Compare with the assistant's own decisions for the same take in
  `bench duplex`'s log: does "stop" stop, and how much sooner is a redirect caught?
- **Bonsai 2**: the planner bench (`bench planner --server`, the same cases, prompt and grammar
  as for the bundled models, through Prism's `llama-server` because the assistant's own
  llama.cpp can't load ternary packings) and `llama-bench` speed. Prism's build is its latest
  release (`setup.sh` pins the tag); LM Studio's Bonsai runtime has an older one.

