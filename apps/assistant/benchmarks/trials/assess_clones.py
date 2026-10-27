#!/usr/bin/env python3
"""Check generated clone samples for intelligibility with installed local Parakeet."""

import json
import re
import subprocess
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
RESULTS = HERE / "results"
BACKENDS = ["chatterbox-turbo", "qwen-clone-0.6B-Base", "qwen-clone-1.7B-Base"]
STT = "/usr/lib/voxtype/voxtype-onnx-avx2"


def words(text):
    text = text.lower().replace("52", "fifty two")
    return re.findall(r"[a-z]+", text)


def edits(a, b):
    row = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        next_row = [i]
        for j, y in enumerate(b, 1):
            next_row.append(min(next_row[-1] + 1, row[j] + 1, row[j - 1] + (x != y)))
        row = next_row
    return row[-1]


rows = []
for backend in BACKENDS:
    profile = json.loads((RESULTS / f"{backend}-shared-ref.json").read_text())
    for key, phrase in profile["phrases"].items():
        source = RESULTS / f"{backend}-shared-ref-{key}.wav"
        with tempfile.TemporaryDirectory() as temp:
            wav = Path(temp) / "sample.wav"
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(source), "-ar", "16000", "-ac", "1", str(wav)], check=True)
            result = subprocess.run([STT, "transcribe", "--engine", "parakeet", str(wav)], check=True, capture_output=True, text=True)
        heard = result.stdout.strip().splitlines()[-1]
        expected = words(phrase["text"])
        actual = words(heard)
        row = {"backend": backend, "phrase": key, "transcription": heard, "reference_words": len(expected), "word_errors": edits(expected, actual)}
        rows.append(row)
        print(f"{backend:24} {key:8} {row['word_errors']}/{row['reference_words']} word errors: {heard}", flush=True)

(RESULTS / "clone-intelligibility-shared-ref.json").write_text(json.dumps(rows, indent=2) + "\n")
