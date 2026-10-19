"""TYGER's turn-taking decisions on a recorded exchange, offline: the same engine tyger-s2s
runs, fed 320 ms hops from two aligned 16 kHz recordings, the microphone and the
assistant's own playback (record both with record_pair.sh).

    ~/.cache/nixe-assistant/trials/tyger/.venv/bin/python tyger_offline.py MIC.wav PLAYBACK.wav

A hop counts as the assistant speaking when its playback is louder than --speaking-db.
Prints each event (ENDPOINT, BARGE, BACKCHANNEL) with its time, and writes them, the time
each hop took (it must stay under 320 ms) and peak graphics memory to
results/tyger-<mic name>.json, to compare with what `bench duplex` logged for the same take.
"""
import argparse
import json
import os
import statistics
import sys
import time
from pathlib import Path

TRIALS = Path.home() / ".cache/nixe-assistant/trials"
sys.path.insert(0, str(TRIALS / "tyger/tyger-s2s"))


def load(path, sr):
    import numpy as np
    import soundfile as sf

    wav, rate = sf.read(path, dtype="float32", always_2d=True)
    wav = wav.mean(axis=1)
    if rate != sr:
        import librosa

        wav = librosa.resample(wav, orig_sr=rate, target_sr=sr)
    return wav.astype(np.float32)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mic")
    ap.add_argument("playback")
    ap.add_argument("--bundle", default=str(TRIALS / "tyger/weights/tyger"))
    ap.add_argument("--speaking-db", type=float, default=-45.0)
    a = ap.parse_args()

    import numpy as np
    import torch
    from app.plugins.ears.tyger.constants import FRAME_S, HOP_N, SR
    from app.plugins.ears.tyger.engine import EarsEngine
    from app.plugins.ears.tyger.session import EarsSession

    mic, play = load(a.mic, SR), load(a.playback, SR)
    n = min(len(mic), len(play))
    t = time.perf_counter()
    engine = EarsEngine(a.bundle, slots=1)
    load_s = time.perf_counter() - t
    sess = EarsSession(0)
    events, hop_ms = [], []
    for k in range(n // HOP_N):
        uh, ah = mic[k * HOP_N:(k + 1) * HOP_N], play[k * HOP_N:(k + 1) * HOP_N]
        rms_db = 20 * np.log10(np.sqrt(np.mean(ah**2)) + 1e-9)
        mode = "speaking" if rms_db > a.speaking_db else "listen"
        t = time.perf_counter()
        found, _finals = engine.process([(sess, uh, ah, mode)])[sess]
        torch.cuda.synchronize()
        hop_ms.append((time.perf_counter() - t) * 1000)
        end_s = (k + 1) * HOP_N / SR
        for ev in found:
            if ev["kind"] in ("ENDPOINT", "BARGE", "BACKCHANNEL"):
                ev = {"t_s": round(end_s, 2), "mode": mode, **{k2: v for k2, v in ev.items() if k2 != "audio"}}
                events.append(ev)
                print(json.dumps(ev), flush=True)
                if ev["kind"] in ("ENDPOINT", "BARGE"):
                    sess.reset()  # as tyger-s2s's conversation does: the next hop starts a new turn
    hop_ms.sort()
    result = {
        "mic": a.mic, "playback": a.playback, "seconds": round(n / SR, 1), "load_s": round(load_s, 1),
        "hop_ms_median": round(statistics.median(hop_ms), 1), "hop_ms_p90": round(hop_ms[int(len(hop_ms) * 0.9)], 1),
        "hop_budget_ms": round(HOP_N / SR * 1000), "frame_s": FRAME_S,
        "peak_vram_mib": round(torch.cuda.max_memory_allocated() / 2**20), "events": events,
    }
    out = Path(__file__).resolve().parent / "results"
    out.mkdir(exist_ok=True)
    path = out / f"tyger-{Path(a.mic).stem}.json"
    path.write_text(json.dumps(result, indent=2))
    print(f"{len(events)} events; hops {result['hop_ms_median']} ms median, {result['hop_ms_p90']} ms p90 (budget {result['hop_budget_ms']}); peak VRAM {result['peak_vram_mib']} MiB → {path}")


if __name__ == "__main__":
    main()
    # The engine's native threads abort the interpreter's shutdown ("terminate called
    # without an active exception", status 134) once everything is written, so leave now.
    sys.stdout.flush()
    os._exit(0)
