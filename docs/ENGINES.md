# Bring your own speech engine

By default Interpres saves what **Live Captions** shows (Windows or macOS). That needs no setup.

If you run your own speech-to-text model (Phonon-2, Parakeet, Whisper, …), Interpres can save its output instead. Interpres does **not** include or download any model. You run the engine as your own program, and Interpres:

- starts it when you press **Start recording**
- reads caption lines from its standard output
- saves each finished line to the transcript, with the same banner, idle prompt, Open/Copy buttons and debug log as Live Captions
- restarts it if it exits (after 5 s, noted in the transcript)
- asks it to stop when you press **Stop recording**, and saves anything it prints while finishing

`interpres.exe` stays zero-dependency. Whatever your engine needs (Python, ONNX Runtime, MLX, CUDA, …) lives in your engine, not in Interpres.

---

## 1. Point Interpres at your engine

Open **Settings ▾ → Edit settings file…** on Windows, or edit the file directly:

- Windows: `%APPDATA%\Interpres\settings.conf`
- macOS: `~/.config/interpres/settings.conf`

```ini
source=engine
helper_path=C:\Users\you\AppData\Local\Programs\Python\Python313\python.exe
helper_args=-u "C:\engines\my_engine.py" --model "C:\models\phonon-2"
```

| Key | Meaning |
|-----|---------|
| `source` | `engine` to use your engine, `os` for Live Captions (default) |
| `helper_path` | The program to run, as a full path |
| `helper_args` | Its arguments. Wrap paths with spaces in double quotes |

Then choose **Settings ▾ → Captions from: external engine** (or set `source=engine` yourself) and press **Start recording**. Changes apply the next time you start recording.

The working directory is not changed, so use **absolute paths** for scripts and models.

---

## 2. What your engine must do

1. **Capture audio itself.** Interpres doesn’t give your engine any audio. For meetings you usually want both the other people (system output: WASAPI loopback on Windows, ScreenCaptureKit on macOS) and your microphone.
2. **Print one line per event to stdout, in UTF-8, and flush after every line.** Run Python with `-u`, or call `flush()`.
3. **Print `READY`** once the model is loaded. Until then the banner says *Starting…*.
4. **Print `FINAL text=…`** for each finished utterance. Each FINAL becomes one transcript line, exactly as sent.
5. Optionally print **`PARTIAL text=…`** while someone is still talking. It fills the live line at the bottom of the transcript and is never saved.
6. **Stop** when a line `SHUTDOWN` arrives on stdin, or when stdin closes. You have **2 seconds** to print FINAL lines for audio you still hold before Interpres ends the process.

Anything your engine writes to **stderr** goes into the Interpres debug log (turn on **Settings ▾ → Write debug log**), prefixed `engine stderr:`.

### Lines

```text
READY
PARTIAL text=we should probably
FINAL text=We should probably move the launch to Thursday.
ERROR message=Microphone not found
LOG level=info message=model loaded in 2.1 s
STATUS lc=stopped reason=audio device lost
```

| Line | Effect in Interpres |
|------|---------------------|
| `READY` | Banner: *Ready — waiting for speech* |
| `PARTIAL text=…` | Live line (not saved) |
| `FINAL text=…` | Saved transcript line; banner: *Recording* |
| `ERROR message=…` | Shown under the banner and logged |
| `LOG level=… message=…` | Debug log only |
| `STATUS lc=stopped` / `lc=degraded` | Banner: *Not capturing — caption engine stopped* |
| `STATUS lc=running` | Banner: *Ready* (if nothing else has happened yet) |

Plain text after `text=` is fine: spaces and any language work as-is. Escape only `%` as `%25` and line breaks as `%0A`. Percent-encoded UTF-8 such as `caf%C3%A9` is also accepted. The full grammar is in [PROTOCOL.md](PROTOCOL.md).

---

## 3. Start from the example

[`examples/engines/example_engine.py`](../examples/engines/example_engine.py) uses only the Python standard library. It speaks the protocol correctly and prints demo sentences, so you can check the wiring before adding a model. Replace its `utterances()` function with your model:

```python
def utterances():
    # capture audio → split at pauses (VAD) → transcribe each chunk
    for chunk in audio_chunks_split_at_pauses():
        yield True, model.transcribe(chunk)
```

Offline models such as Parakeet and Phonon-2 transcribe a whole chunk at once. Split at pauses and send one `FINAL` per chunk: each line then appears about a second after the speaker pauses, with no draft rewrites.

---

## Troubleshooting

| Symptom | Check |
|---------|-------|
| Banner stays on *Starting…* | Your engine never printed `READY`, or didn’t flush stdout (Python: `-u`) |
| *Could not start …* | `helper_path` is wrong or not a full path |
| Engine keeps restarting | It exits on its own. Turn on the debug log and read the `engine stderr:` lines |
| Accents look wrong | Your engine isn’t writing UTF-8. In Python call `sys.stdout.reconfigure(encoding="utf-8")` |
| Last sentence missing after Stop | Print its `FINAL` within 2 s of receiving `SHUTDOWN` |
