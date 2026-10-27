#!/usr/bin/env python3
"""Persistent CPU speech synthesis. JSON lines in/out; no microphone access.

The model argument is a Piper .onnx voice, or a .kokoro file whose first line
names a Kokoro voice (for example am_michael): Kokoro sounds far more natural.
"""
import base64
import io
import json
import os
import sys
import wave

os.environ.setdefault("OMP_NUM_THREADS", "2")
os.environ.setdefault("OPENBLAS_NUM_THREADS", "2")

model = sys.argv[1]

if model.endswith(".kokoro"):
    import numpy
    import torch
    from kokoro import KPipeline

    # Keep synthesis from occupying all game/render threads.
    torch.set_num_threads(int(os.environ.get("OMP_NUM_THREADS", "2")))
    with open(model, encoding="utf-8") as voice_file:
        voice_name = voice_file.readline().strip() or "am_michael"
    pipeline = KPipeline(lang_code=voice_name[0], repo_id="hexgrad/Kokoro-82M")
    # Load the voice and warm the model so the first reply is not slow.
    for _ in pipeline("Ready.", voice=voice_name):
        pass

    def synthesize(text, wav):
        pieces = [audio.numpy() if hasattr(audio, "numpy") else numpy.asarray(audio)
                  for _, _, audio in pipeline(text, voice=voice_name) if audio is not None]
        samples = numpy.concatenate(pieces) if pieces else numpy.zeros(1, dtype=numpy.float32)
        pcm = (numpy.clip(samples, -1.0, 1.0) * 32767).astype("<i2")
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(24000)
        wav.writeframes(pcm.tobytes())
else:
    from piper import PiperVoice
    from piper.config import PiperConfig
    import onnxruntime

    # Keep synthesis from occupying all game/render threads.
    options = onnxruntime.SessionOptions()
    options.intra_op_num_threads = 2
    options.inter_op_num_threads = 1
    with open(model + ".json", encoding="utf-8") as config_file:
        config = PiperConfig.from_dict(json.load(config_file))
    voice = PiperVoice(session=onnxruntime.InferenceSession(model, sess_options=options, providers=["CPUExecutionProvider"]), config=config)

    def synthesize(text, wav):
        voice.synthesize_wav(text, wav)

print(json.dumps({"ready": True}), flush=True)
for line in sys.stdin:
    try:
        text = json.loads(line)["text"]
        if not isinstance(text, str) or not text.strip() or len(text) > 4000:
            raise ValueError("Speech must contain between 1 and 4000 characters")
        output = io.BytesIO()
        with wave.open(output, "wb") as wav:
            synthesize(text, wav)
        print(json.dumps({"wav": base64.b64encode(output.getvalue()).decode("ascii")}), flush=True)
    except Exception as error:
        print(json.dumps({"error": str(error)}), flush=True)
