#!/usr/bin/env bash
# Shows what the hardened runtime does to an ad-hoc signed app that asks for the microphone,
# with and without com.apple.security.device.audio-input. For a CI runner, not a working Mac:
# the entitled run shows the real system prompt and leaves a TCC entry behind.
# Usage: scripts/mic-probe/run.sh <out-dir>
set -euo pipefail
cd "$(dirname "$0")"
out=$(mkdir -p "$1" && cd "$1" && pwd)
swiftc -O probe.swift -o "$out/probe" -framework AVFoundation

for variant in plain entitled; do
  app="$out/$variant/MicProbe.app"
  mkdir -p "$app/Contents/MacOS"
  cp "$out/probe" "$app/Contents/MacOS/MicProbe"
  cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.pinned-ci.mic-probe.$variant.$RANDOM</string>
<key>CFBundleExecutable</key><string>MicProbe</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>NSMicrophoneUsageDescription</key><string>CI probe for pinned's microphone entitlement.</string>
</dict></plist>
PLIST
  args=(--force --sign - --options runtime)
  [ "$variant" = entitled ] && args+=(--entitlements ../../Entitlements.plist)
  codesign "${args[@]}" "$app"
  echo "== $variant"
  codesign -dv "$app" 2>&1 | grep -E '^(Identifier|CodeDirectory)'
  codesign -d --entitlements - "$app" 2>&1 | grep -c audio-input | sed 's/^/audio-input entitlements: /' || true
  since=$(date '+%Y-%m-%d %H:%M:%S')
  open -n "$app" --args "$out/$variant.txt"
  sleep 6
  screencapture -x "$out/$variant.png" || echo "screencapture failed"
  for _ in $(seq 1 30); do [ -s "$out/$variant.txt" ] && break; sleep 1; done
  cat "$out/$variant.txt" 2>/dev/null || echo "no answer yet"
  log show --start "$since" --style compact --predicate 'subsystem == "com.apple.TCC" AND eventMessage CONTAINS "kTCCServiceMicrophone"' 2>/dev/null \
    | grep -oE "(Prompting policy for hardened runtime; service: kTCCServiceMicrophone requires entitlement [a-z.-]+ but it is missing|Policy disallows prompt|AUTHREQ_PROMPTING: msgID=[0-9.]+, service=kTCCServiceMicrophone)" \
    | sort | uniq -c | tee "$out/$variant.tcc.txt"
  pkill -f "$variant/MicProbe.app" || true
done
