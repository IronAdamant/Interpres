#!/bin/bash
# Optional helper loop: emit STATUS based on Live Captions process.
# Full AX scrape is in the Interpres binary (in-process). This script is a
# fallback process-only signal source for external host mode.
set -euo pipefail
echo "READY"
while true; do
  if pgrep -f "Live Captions" >/dev/null 2>&1; then
    echo "STATUS lc=running reason=process"
  else
    echo "STATUS lc=stopped reason=process"
  fi
  sleep 1
done
