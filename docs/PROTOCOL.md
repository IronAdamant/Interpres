# Caption helper protocol

One UTF-8 line per message. No JSON crates in core.

```
READY
STATUS lc=running|stopped|degraded reason=...
PARTIAL text=...
FINAL text=...
ERROR message=...
LOG level=info|warn|error message=...
SHUTDOWN
```

Text after `text=` / `message=` runs to the end of the line, so plain spaces are fine.
Escape `%` as `%25` and line breaks as `%0A`; other `%XX` escapes (including UTF-8
bytes like `%C3%A9`) are decoded too. Lines are UTF-8.

Using this to plug in your own speech engine: see [ENGINES.md](ENGINES.md).
