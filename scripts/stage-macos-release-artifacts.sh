#!/usr/bin/env bash
set -euo pipefail

bundle_dir="${1:-target/aarch64-apple-darwin/release/bundle}"
output_dir="${2:-dist/macos-release}"

if [[ ! -d "$bundle_dir" ]]; then
  echo "bundle directory does not exist: $bundle_dir" >&2
  exit 1
fi

rm -rf "$output_dir"
mkdir -p "$output_dir"

artifacts=()
while IFS= read -r artifact; do
  artifacts+=("$artifact")
done < <(
  find "$bundle_dir" -type f \( \
    -name '*.dmg' \
    -o -name '*.zip' \
    -o -name '*.tar.gz' \
    -o -name '*.sig' \
    -o -name 'latest.json' \
  \) | sort
)

if [[ "${#artifacts[@]}" -eq 0 ]]; then
  echo "no release artifacts found under $bundle_dir" >&2
  exit 1
fi

for artifact in "${artifacts[@]}"; do
  cp "$artifact" "$output_dir/"
done

(
  cd "$output_dir"
  : > SHA256SUMS
  for artifact in *; do
    if [[ "$artifact" == "SHA256SUMS" ]]; then
      continue
    fi
    shasum -a 256 "$artifact" >> SHA256SUMS
  done
)

echo "staged ${#artifacts[@]} release artifact(s) in $output_dir"

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  echo "artifact_count=${#artifacts[@]}" >> "$GITHUB_OUTPUT"
fi
