"""Per-phrase speech synthesis, as the assistant uses it: one phrase in, its audio out.

    <env>/bin/python tts_bench.py qwen   WEIGHTS [--speaker NAME]
    <env>/bin/python tts_bench.py gepard WEIGHTS [--reference CLIP.wav]
    <env>/bin/python tts_bench.py vibevoice WEIGHTS [--speaker NAME]

Times a short first phrase (what decides when the first word is heard), a typical phrase
and a long one, three times each after a warm-up, and writes the median time, real-time
factor and peak graphics memory to results/<backend>-<weights>.json, with each phrase's
audio beside it for listening. Kokoro's numbers for the same phrases are in
docs/BENCHMARKS.md ("Model load", "Streaming speech").
"""
import argparse
import json
import statistics
import sys
import time
from pathlib import Path

PHRASES = {
    "first": "Sure, I can do that.",
    "typical": "The weather in Chicago is clear right now, with a high of fifty-two degrees.",
    "long": "I found three files that match your search: the tax return from last April, a scanned receipt from the hardware store, and a draft letter you started on Sunday evening.",
}
RUNS = 3


def qwen(weights: str, speaker: str | None):
    import torch
    from qwen_tts import Qwen3TTSModel

    model = Qwen3TTSModel.from_pretrained(weights, device_map="cuda:0", dtype=torch.bfloat16)
    speakers = list(model.get_supported_speakers())
    voice = speaker or next((s for s in speakers if s.lower() in ("ryan", "aiden")), speakers[0])

    def speak(text):
        wavs, sr = model.generate_custom_voice(text=text, language="English", speaker=voice)
        return wavs[0], sr

    return speak, {"speaker": voice, "speakers": speakers}


def gepard(weights: str, reference: str | None):
    repo = Path.home() / ".cache/nixe-assistant/trials/gepard/gepard-inference"
    sys.path.insert(0, str(repo))
    from gepard_inference import GepardSession, SessionConfig  # noqa: E402

    # The repo's generation defaults, with this checkpoint and voice.
    cfg = SessionConfig.from_yaml(repo / "config.yaml")
    cfg.checkpoint = weights
    if reference:
        cfg.reference_audio = str(Path(reference).resolve())
    session = GepardSession(cfg, device="cuda").load()

    def speak(text):
        sr, wav = session.synthesize(text)
        return wav, sr

    return speak, {"reference": cfg.reference_audio}


def vibevoice(weights: str, speaker: str | None):
    import copy

    import torch
    from transformers.cache_utils import DynamicCache
    from transformers.modeling_outputs import BaseModelOutputWithPast
    from vibevoice.modular.modeling_vibevoice_streaming_inference import VibeVoiceStreamingForConditionalGenerationInference
    from vibevoice.processor.vibevoice_streaming_processor import VibeVoiceStreamingProcessor

    voices = Path.home() / ".cache/nixe-assistant/trials/vibevoice/VibeVoice/demo/voices/streaming_model"
    name = speaker or "en-Emma_woman"
    processor = VibeVoiceStreamingProcessor.from_pretrained(weights)
    # Microsoft's demo prefers FlashAttention 2 (not installed here); SDPA is its fallback.
    model = VibeVoiceStreamingForConditionalGenerationInference.from_pretrained(weights, torch_dtype=torch.bfloat16, device_map="cuda", attn_implementation="sdpa")
    model.eval()
    model.set_ddpm_inference_steps(num_steps=5)
    # The voice presets are pickled model outputs that PyTorch 2.11's weights-only loader can't
    # rebuild (Microsoft's demo targets an older PyTorch). They come from Microsoft's own repo,
    # cloned by setup.sh, so the full loader is used for them.
    with torch.serialization.safe_globals([BaseModelOutputWithPast, DynamicCache]):
        prompt = torch.load(voices / f"{name}.pt", map_location="cuda", weights_only=False)

    def speak(text):
        inputs = processor.process_input_with_cached_prompt(text=text, cached_prompt=prompt, padding=True, return_tensors="pt", return_attention_mask=True)
        inputs = {k: v.to("cuda") if torch.is_tensor(v) else v for k, v in inputs.items()}
        out = model.generate(**inputs, max_new_tokens=None, cfg_scale=1.5, tokenizer=processor.tokenizer, generation_config={"do_sample": False}, verbose=False, all_prefilled_outputs=copy.deepcopy(prompt))
        return out.speech_outputs[0].float().cpu().numpy(), 24000

    return speak, {"speaker": name, "attention": "sdpa", "voices": sorted(p.stem for p in voices.glob("en-*.pt"))}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("backend", choices=["qwen", "gepard", "vibevoice"])
    ap.add_argument("weights")
    ap.add_argument("--speaker")
    ap.add_argument("--reference")
    a = ap.parse_args()

    import numpy as np
    import soundfile as sf
    import torch

    out = Path(__file__).resolve().parent / "results"
    out.mkdir(exist_ok=True)
    name = f"{a.backend}-{Path(a.weights.rstrip('/')).name}" + (f"-{a.speaker}" if a.speaker else "")
    torch.cuda.reset_peak_memory_stats()
    t = time.perf_counter()
    loaders = {"qwen": lambda: qwen(a.weights, a.speaker), "gepard": lambda: gepard(a.weights, a.reference), "vibevoice": lambda: vibevoice(a.weights, a.speaker)}
    speak, info = loaders[a.backend]()
    load_s = time.perf_counter() - t
    speak("Warming up.")
    rows = {}
    for key, text in PHRASES.items():
        times, audio_s = [], 0.0
        for i in range(RUNS):
            torch.cuda.synchronize()
            t = time.perf_counter()
            wav, sr = speak(text)
            torch.cuda.synchronize()
            times.append(time.perf_counter() - t)
            wav = np.asarray(wav, dtype=np.float32).reshape(-1)
            audio_s = len(wav) / sr
            if i == 0:
                sf.write(out / f"{name}-{key}.wav", wav, sr)
        med = statistics.median(times)
        rows[key] = {"text": text, "words": len(text.split()), "median_ms": round(med * 1000), "audio_s": round(audio_s, 2), "rtf": round(med / audio_s, 3) if audio_s else None}
        print(f"{key:8} {rows[key]['median_ms']:6} ms for {audio_s:5.2f} s of audio (RTF {rows[key]['rtf']})", flush=True)
    result = {"backend": a.backend, "weights": a.weights, "load_s": round(load_s, 1), "peak_vram_mib": round(torch.cuda.max_memory_allocated() / 2**20), "phrases": rows, **info}
    (out / f"{name}.json").write_text(json.dumps(result, indent=2))
    print(f"load {result['load_s']} s, peak VRAM {result['peak_vram_mib']} MiB → {out / (name + '.json')}")


if __name__ == "__main__":
    main()
