#!/bin/sh
# Renders the link preview from assets/social.html and the README banners from
# assets/src/banner.html with headless Chrome. Run from the repository root.
set -eu
chrome=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
page="file://$PWD/assets/src/banner.html"
shot() {
  "$chrome" --headless=new --disable-gpu --hide-scrollbars --virtual-time-budget=8000 \
    --window-size="$2" --force-device-scale-factor="$3" --screenshot="$4" "$5$1" >/dev/null 2>&1
}
shot "" 1200,630 1 assets/social.png "file://$PWD/assets/social.html"
shot "?v=hero" 1600,700 2 assets/hero.png "$page"
shot "?v=hero&lang=ko" 1600,700 2 assets/hero.ko.png "$page"
