# Live Captions detection signals (L0)

Update these when an OS update breaks capture. Values live in `src/platform/signals.rs`.

## Windows

| Signal | Value |
|--------|--------|
| Process | `LiveCaptions` / `LiveCaptions.exe` |
| Window class | `LiveCaptionsDesktopWindow` |
| Text AutomationId | `CaptionsTextBlock` (fallback `CaptionsScrollViewer`) |
| Ignore | `ReadyToCaptionTextBlock` |

Reader: in-process UI Automation (`src/platform/windows_uia.rs`), IUIAutomation2 timeouts 1.5 s connect / 2.5 s per call.

## macOS

| Signal | Value |
|--------|--------|
| Bundle ID | `com.apple.accessibility.LiveTranscriptionAgent` |
| Process | `Live Captions` |
| Permission | Accessibility for Interpres |

Run `interpres probe` after OS updates.
