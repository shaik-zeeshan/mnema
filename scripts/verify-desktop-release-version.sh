#!/usr/bin/env bash
set -euo pipefail

requested_version="${1:-}"
if [[ -z "$requested_version" ]]; then
  echo "usage: $0 <version>" >&2
  exit 2
fi

version="${requested_version#v}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "release version must look like 0.1.0 or v0.1.0; got: $requested_version" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

json_version() {
  local path="$1"
  bun --eval 'const fs = require("fs"); const path = process.argv[1]; console.log(JSON.parse(fs.readFileSync(path, "utf8")).version);' "$path"
}

tauri_version="$(json_version "$repo_root/apps/desktop/src-tauri/tauri.conf.json")"
package_version="$(json_version "$repo_root/apps/desktop/package.json")"
cargo_version="$(
  cargo metadata \
    --no-deps \
    --format-version 1 \
    --manifest-path "$repo_root/apps/desktop/src-tauri/Cargo.toml" \
    | bun --eval 'const fs = require("fs"); const metadata = JSON.parse(fs.readFileSync(0, "utf8")); const pkg = metadata.packages.find((entry) => entry.name === "mnema"); if (!pkg) throw new Error("mnema package not found in Cargo metadata"); console.log(pkg.version);'
)"

if [[ "$tauri_version" != "$version" ]]; then
  echo "apps/desktop/src-tauri/tauri.conf.json version is $tauri_version, expected $version" >&2
  exit 1
fi

if [[ "$package_version" != "$version" ]]; then
  echo "apps/desktop/package.json version is $package_version, expected $version" >&2
  exit 1
fi

if [[ "$cargo_version" != "$version" ]]; then
  echo "apps/desktop/src-tauri/Cargo.toml version is $cargo_version, expected $version" >&2
  exit 1
fi

tag="v$version"
echo "release version verified: $version ($tag)"

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  {
    echo "version=$version"
    echo "tag=$tag"
  } >> "$GITHUB_OUTPUT"
fi
