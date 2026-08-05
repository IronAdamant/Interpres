# Live Captions detection signals (L0)

Update these when an OS update breaks capture. Values live in `src/platform/signals.rs`.

## Windows

| Signal | Value |
|--------|--------|
| Process | `LiveCaptions` / `LiveCaptions.exe` |
| Window class | `LiveCaptionsDesktopWindow` |
| Text AutomationId | `CaptionsTextBlock` (fallback `CaptionsScrollViewer`) |
| Ignore | `ReadyToCaptionTextBlock` |

Helper: `helpers/windows/Get-LiveCaptionsText.ps1`

## macOS

| Signal | Value |
|--------|--------|
| Bundle ID | `com.apple.accessibility.LiveTranscriptionAgent` |
| Process | `Live Captions` |
| Permission | Accessibility for Interpres |

Run `interpres probe` after OS updates.
