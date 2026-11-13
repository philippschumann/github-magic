#!/bin/bash
# Registers open_pr_in_studio.py as a native messaging host for Chrome and Firefox (macOS).
# Re-run after moving this folder: the manifests store the script's absolute path.
set -euo pipefail

NAME=github_pr_quick_filters
HOST="$(cd "$(dirname "$0")" && pwd)/open_pr_in_studio.py"
CHROME_ID=nbiobejjfpehmbkkbddjhplmdeobhcjb # fixed by "key" in manifest.json
FIREFOX_ID=pr-quick-filters@local          # browser_specific_settings.gecko.id in manifest.json

chmod +x "$HOST"

write() { # dir, allowed-key, allowed-value
  mkdir -p "$1"
  cat > "$1/$NAME.json" <<EOF
{
  "name": "$NAME",
  "description": "Open GitHub PRs in an IDE or on a device",
  "path": "$HOST",
  "type": "stdio",
  "$2": ["$3"]
}
EOF
  echo "wrote $1/$NAME.json"
}

write "$HOME/Library/Application Support/Google/Chrome/NativeMessagingHosts" allowed_origins "chrome-extension://$CHROME_ID/"
write "$HOME/Library/Application Support/Mozilla/NativeMessagingHosts" allowed_extensions "$FIREFOX_ID"
