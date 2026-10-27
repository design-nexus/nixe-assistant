"""Local Chatterbox Turbo worker. One JSON request/response per line on stdio."""

import contextlib
import json
import sys
from pathlib import Path

with contextlib.redirect_stdout(sys.stderr):
    from chatterbox.tts_turbo import ChatterboxTurboTTS
    model = ChatterboxTurboTTS.from_pretrained(device="cuda")

for line in sys.stdin:
    try:
        request = json.loads(line)
        reference = Path(request["reference"])
        output = Path(request["output"])
        if not reference.is_file() or reference.suffix.lower() != ".wav":
            raise ValueError("reference WAV is unavailable")
        if not output.parent.is_dir():
            raise ValueError("output directory is unavailable")
        with contextlib.redirect_stdout(sys.stderr):
            audio = model.generate(request["text"], audio_prompt_path=str(reference))
        audio.squeeze().detach().float().cpu().numpy().astype("<f4").tofile(output)
        response = {"ok": True, "sample_rate": model.sr}
    except Exception as error:
        response = {"ok": False, "error": str(error)}
    print(json.dumps(response), flush=True)
