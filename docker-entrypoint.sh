#!/bin/sh
set -e

# Update yt-dlp to the latest version at container startup if network is available
if command -v uv >/dev/null 2>&1; then
    uv tool upgrade yt-dlp 2>/dev/null || true
fi

exec "$@"
