# Persona voices

Persona prompts and wake aliases remain in `~/.config/nyx-assistant/personas/<id>.toml`.
The active voice and Aura tint come from `~/.config/nyx-assistant/voices/<id>.toml`.
On first load, Assistant creates a voice profile from each persona's legacy `voice` field;
existing profiles are never overwritten. The legacy field remains readable for old
profiles, but editing it no longer changes an existing voice profile.

```toml
persona_id = "iris"
provider = "kokoro"
voice = "af_bella"
aura_tint = "accent" # accent, highlight, danger, border; follows the Nyx theme
# aura_style = "full" # optional persona override; otherwise [ui].aura_style
```

`nyx-assistant persona set iris` loads Iris's voice profile. `persona voices` lists
voices present in the local Kokoro pack. `persona voice-override af_bella` selects
an installed voice across personas; `persona voice-override clear` restores each
persona's own voice. The TUI voice cycle sets this same override. No cloud or
voice cloning interface is exposed.

Kokoro is the default for every profile. A developer can distribute a Chatterbox
profile with a licensed reference recording:

```toml
persona_id = "iris"
provider = "chatterbox"
voice = "iris-developer" # descriptive profile name; Kokoro fallback comes from persona.toml
reference_audio = "/absolute/path/to/licensed/iris.wav"
aura_tint = "accent"
```

The local Chatterbox worker starts only when that persona speaks, stays loaded
for follow-up phrases, and unloads after two idle minutes or when switching away.
Cancelling a turn stops in-progress synthesis and discards its audio. An installed Kokoro voice
override takes precedence. Install its optional runtime with
`apps/assistant/scripts/install-chatterbox.sh`. The runtime runs offline after
its weights are cached; synthesis errors fall back to Kokoro and are logged.
Status then reports Kokoro, and a configuration reload allows a new clone attempt.

## Synthesis backend decision

Kokoro remains the shipped backend. Our [local trials](BENCHMARKS.md) measured
Kokoro on CPU at 824 ms for a typical phrase, versus 2.8–3.3 s for Qwen3-TTS
CustomVoice on GPU, with 2.4–4.3 GB additional VRAM. That trial compared preset
voices; it did not measure cloned voice fidelity or Qwen's streaming first audio.
Qwen's Base models support reference-audio voice cloning, but integrating them
requires a separate local inference worker and a sample-level streaming trial.

Chatterbox Turbo supports reference-clip cloning through its Python/PyTorch
runtime. Its model repository is roughly 4 GB. On this RTX 5080 with the Assistant
daemon resident, a local reference-clip trial measured a 17.4 s cold load,
646/982/1734 ms for first/typical/long phrases, and 3318 MiB peak allocated VRAM.
The full trial record and WAV outputs are in `benchmarks/trials/results/`.
Naturalness and likeness to a licensed developer reference still need human
review; streaming first-audio latency and cancellation recovery were not measured.
The evaluation gate is a blind listening comparison on the same approved
reference clips and phrases, plus warm/cold latency, peak memory, and concurrent
STT/LLM/TTS measurements. No replacement is selected from unmeasured claims.
Distributed persona voices require rights to the reference recordings and the
resulting likeness; the profile format does not grant those rights.

Sources: [Qwen3-TTS implementation](https://github.com/QwenLM/Qwen3-TTS),
[Chatterbox implementation](https://github.com/resemble-ai/chatterbox),
[Chatterbox Turbo model files](https://huggingface.co/ResembleAI/chatterbox-turbo/tree/main).
