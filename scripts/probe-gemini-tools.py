#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["websockets>=15,<17"]
# ///
"""Exercise Gemini's real tool transport without GameBuddy or saved game data.

Uses the GameBuddy Gemini key or GEMINI_API_KEY. This makes billable API calls.
Only the local diagnostic_echo function can run; it has no side effects.
"""

import argparse
import asyncio
import base64
import contextlib
import json
import os
from pathlib import Path
import re
import secrets
import ssl
import time

from websockets.asyncio.client import connect


async def probe(args, key, model):
    thinking = model.endswith("extended-thinking")
    generation = {
        "responseModalities": ["AUDIO"],
        "speechConfig": {"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Puck"}}},
    }
    if thinking:
        generation["thinkingConfig"] = {"thinkingLevel": args.thinking.upper()}
    setup = {
        "model": "models/" + model,
        "generationConfig": generation,
        "outputAudioTranscription": {},
        "tools": [{"functionDeclarations": [{
            "name": "diagnostic_echo",
            "description": "Return a fresh diagnostic verification code. No game data is read or changed.",
            "behavior": "NON_BLOCKING" if thinking else "BLOCKING",
            "parameters": {
                "type": "OBJECT",
                "properties": {"nonce": {"type": "STRING"}},
                "required": ["nonce"],
            },
        }]}],
    }
    if args.audio:
        setup["inputAudioTranscription"] = {}
        setup["realtimeInputConfig"] = {"automaticActivityDetection": {"disabled": True}}
    result = {
        "model": model, "api_version": args.api_version, "setup_accepted": False,
        "tool_calls": [], "transcript": "", "input_transcript": "",
        "audio_packets": 0, "idle": False, "verified": False,
    }
    code = str(secrets.randbelow(8000) + 1000)
    transcript = ""
    started = time.monotonic()
    sender = None
    cafile = os.environ.get("SSL_CERT_FILE")
    if not cafile and Path("/etc/ssl/certs/ca-certificates.crt").exists():
        cafile = "/etc/ssl/certs/ca-certificates.crt"
    try:
        url = ("wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage."
               + args.api_version + ".GenerativeService.BidiGenerateContent")
        async with connect(
            url, additional_headers={"x-goog-api-key": key},
            ssl=ssl.create_default_context(cafile=cafile), open_timeout=15,
            close_timeout=3, max_size=10_000_000,
        ) as ws:
            await ws.send(json.dumps({"setup": setup}))
            first = json.loads(await asyncio.wait_for(ws.recv(), 15))
            if "setupComplete" not in first:
                raise RuntimeError("Setup not acknowledged: " + json.dumps(first.get("error", list(first))))
            result["setup_accepted"] = True
            if args.audio:
                async def stream():
                    await ws.send(json.dumps({"realtimeInput": {"activityStart": {}}}))
                    pcm = args.audio.read_bytes()
                    for offset in range(0, len(pcm), 3200):
                        await ws.send(json.dumps({"realtimeInput": {"audio": {
                            "mimeType": "audio/pcm;rate=16000",
                            "data": base64.b64encode(pcm[offset:offset + 3200]).decode(),
                        }}}))
                        await asyncio.sleep(.1)
                    await ws.send(json.dumps({"realtimeInput": {"activityEnd": {}}}))
                    await ws.send(json.dumps({"realtimeInput": {"audioStreamEnd": True}}))
                sender = asyncio.create_task(stream())
            else:
                await ws.send(json.dumps({"clientContent": {
                    "turns": [{"role": "user", "parts": [{"text":
                        "Call diagnostic_echo with nonce 237 now. Then say the verification_code "
                        "returned by the tool. Do not guess it or reuse a previous result."
                    }]}], "turnComplete": True,
                }}))
            async with asyncio.timeout(args.timeout):
                while True:
                    message = json.loads(await ws.recv())
                    if message.get("error"):
                        raise RuntimeError(json.dumps(message["error"]))
                    content = message.get("serverContent", {})
                    status = message.get("interactionStatus") or content.get("interactionStatus")
                    if status:
                        result["last_status"] = status
                    transcript += content.get("outputTranscription", {}).get("text", "")
                    result["input_transcript"] += content.get("inputTranscription", {}).get("text", "")
                    result["audio_packets"] += sum(
                        bool(part.get("inlineData", {}).get("data"))
                        for part in content.get("modelTurn", {}).get("parts", [])
                    )
                    for call in message.get("toolCall", {}).get("functionCalls", []):
                        valid = call.get("name") == "diagnostic_echo" and call.get("args", {}).get("nonce") == "237"
                        result["tool_calls"].append({
                            "name": call.get("name"), "valid": valid,
                            "at_s": round(time.monotonic() - started, 3),
                        })
                        response = {"output": {"verification_code": code}} if valid else {"error": "Only diagnostic_echo with nonce 237 is available."}
                        await ws.send(json.dumps({"toolResponse": {"functionResponses": [{
                            "id": call["id"], "name": call["name"], "response": response,
                        }]}}))
                    if status == "IDLE" or (not thinking and content.get("turnComplete")):
                        result["idle"] = True
                        break
    except TimeoutError:
        result["error"] = "Timed out waiting for the complete interaction"
    except Exception as error:
        result["error"] = (type(error).__name__ + ": " + str(error)).replace(key, "[REDACTED]")[:1000]
    finally:
        if sender:
            sender.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                try:
                    await sender
                except Exception as error:
                    result["error"] = ("Audio input: " + str(error)).replace(key, "[REDACTED]")[:1000]
    result["transcript"] = transcript.replace(key, "[REDACTED]")[:1000]
    result["reported_system_error"] = bool(re.search(r"\b(?:system|internal)\s+error\b", transcript, re.I))
    result["elapsed_s"] = round(time.monotonic() - started, 3)
    result["verified"] = (
        result["idle"] and not result.get("error") and not result["reported_system_error"]
        and any(call["valid"] for call in result["tool_calls"])
        and code in "".join(c for c in transcript if c.isdigit())
    )
    return result


async def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compare", action="store_true", help="Run standard Live, then Extended Thinking")
    parser.add_argument("--thinking", choices=["low", "medium", "high"], default="high")
    parser.add_argument("--api-version", choices=["v1alpha", "v1beta"], default="v1beta")
    parser.add_argument("--timeout", type=float, default=90)
    parser.add_argument("--audio", type=Path, help="16 kHz mono s16le PCM asking diagnostic_echo for nonce 237")
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    if args.audio and (not args.audio.is_file() or not args.audio.stat().st_size or args.audio.stat().st_size % 2):
        parser.error("--audio must contain nonempty 16-bit PCM samples")
    key = os.environ.get("GEMINI_API_KEY", "").strip()
    if not key:
        keyfile = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config") / "gamebuddy/gemini-api-key"
        if not keyfile.is_file():
            parser.error("Configure a GameBuddy Gemini API key or set GEMINI_API_KEY")
        key = keyfile.read_text().strip()
    if not key:
        parser.error("The Gemini API key is empty")
    models = ["gemini-3.8-live"] if args.compare else []
    models.append("gemini-3.8-live-extended-thinking")
    passed = True
    for model in models:
        result = await probe(args, key, model)
        print(json.dumps(result).replace(key, "[REDACTED]"), flush=True)
        passed = passed and result["verified"]
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
