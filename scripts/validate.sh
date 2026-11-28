#!/bin/sh
# The verdict, one command: docs, format, lint, types, the Sowel contract, tests, UI budget, the four ESP32 firmware
# images (the box's, the one QEMU runs, the measuring one, the radio one) and the engine's run in QEMU.
# QuickJS (script recipes, spec 041) is in the radio image and the QEMU image's script build only: a
# symbol search of each ELF proves the box's, the plain sim and the measuring images carry none.
#
#   sh scripts/validate.sh          everything (before a commit, in the nightly CI)
#   sh scripts/validate.sh --fast   the quick checks only (the pre-commit hook): docs, format, lint
#   sh scripts/validate.sh --only rust|ui|firmware
#                                   one third of the verdict, for the CI's parallel jobs: the three
#                                   together are exactly the full run
#   sh scripts/validate.sh --only e2e
#                                   opt-in, in no other run and not in the CI (it needs a browser):
#                                   the simulated path driven in headless Chromium
#   sh scripts/validate.sh --changed [--base <ref>]
#                                   the development loop: the quick checks and the tests of the
#                                   groups the change touches (`scripts/scope.sh`), no firmware build,
#                                   no slow test, no QEMU; the UI's tests related to the changed files
#   sh scripts/validate.sh --pre-merge [--base <ref>] [--only rust|ui|firmware]
#                                   the pull request's verdict, scaled to the change: docs only, the
#                                   quick checks; normal, the touched groups without the slow checks;
#                                   critical (a path where a fault reaches the box), the touched
#                                   groups in full, QEMU included. A group the change does not touch
#                                   is skipped; main and the nightly run everything
#   sh scripts/validate.sh --skip-slow [--only rust|ui|firmware]
#                                   the same without the slow checks: the Rust tests marked
#                                   `#[ignore = "slow: ..."]` and the engine's run in QEMU (the
#                                   pull-request CI); the full run (the Mac, the nightly CI) has them
#
# Green only when every step is. Never weaken a step to get green: fix the cause.
set -e
cd "$(dirname "$0")/.."

# Tools installed in the user's folder (rustup, espup) are not always on the PATH of a hook.
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

fast=false
slow=true
only=""
mode=""
base="origin/main"
while [ $# -gt 0 ]; do
  case "$1" in
    --fast) fast=true; shift ;;
    --skip-slow) slow=false; shift ;;
    --changed) mode=changed; slow=false; shift ;;
    --pre-merge) mode=pre-merge; shift ;;
    --base) base="$2"; shift 2 ;;
    --only)
      only="$2"
      case "$only" in rust | ui | firmware | e2e) ;; *) echo "validate: --only takes rust, ui, firmware or e2e" >&2; exit 2 ;; esac
      shift 2
      ;;
    *) echo "validate: unknown option $1 (--fast, --skip-slow, --changed, --pre-merge, --base <ref>, --only <group>)" >&2; exit 2 ;;
  esac
done

# --changed and --pre-merge: the groups and the level from the change's files.
touched="rust ui firmware"
docs_only=false
if [ -n "$mode" ]; then
  scope=$(sh scripts/scope.sh "$base")
  touched=$(printf '%s\n' "$scope" | sed -n 's/^groups: *//p')
  level=$(printf '%s\n' "$scope" | sed -n 's/^level: *//p')
  printf 'validate --%s: groups [%s], level %s (from the files since %s)\n' "$mode" "$touched" "$level" "$base"
  if [ "$mode" = pre-merge ]; then
    case "$level" in
      docs) docs_only=true ;;
      normal) slow=false ;;
      critical) slow=true ;;
    esac
  fi
  if [ -n "$only" ] && ! printf ' %s ' "$touched" | grep -q " $only "; then
    # The rust third still checks the docs: every change has some.
    if [ "$only" = rust ]; then
      touched="rust"
      docs_only=true
    else
      printf 'validate: the %s group is not touched by this change: skipped\n' "$only"
      exit 0
    fi
  fi
fi
$docs_only && fast=true
# True when the group is part of this run: named by --only (or every group), and touched.
want() {
  { [ -z "$only" ] || [ "$only" = "$1" ]; } && printf ' %s ' "$touched" | grep -q " $1 "
}

step() { printf '\n== %s\n' "$1"; }

# The radio image embeds the built interface (spec 023 R7.4): the firmware group builds ui/dist too.
if { want ui || want firmware; } && [ ! -d ui/node_modules ]; then
  echo "validate: ui/node_modules is missing. Run: cd ui && npm ci" >&2
  exit 1
fi

if want ui && [ ! -d tools/z2m-coverage/node_modules ]; then
  echo "validate: tools/z2m-coverage/node_modules is missing. Run: cd tools/z2m-coverage && npm ci" >&2
  exit 1
fi

if want rust || { [ -z "$only" ] && [ -n "$mode" ]; }; then
  step "Docs"
  sh scripts/check-docs.sh
fi

if want rust; then
  # The firmware is outside the workspace, yet its ESP application descriptor must carry the
  # version the core reports (spec 032 R7): the two `version` lines are the same.
  step "Versions: firmware/Cargo.toml and the workspace"
  workspace_version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1)
  firmware_version=$(sed -n '/^\[package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' firmware/Cargo.toml | head -n 1)
  [ -n "$workspace_version" ] && [ "$workspace_version" = "$firmware_version" ] \
    || { echo "validate: firmware/Cargo.toml version is '$firmware_version', the workspace's Cargo.toml version is '$workspace_version': they must be equal" >&2; exit 1; }

  step "Rust: format"
  cargo fmt --all --check
  rustfmt --check --edition 2024 firmware/build.rs firmware/src/*.rs

  step "Rust: lint (core, host)"
  cargo clippy --workspace --all-targets --quiet -- -D warnings
fi

if want ui; then
  step "UI and tools: format, lint"
  (cd ui && npm run -s format:check && npm run -s lint)
  (cd tools/z2m-coverage && npm run -s format:check)
fi

if $fast; then
  printf '\nvalidate --fast: OK\n'
  exit 0
fi

if want rust; then
  # Spec 027: Solite's names against Sowel's (the snapshot, and ../sowel when present).
  step "Sowel contract"
  cargo test -p solite-core --features script-recipes --quiet --test sowel_contract -- --nocapture solite_is_a_subset_of_sowel

  if [ "$mode" = changed ] && ! printf ' %s ' "$touched" | grep -q ' firmware '; then
    # Only crates outside the core and the SDK: their own tests (no other crate depends on them).
    crates=$(sh scripts/scope.sh --crates "$base")
    step "Rust: tests of the changed crates ($crates), without the slow ones"
    for crate in $crates; do cargo test -p "$crate" --quiet; done
  elif $slow; then
    # The slow tests are the ignored ones: no test is ignored for any other reason.
    step "Rust: tests (core, host), the slow ones included"
    cargo test --workspace --quiet -- --include-ignored
  else
    step "Rust: tests (core, host), without the slow ones"
    cargo test --workspace --quiet
  fi
fi

if want ui && [ "$mode" = changed ]; then
  step "UI: types, the tests related to the changed files"
  merge=$(git merge-base "$base" HEAD 2>/dev/null || echo HEAD)
  (cd ui && npm run -s typecheck && npx vitest run --changed "$merge" --passWithNoTests --silent)
elif want ui; then
  step "UI: types, tests, build, budget"
  (cd ui && npm run -s typecheck && npm test --silent && npm run -s build >/dev/null && npm run -s check:bundle)

  step "Tools: types, tests"
  (cd tools/z2m-coverage && npm run -s typecheck && npm test --silent)
fi

if want firmware && [ "$mode" = changed ]; then
  step "Firmware: not built in the development loop (--pre-merge and the CI build it)"
elif want firmware; then
  step "Firmware: the interface built for the radio image (ui/dist, always rebuilt: an old one would be embedded)"
  (cd ui && npm run -s build >/dev/null)
  script=$(grep -o 'assets/index-[A-Za-z0-9_-]*\.js' ui/dist/index.html | head -n 1)
  [ -n "$script" ] || { echo "validate: ui/dist/index.html names no assets/index-*.js script" >&2; exit 1; }

  step "Firmware: ESP32 builds and lint (the box's image, the QEMU image with feature sim (and sim,scripts), the measuring image with feature flash-saves, the radio image with feature radio)"
  if [ -f "$HOME/export-esp.sh" ]; then
    . "$HOME/export-esp.sh"
  else
    echo "validate: the ESP32 toolchain is not set up (espup install, see README.md)." >&2
    exit 1
  fi
  # The size of each image (spec 033 R7): its application image is made right after its build (the
  # three images that are not the radio's share one ELF path), into a temporary folder.
  ROOT=$(pwd -P)
  fw_fail() { echo "validate: $1" >&2; exit 1; }
  . scripts/lib/firmware.sh
  sizes_dir=$(mktemp -d "${TMPDIR:-/tmp}/solite-sizes.XXXXXX")
  trap 'rm -rf "$sizes_dir"' EXIT
  : >"$sizes_dir/lines"
  # $1 the image's name, $2 its ELF, $3 `with` or `without` QuickJS (spec 041: a symbol search; the
  # ELF is read before the next build overwrites it). The tools exist once the first build has
  # downloaded ESP-IDF.
  # $4 `with` or `without` the source of `motion-light` (spec 042 R3.10): only the QEMU
  # image's script build reads it, for its own phase.
  measure_image() {
    _why=$(fw_check_quickjs "$2" "$1" "$3") || { echo "validate: $_why" >&2; exit 1; }
    _why=$(fw_check_script_source "$2" "$1" "$4") || { echo "validate: $_why" >&2; exit 1; }
    if [ ! -f "$sizes_dir/table.bin" ]; then
      fw_find_tools
      fw_build_table partitions.csv "$sizes_dir/table.bin"
    fi
    fw_elf2image "$2" "$sizes_dir/app.bin"
    fw_size_line "$1" "$sizes_dir/app.bin" >>"$sizes_dir/lines"
  }
  (
    cd firmware
    cargo build --release --quiet && cargo clippy --release --quiet -- -D warnings
    measure_image box target/xtensa-esp32-espidf/release/solite-firmware without without
    cargo build --release --features sim --quiet && cargo clippy --release --features sim --quiet -- -D warnings
    measure_image sim target/xtensa-esp32-espidf/release/solite-firmware without without
    # The image QEMU runs: the sim image with the scripts (spec 041); the difference with `sim` is
    # what QuickJS and the scripts cost in flash.
    cargo build --release --features sim,scripts --quiet && cargo clippy --release --features sim,scripts --quiet -- -D warnings
    measure_image sim+scripts target/xtensa-esp32-espidf/release/solite-firmware with with
    cargo build --release --features flash-saves --quiet && cargo clippy --release --features flash-saves --quiet -- -D warnings
    measure_image flash-saves target/xtensa-esp32-espidf/release/solite-firmware without without
    # The radio image has its own target folder and its own ESP-IDF settings (spec 021 R1.1): the
    # other images keep theirs.
    export CARGO_TARGET_DIR=target/radio ESP_IDF_SDKCONFIG_DEFAULTS="sdkconfig.defaults;sdkconfig.radio.defaults"
    cargo build --release --features radio --quiet && cargo clippy --release --features radio --quiet -- -D warnings
    measure_image radio target/radio/xtensa-esp32-espidf/release/solite-firmware with without
    # The settings file was read: the UART interrupt is in internal RAM in the radio build, and
    # in none of the others.
    grep -aqs '^CONFIG_UART_ISR_IN_IRAM=y' target/radio/xtensa-esp32-espidf/release/build/esp-idf-sys-*/out/sdkconfig \
      || { echo "validate: the radio build lacks CONFIG_UART_ISR_IN_IRAM=y (sdkconfig.radio.defaults was not read)" >&2; exit 1; }
    ! grep -aqs '^CONFIG_UART_ISR_IN_IRAM=y' target/xtensa-esp32-espidf/release/build/esp-idf-sys-*/out/sdkconfig \
      || { echo "validate: the other images' build holds CONFIG_UART_ISR_IN_IRAM=y: they must keep their settings" >&2; exit 1; }
    # The WebSocket of `/ws` is on in the radio build only (spec 023 R7.4).
    grep -aqs '^CONFIG_HTTPD_WS_SUPPORT=y' target/radio/xtensa-esp32-espidf/release/build/esp-idf-sys-*/out/sdkconfig \
      || { echo "validate: the radio build lacks CONFIG_HTTPD_WS_SUPPORT=y (sdkconfig.radio.defaults was not read)" >&2; exit 1; }
    ! grep -aqs '^CONFIG_HTTPD_WS_SUPPORT=y' target/xtensa-esp32-espidf/release/build/esp-idf-sys-*/out/sdkconfig \
      || { echo "validate: the other images' build holds CONFIG_HTTPD_WS_SUPPORT=y: they must keep their settings" >&2; exit 1; }
    # `/ws` checks a client and sends its welcome in ESP-IDF's post-handshake callback (D37): without
    # it the handshake succeeds and the client gets nothing.
    grep -aqs '^CONFIG_HTTPD_WS_POST_HANDSHAKE_CB_SUPPORT=y' target/radio/xtensa-esp32-espidf/release/build/esp-idf-sys-*/out/sdkconfig \
      || { echo "validate: the radio build lacks CONFIG_HTTPD_WS_POST_HANDSHAKE_CB_SUPPORT=y (sdkconfig.radio.defaults was not read)" >&2; exit 1; }
    # The engine's task watchdog (spec 036 R8): 20 s, and a timeout panics (the supervisor reads
    # it at the next boot) in the radio build; the other images keep the default (a log line).
    grep -aqs '^CONFIG_ESP_TASK_WDT_TIMEOUT_S=20' target/radio/xtensa-esp32-espidf/release/build/esp-idf-sys-*/out/sdkconfig \
      || { echo "validate: the radio build lacks CONFIG_ESP_TASK_WDT_TIMEOUT_S=20 (sdkconfig.radio.defaults was not read)" >&2; exit 1; }
    grep -aqs '^CONFIG_ESP_TASK_WDT_PANIC=y' target/radio/xtensa-esp32-espidf/release/build/esp-idf-sys-*/out/sdkconfig \
      || { echo "validate: the radio build lacks CONFIG_ESP_TASK_WDT_PANIC=y (sdkconfig.radio.defaults was not read)" >&2; exit 1; }
    ! grep -aqs '^CONFIG_ESP_TASK_WDT_PANIC=y' target/xtensa-esp32-espidf/release/build/esp-idf-sys-*/out/sdkconfig \
      || { echo "validate: the other images' build holds CONFIG_ESP_TASK_WDT_PANIC=y: they must keep their settings" >&2; exit 1; }
    # The certificate bundle the box's https check and pull verify with (spec 039 R8): the common list.
    grep -aqs '^CONFIG_MBEDTLS_CERTIFICATE_BUNDLE_DEFAULT_CMN=y' target/radio/xtensa-esp32-espidf/release/build/esp-idf-sys-*/out/sdkconfig \
      || { echo "validate: the radio build lacks CONFIG_MBEDTLS_CERTIFICATE_BUNDLE_DEFAULT_CMN=y (sdkconfig.radio.defaults was not read)" >&2; exit 1; }
    # The interface embedded is the one just built: the marker and the name of its hashed script.
    elf=target/radio/xtensa-esp32-espidf/release/solite-firmware
    grep -aqF 'solite-ui ' "$elf" || { echo "validate: the radio image holds no embedded interface (no solite-ui marker)" >&2; exit 1; }
    grep -aqF "$script" "$elf" || { echo "validate: the radio image does not embed the current ui/dist ($script missing): rebuild it" >&2; exit 1; }
  )

  step "Firmware: image sizes"
  cat "$sizes_dir/lines"
  rm -rf "$sizes_dir"
  trap - EXIT

  if $slow; then
    step "Firmware: the engine runs in QEMU (heap peak and internal RAM checked)"
    sh scripts/qemu.sh
  else
    step "Firmware: the engine's run in QEMU is skipped (--skip-slow)"
  fi
fi

if [ "$only" = "e2e" ]; then
  step "End to end: the simulated path in headless Chromium"
  [ -d tools/e2e/node_modules ] || { echo "validate: tools/e2e/node_modules is missing. Run: cd tools/e2e && npm ci" >&2; exit 1; }
  (cd tools/e2e && npm run -s format:check)
  sh scripts/e2e-sim.sh
fi

if [ -n "$mode" ]; then
  printf '\nvalidate --%s: OK (groups [%s]%s)\n' "$mode" "$touched" "$($slow && echo ', slow checks and QEMU included' || echo ', no slow check, no QEMU')"
elif $slow; then
  printf '\nvalidate: OK\n'
else
  printf '\nvalidate --skip-slow: OK (the slow tests and QEMU did not run)\n'
fi
