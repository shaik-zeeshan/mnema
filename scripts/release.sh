#!/usr/bin/env bash
set -euo pipefail

# Bump the desktop app version everywhere, commit, tag, and push.
# Pushing the v<version> tag triggers .github/workflows/macos-release.yml.
#
# Usage:
#   scripts/release.sh patch|minor|major      # bump from current version
#   scripts/release.sh 0.2.0                   # set an explicit version
#   scripts/release.sh v0.2.0 --yes            # skip the confirmation prompt
#
# Run from a clean tree on the release branch (main).

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

release_branch="${RELEASE_BRANCH:-main}"
assume_yes=0
bump_spec=""

for arg in "$@"; do
  case "$arg" in
    -y|--yes) assume_yes=1 ;;
    -h|--help)
      sed -n '3,13p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    -*)
      echo "unknown flag: $arg" >&2
      exit 2
      ;;
    *)
      if [[ -n "$bump_spec" ]]; then
        echo "unexpected extra argument: $arg" >&2
        exit 2
      fi
      bump_spec="$arg"
      ;;
  esac
done

if [[ -z "$bump_spec" ]]; then
  echo "usage: $0 <patch|minor|major|X.Y.Z> [--yes]" >&2
  exit 2
fi

tauri_conf="$repo_root/apps/desktop/src-tauri/tauri.conf.json"
pkg_json="$repo_root/apps/desktop/package.json"
cargo_toml="$repo_root/apps/desktop/src-tauri/Cargo.toml"
bun_lock="$repo_root/bun.lock"
cargo_lock="$repo_root/Cargo.lock"

json_version() {
  bun --eval 'const fs = require("fs"); console.log(JSON.parse(fs.readFileSync(process.argv[1], "utf8")).version);' "$1"
}

# Replace the first literal occurrence of a string in a file (no regex).
replace_literal() {
  local file="$1" search="$2" replace="$3"
  bun --eval '
    const fs = require("fs");
    const file = process.argv[1];
    const search = process.argv[2];
    const replace = process.argv[3];
    const text = fs.readFileSync(file, "utf8");
    const idx = text.indexOf(search);
    if (idx === -1) {
      console.error(`could not find expected text in ${file}`);
      process.exit(1);
    }
    fs.writeFileSync(file, text.slice(0, idx) + replace + text.slice(idx + search.length));
  ' "$file" "$search" "$replace"
}

current_version="$(json_version "$tauri_conf")"
if [[ ! "$current_version" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
  echo "current version is not a plain X.Y.Z semver: $current_version" >&2
  exit 1
fi
cur_major="${BASH_REMATCH[1]}"
cur_minor="${BASH_REMATCH[2]}"
cur_patch="${BASH_REMATCH[3]}"

case "$bump_spec" in
  major) new_version="$((cur_major + 1)).0.0" ;;
  minor) new_version="${cur_major}.$((cur_minor + 1)).0" ;;
  patch) new_version="${cur_major}.${cur_minor}.$((cur_patch + 1))" ;;
  *)
    new_version="${bump_spec#v}"
    if [[ ! "$new_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
      echo "explicit version must look like 0.2.0 or v0.2.0; got: $bump_spec" >&2
      exit 2
    fi
    ;;
esac

tag="v$new_version"

if [[ "$new_version" == "$current_version" ]]; then
  echo "new version ($new_version) is the same as the current version" >&2
  exit 1
fi

# --- Pre-flight safety checks (before touching any files) ---

current_branch="$(git rev-parse --abbrev-ref HEAD)"
if [[ "$current_branch" != "$release_branch" ]]; then
  echo "on branch '$current_branch', expected '$release_branch'." >&2
  echo "set RELEASE_BRANCH=$current_branch to override." >&2
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "working tree is not clean; commit or stash changes before releasing." >&2
  exit 1
fi

if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "tag $tag already exists locally." >&2
  exit 1
fi

echo "Fetching origin/$release_branch ..."
git fetch --quiet origin "$release_branch"
if git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1; then
  echo "tag $tag already exists on origin." >&2
  exit 1
fi
behind="$(git rev-list --count "HEAD..origin/$release_branch")"
if [[ "$behind" -gt 0 ]]; then
  echo "local $release_branch is behind origin by $behind commit(s); pull first." >&2
  exit 1
fi

# --- Confirmation ---

echo
echo "  Release:  $current_version -> $new_version  (tag $tag)"
echo "  Branch:   $release_branch"
echo "  Effect:   commit + tag + push to origin, which triggers the macOS release build."
echo
if [[ "$assume_yes" -ne 1 ]]; then
  if [[ -t 0 ]]; then
    read -r -p "Proceed? [y/N] " reply
    case "$reply" in
      y|Y|yes|YES) ;;
      *) echo "aborted."; exit 1 ;;
    esac
  else
    echo "not a terminal; re-run with --yes to release non-interactively." >&2
    exit 1
  fi
fi

# --- Apply the version bump ---

echo "Updating version in source files ..."
replace_literal "$pkg_json"    "\"version\": \"$current_version\"" "\"version\": \"$new_version\""
replace_literal "$tauri_conf"  "\"version\": \"$current_version\"" "\"version\": \"$new_version\""
replace_literal "$cargo_toml"  $'name = "mnema"\nversion = "'"$current_version"'"' $'name = "mnema"\nversion = "'"$new_version"'"'
replace_literal "$bun_lock"    "\"version\": \"$current_version\"" "\"version\": \"$new_version\""
replace_literal "$cargo_lock"  $'name = "mnema"\nversion = "'"$current_version"'"' $'name = "mnema"\nversion = "'"$new_version"'"'

# Verify all sources agree, and let cargo reconcile Cargo.lock if needed.
echo "Verifying version consistency ..."
./scripts/verify-desktop-release-version.sh "$new_version"

# --- Commit, tag, push ---

echo "Committing, tagging, and pushing ..."
git add "$pkg_json" "$tauri_conf" "$cargo_toml" "$bun_lock" "$cargo_lock"
git commit -m "Bump app version to $new_version"
git tag -a "$tag" -m "Mnema $tag"
git push origin "$release_branch"
git push origin "$tag"

echo
echo "Released $tag. The macOS release workflow is now running:"
echo "  https://github.com/shaik-zeeshan/mnema/actions/workflows/macos-release.yml"
