#!/bin/sh
# What a change touches, from its files: the groups of the verdict to run and how critical it is.
# Read by `validate.sh --changed` (the development loop) and `--pre-merge` (the pull request's CI).
#
#   sh scripts/scope.sh --crates [<base>]
#                                   the packages of crates/ the change touches (their Cargo names)
#   sh scripts/scope.sh [<base>]    prints two lines:
#     groups: rust ui firmware      the thirds of the verdict the change reaches (may be empty)
#     level: docs|normal|critical   docs: only docs and specs; critical: a path where a fault
#                                   reaches the box (firmware, memory, update, radio, storage,
#                                   recipe installs and scripts, dependencies, CI, release); else normal
#
# The files are the commits since <base> (default: origin/main) and the uncommitted ones. An
# unreadable base counts every group, critical: when in doubt, everything runs.
set -e
cd "$(dirname "$0")/.."

crates_only=false
[ "${1:-}" = --crates ] && { crates_only=true; shift; }
base="${1:-origin/main}"
if ! merge=$(git merge-base "$base" HEAD 2>/dev/null); then
  if $crates_only; then
    echo "--workspace"
  else
    echo "groups: rust ui firmware"
    echo "level: critical"
  fi
  exit 0
fi
files=$({
  git diff --name-only "$merge" HEAD
  git diff --name-only HEAD
  git ls-files --others --exclude-standard
} | sort -u)

if $crates_only; then
  for dir in $(printf '%s\n' $files | sed -n 's|^crates/\([^/]*\)/.*|\1|p' | sort -u); do
    [ -f "crates/$dir/Cargo.toml" ] && sed -n 's/^name *= *"\(.*\)"/\1/p' "crates/$dir/Cargo.toml" | head -n 1
  done | tr '\n' ' '
  echo
  exit 0
fi

rust=false
ui=false
firmware=false
critical=false
code=false
for f in $files; do
  case "$f" in
    docs/sowel-subset.md) code=true ;;
    docs/* | specs/* | inputs/* | *.md) ;;
    *) code=true ;;
  esac
  case "$f" in
    crates/core/* | crates/recipe-sdk/*) rust=true; firmware=true ;;
    crates/*) rust=true ;;
    firmware/*) firmware=true ;;
    recipes/*) rust=true; firmware=true ;;
    ui/*) ui=true; firmware=true ;;
    tools/*) ui=true ;;
    Cargo.toml | Cargo.lock | rust-toolchain.toml) rust=true; firmware=true ;;
    scripts/* | .github/* | .githooks/* | release/*) rust=true; ui=true; firmware=true ;;
    docs/living-spec/* | docs/sowel-subset.md) rust=true ;;
  esac
  case "$f" in
    firmware/* | Cargo.lock | rust-toolchain.toml | release/* | .github/* \
      | scripts/validate.sh | scripts/qemu.sh | scripts/release.sh | scripts/scope.sh \
      | crates/core/src/update/* | crates/core/src/backup/* | crates/core/src/radio/* \
      | crates/core/src/storage* | crates/core/src/home/save* \
      | crates/core/src/recipes/script/* | crates/core/src/recipes/install/* | recipes/scripts/* \
      | crates/core/tests/*memory* | crates/sim/tests/*memory*)
      critical=true ;;
  esac
done

groups=""
$rust && groups="$groups rust"
$ui && groups="$groups ui"
$firmware && groups="$groups firmware"
echo "groups:$groups"
if $critical; then
  echo "level: critical"
elif $code; then
  echo "level: normal"
else
  echo "level: docs"
fi
