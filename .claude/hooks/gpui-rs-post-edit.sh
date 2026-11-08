#!/usr/bin/env bash
# PostToolUse(Edit|Write): rustfmt the one touched GPUI/app-core file, then enforce the 800-line cap.
f=$(jq -r '.tool_input.file_path // empty')
case "$f" in
  */apps/desktop-gpui/*.rs|*/crates/app-core/*.rs) ;;
  *) exit 0 ;;
esac
[ -f "$f" ] || exit 0
rustfmt --edition 2021 "$f" 2>&1 >/dev/null || { echo "rustfmt failed on $f" >&2; exit 2; }
n=$(wc -l < "$f" | tr -d ' ')
if [ "$n" -gt 800 ]; then
  echo "$f has $n lines; the cap is 800. Split it by responsibility." >&2
  exit 2
fi
