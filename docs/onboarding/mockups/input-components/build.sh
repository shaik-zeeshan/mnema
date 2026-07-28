#!/usr/bin/env bash
# Concatenate the component fragments into one page, in onboarding flow order.
# ponytail: cat + a shell, not a bundler — the parts are already self-contained.
set -euo pipefail
cd "$(dirname "$0")"

ORDER=(sentence excluded switches providers models aisetup)

{
  cat _shell-head.html
  for slug in "${ORDER[@]}"; do
    if [[ -f "parts/$slug.part.html" ]]; then
      cat "parts/$slug.part.html"
      echo
    else
      echo "  <!-- missing: parts/$slug.part.html -->"
      echo "missing parts/$slug.part.html" >&2
    fi
  done
  cat _shell-foot.html
} > index.html

echo "wrote $(pwd)/index.html ($(wc -c < index.html | tr -d ' ') bytes)"
