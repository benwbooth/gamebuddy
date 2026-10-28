#!/usr/bin/env python3
"""Persistent speech synthesis. JSON lines in/out; no microphone access.

The model argument is a Piper .onnx voice, a .orpheus file naming an Orpheus
voice (spoken by GameBuddy's own llama-server, decoded with SNAC), a .omni file naming an LFM3-Omni
voice (male or female, spoken by GameBuddy's own llama-server), or a .kokoro file whose first line
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

if model.endswith(".omni"):
    # LFM3-Omni through GameBuddy's own llama-server (`/tts`): the file
    # names the voice: male, female, or ref:<clip.wav> to copy a voice.
    import time
    import urllib.request

    with open(model, encoding="utf-8") as voice_file:
        voice_name = voice_file.readline().strip() or "male"
    # "ref:<file.wav>" (relative to the .omni file) makes Omni speak in the
    # voice of that short reference clip instead of its built-in voices.
    speaker_ref = None
    if voice_name.startswith("ref:"):
        import base64 as _b64
        reference = os.path.join(os.path.dirname(os.path.abspath(model)), voice_name[4:].strip())
        with open(reference, "rb") as clip:
            speaker_ref = _b64.b64encode(clip.read()).decode("ascii")
    base = "http://127.0.0.1:11441"
    for _ in range(72):
        try:
            with urllib.request.urlopen(base + "/health", timeout=2) as reply:
                if reply.status == 200:
                    break
        except Exception:
            pass
        time.sleep(0.25)

    def synthesize(text, wav):
        request = urllib.request.Request(
            base + "/tts",
            data=json.dumps(
                {"input": text, "lang": "en", "response_format": "wav", "speaker_ref_b64": speaker_ref}
                if speaker_ref else
                {"input": text, "voice": voice_name, "lang": "en", "response_format": "wav"}
            ).encode(),
            headers={"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(request, timeout=60) as reply:
            data = reply.read()
        with wave.open(io.BytesIO(data), "rb") as source:
            wav.setnchannels(source.getnchannels())
            wav.setsampwidth(source.getsampwidth())
            wav.setframerate(source.getframerate())
            wav.writeframes(source.readframes(source.getnframes()))
elif model.endswith(".orpheus"):
    # Orpheus 3B through GameBuddy's own llama-server (port 11443): the model
    # writes SNAC audio codes as tokens, decoded here on the GPU. The file
    # names the voice (leo, zac, tara, ...).
    import re
    import time
    import urllib.request

    import numpy
    import torch
    from snac import SNAC

    with open(model, encoding="utf-8") as voice_file:
        voice_name = voice_file.readline().strip() or "leo"
    base = "http://127.0.0.1:11443"
    for _ in range(240):
        try:
            with urllib.request.urlopen(base + "/health", timeout=2) as reply:
                if reply.status == 200:
                    break
        except Exception:
            pass
        time.sleep(0.25)
    device = "cuda" if torch.cuda.is_available() else "cpu"
    snac = SNAC.from_pretrained("hubertsiuzdak/snac_24khz").eval().to(device)

    def audio_codes(text):
        request = urllib.request.Request(
            base + "/v1/completions",
            data=json.dumps({"prompt": f"<|audio|>{voice_name}: {text}<|eot_id|>", "max_tokens": 2400,
                             "temperature": 0.6, "top_p": 0.9, "repeat_penalty": 1.1}).encode(),
            headers={"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(request, timeout=120) as reply:
            written = json.load(reply)["choices"][0]["text"]
        codes = []
        for number in re.findall(r"<custom_token_(\d+)>", written):
            code = int(number) - 10 - ((len(codes) % 7) * 4096)
            if 0 < code <= 4096:
                codes.append(code)
        return codes

    def decode(window):
        layers = ([], [], [])
        for j in range(len(window) // 7):
            f = window[7 * j:7 * j + 7]
            layers[0].append(f[0])
            layers[1].extend([f[1], f[4]])
            layers[2].extend([f[2], f[3], f[5], f[6]])
        return snac.decode([torch.tensor(l, device=device).unsqueeze(0) for l in layers])

    def synthesize(text, wav):
        codes = audio_codes(text)
        pieces = []
        # Seven codes are one frame. Decoding four frames at a time and
        # keeping the middle keeps one tensor shape, so the GPU kernels are
        # built once (decoding whole sentences rebuilt them for every new
        # length, taking seconds each).
        with torch.inference_mode():
            for end in range(28, len(codes) + 1, 7):
                pieces.append(decode(codes[end - 28:end])[0, 0, 2048:4096].float().cpu().numpy())
        samples = numpy.concatenate(pieces) if pieces else numpy.zeros(1, dtype=numpy.float32)
        pcm = (numpy.clip(samples, -1.0, 1.0) * 32767).astype("<i2")
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(24000)
        wav.writeframes(pcm.tobytes())

    # Build the kernels before the first reply.
    with torch.inference_mode():
        decode([1] * 28)
elif model.endswith(".kokoro"):
    import numpy
    import torch
    from kokoro import KPipeline

    # Keep synthesis from occupying all game/render threads.
    torch.set_num_threads(int(os.environ.get("OMP_NUM_THREADS", "2")))
    with open(model, encoding="utf-8") as voice_file:
        voice_name = voice_file.readline().strip() or "am_michael"
        # Optional second line: speaking speed (below 1 is slower and clearer).
        try:
            speed = float(voice_file.readline().strip() or "1")
        except ValueError:
            speed = 1.0
    pipeline = KPipeline(lang_code=voice_name[0], repo_id="hexgrad/Kokoro-82M")
    # Load the voice and warm the model so the first reply is not slow.
    for _ in pipeline("Ready.", voice=voice_name):
        pass

    def synthesize(text, wav):
        pieces = [audio.numpy() if hasattr(audio, "numpy") else numpy.asarray(audio)
                  for _, _, audio in pipeline(text, voice=voice_name, speed=speed) if audio is not None]
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
