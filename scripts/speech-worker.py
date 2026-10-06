#!/usr/bin/env python3
"""Persistent CPU speech synthesis. JSON lines in/out; no microphone access."""
import base64
import io
import json
import os
import sys
import wave

os.environ.setdefault("OMP_NUM_THREADS", "2")
os.environ.setdefault("OPENBLAS_NUM_THREADS", "2")
from piper import PiperVoice
from piper.config import PiperConfig
import onnxruntime

model = sys.argv[1]
# Keep synthesis from occupying all game/render threads.
options = onnxruntime.SessionOptions()
options.intra_op_num_threads = 2
options.inter_op_num_threads = 1
with open(model + ".json", encoding="utf-8") as config_file:
    config = PiperConfig.from_dict(json.load(config_file))
voice = PiperVoice(session=onnxruntime.InferenceSession(model, sess_options=options, providers=["CPUExecutionProvider"]), config=config)
print(json.dumps({"ready": True}), flush=True)
for line in sys.stdin:
    try:
        text = json.loads(line)["text"]
        if not isinstance(text, str) or not text.strip() or len(text) > 4000:
            raise ValueError("Speech must contain between 1 and 4000 characters")
        output = io.BytesIO()
        with wave.open(output, "wb") as wav:
            voice.synthesize_wav(text, wav)
        print(json.dumps({"wav": base64.b64encode(output.getvalue()).decode("ascii")}), flush=True)
    except Exception as error:
        print(json.dumps({"error": str(error)}), flush=True)
