#!/usr/bin/env python3
"""Example Interpres caption engine — Python standard library only.

Interpres starts this program, reads caption lines from its stdout and saves them.
It prints demo sentences so you can check the wiring; replace `utterances()` with
your own model (Phonon-2, Parakeet, Whisper, ...). Protocol: docs/ENGINES.md.

settings.conf (Windows):
    source=engine
    helper_path=C:\\Users\\you\\AppData\\Local\\Programs\\Python\\Python313\\python.exe
    helper_args=-u "C:\\path\\to\\example_engine.py"

settings.conf (macOS):
    source=engine
    helper_path=/usr/bin/python3
    helper_args=-u "/path/to/example_engine.py"
"""

import itertools
import sys
import threading
import time

# Interpres reads UTF-8; Windows pipes default to the ANSI code page otherwise.
sys.stdout.reconfigure(encoding="utf-8")

stop = threading.Event()


def send(kind: str, text: str = "") -> None:
    """Print one protocol line. Only `%` and line breaks need escaping."""
    text = text.replace("%", "%25").replace("\r", " ").replace("\n", " ").strip()
    field = "message" if kind == "ERROR" else "text"
    sys.stdout.write(f"{kind} {field}={text}\n" if text else f"{kind}\n")
    sys.stdout.flush()


def watch_stdin() -> None:
    """Interpres writes SHUTDOWN (then closes stdin) when the user presses Stop."""
    for line in sys.stdin:
        if line.strip().upper() == "SHUTDOWN":
            break
    stop.set()  # SHUTDOWN or EOF: finish up


def utterances():
    """Yield (is_final, text). Replace this with your speech engine.

    A real engine would: capture audio (system output + microphone), split it at
    pauses (VAD), transcribe each chunk, and yield (True, sentence) per chunk.
    Yielding (False, words-so-far) first is optional — it fills the live line.
    """
    demo = [
        "This is the example engine talking to Interpres.",
        "Replace the utterances function with your own speech model.",
        "Each final line is saved to the transcript exactly as you send it.",
    ]
    for sentence in itertools.cycle(demo):
        words = sentence.split()
        for i in range(2, len(words), 2):
            yield False, " ".join(words[:i])
            time.sleep(0.3)
        yield True, sentence
        time.sleep(2.0)


def main() -> int:
    threading.Thread(target=watch_stdin, daemon=True).start()
    # Load your model here, then say READY (the banner shows "Starting…" until then).
    send("READY")
    for is_final, text in utterances():
        if stop.is_set():
            break
        send("FINAL" if is_final else "PARTIAL", text)
    # A real engine would transcribe any audio still buffered and send it as FINAL here.
    return 0


if __name__ == "__main__":
    sys.exit(main())
