# Shared by scripts/qemu.sh and scripts/box.sh (source it, do not run it): the ESP32 tools, the
# partition table, the application image and the checks of a boot log.
#
# The caller sets ROOT (the repository's folder) and defines `fw_fail "<message>"`, which prints
# the message its own way and exits. Variables set here: ESPRESSIF, IDF_PATH, PY (ESP-IDF's
# Python), ESPTOOL, ESPEFUSE, GEN (commands, without `--chip`), APP_OFFSET, SLOT_BYTES.
#
# Tests point SOLITE_ESPRESSIF at a fake ESP-IDF tree instead of firmware/.embuild/espressif.

# The engine's limits, in KB (spec 016 R4.4): heap peak (D27) and internal RAM minimum free.
PEAK_LIMIT_KB=350
INTERNAL_MIN_KB=64

# The version is the workspace's, not written anywhere else.
fw_version() {
  sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml"
}

# ESP-IDF's Python environment and ESP-IDF itself, downloaded by the firmware build.
fw_find_idf() {
  ESPRESSIF=${SOLITE_ESPRESSIF:-$ROOT/firmware/.embuild/espressif}
  IDF_PATH=$(ls -d "$ESPRESSIF"/esp-idf/*/ 2>/dev/null | tail -n 1)
  IDF_PATH=${IDF_PATH%/}
  PY=$(ls "$ESPRESSIF"/python_env/*/bin/python 2>/dev/null | tail -n 1)
  [ -n "$IDF_PATH" ] && [ -n "$PY" ] || fw_fail "ESP-IDF was not found in firmware/.embuild (build the firmware first)."
}

# The tools: esptool.py, espefuse.py and gen_esp32part.py from the PATH when there, else ESP-IDF's.
fw_find_tools() {
  if command -v esptool.py >/dev/null 2>&1 && command -v espefuse.py >/dev/null 2>&1 \
    && command -v gen_esp32part.py >/dev/null 2>&1; then
    ESPTOOL=esptool.py
    ESPEFUSE=espefuse.py
    GEN=gen_esp32part.py
    return 0
  fi
  fw_find_idf
  ESPTOOL="$PY -m esptool"
  ESPEFUSE="$PY -m espefuse"
  GEN="$PY $IDF_PATH/components/partition_table/gen_esp32part.py"
}

# The bytes of a size as the partition table prints it: 4M, 64K or 0x400000.
fw_bytes() {
  case "$1" in
    *M) echo $((${1%M} * 1048576)) ;;
    *K) echo $((${1%K} * 1024)) ;;
    *) echo $(($1)) ;;
  esac
}

# Builds the partition table from a CSV: $1 the CSV, $2 the binary. Sets APP_OFFSET and SLOT_BYTES
# (the offset and the size of `ota_0`, read from the built table, not written anywhere).
fw_build_table() {
  $GEN "$1" "$2" >/dev/null 2>&1 || fw_fail "firmware/partitions.csv does not build."
  APP_OFFSET=$(fw_part "$2" ota_0 4)
  [ -n "$APP_OFFSET" ] || fw_fail "no ota_0 slot in firmware/partitions.csv."
  SLOT_SIZE=$(fw_part "$2" ota_0 5)
  [ -n "$SLOT_SIZE" ] || fw_fail "no size for the ota_0 slot in firmware/partitions.csv."
  SLOT_BYTES=$(fw_bytes "$SLOT_SIZE")
}

# A field of a partition of a built table: $1 the table, $2 the partition, $3 the column (4 offset,
# 5 size).
fw_part() {
  $GEN "$1" 2>/dev/null | awk -F, -v n="$2" -v c="$3" '$1 == n { gsub(/ /, "", $c); print $c }'
}

# The bootloader: from the esp-idf-sys build folder whose bootloader was built last (a folder
# rebuilt in place keeps its own date, so the folder's date would mislead). $1 the target folder.
fw_find_bootloader() {
  BOOTLOADER=$(ls -t "$1"/build/esp-idf-sys-*/out/build/bootloader/bootloader.bin 2>/dev/null | head -n 1)
  [ -f "$BOOTLOADER" ] || fw_fail "bootloader not found in $1/build."
}

# The application image: $1 the ELF, $2 the .bin; refused when larger than the `ota_0` slot.
fw_elf2image() {
  $ESPTOOL --chip esp32 elf2image --flash_mode dio --flash_size 16MB -o "$2" "$1" >/dev/null \
    || fw_fail "esptool could not make the application image of $1."
  APP_SIZE=$(wc -c < "$2" | tr -d ' ')
  [ "$APP_SIZE" -le "$SLOT_BYTES" ] || fw_fail "app.bin ($APP_SIZE bytes) is larger than the ota_0 slot ($SLOT_BYTES bytes)."
}

# One line of the image sizes (spec 033 R7): $1 the image's name, $2 its application image; needs
# SLOT_BYTES (fw_build_table). `<name>: <n> KB, <p> % of the <s> KB ota_0 slot`, with a warning
# above 75 % of the slot.
fw_size_line() {
  size=$(wc -c < "$2" | tr -d ' ')
  line="$1: $((size / 1024)) KB, $((size * 100 / SLOT_BYTES)) % of the $((SLOT_BYTES / 1024)) KB ota_0 slot"
  [ $((size * 100)) -gt $((SLOT_BYTES * 75)) ] && line="$line WARNING: above 75 % of the slot"
  echo "$line"
}

# A broken boot. $1 the log, $2 the way to count resets: `qemu` (the ROM's reset line, "rst:", seen
# a second time) or `box` (one after the first banner `solite <version>`: the resets before it,
# from the port opening or the script's own reset, are not counted).
fw_broken() {
  grep -aqE 'Guru Meditation Error|abort\(\) was called|panicked at|stack overflow in task' "$1" && return 0
  if [ "$2" = box ]; then
    banner=$(grep -anF -m 1 "solite $(fw_version)" "$1" | cut -d: -f1)
    [ -n "$banner" ] || return 1
    [ "$(tail -n +"$((banner + 1))" "$1" | grep -ac 'rst:')" -ge 1 ]
  else
    [ "$(grep -ac 'rst:' "$1")" -ge 2 ]
  fi
}

# The count of the last `flash: <n> writes since boot` line of a log (spec 025 R2), 0 when there is
# none. $1 the log.
fw_flash_writes() {
  n=$(grep -aoE 'flash: [0-9]+ writes since boot' "$1" | tail -n 1 | sed -E 's/flash: ([0-9]+) .*/\1/')
  echo "${n:-0}"
}

# The lines that must show: the banner, the memory, the storage and the network; then the engine's
# (sim image) or `radio: none` (box image). $1 the log, $2 the mode, `sim` or `box`.
fw_lines_ok() {
  grep -aqF "solite $(fw_version)" "$1" && grep -aq 'memory: internal' "$1" \
    && grep -aq 'storage: /data, ' "$1" && grep -aq 'network: .* no link started' "$1" || return 1
  if [ "$2" = sim ]; then grep -aq 'engine: ' "$1"; else grep -aq 'radio: none' "$1"; fi
}

# The first of those lines that is missing, named ($1 the log, $2 the mode); empty when all show.
fw_missing_line() {
  grep -aqF "solite $(fw_version)" "$1" || { echo "the banner (solite $(fw_version))"; return; }
  grep -aq 'memory: internal' "$1" || { echo "the memory line (memory: internal ...)"; return; }
  grep -aq 'storage: /data, ' "$1" || { echo "the storage line (storage: /data, ...)"; return; }
  # The radio image starts its Wi-Fi link (spec 023 R1.2): its line has no `no link started`.
  if [ "$2" = radio ]; then
    grep -aqE 'network: (wifi|ethernet) \(setting ' "$1" || { echo "the network line (network: ...)"; return; }
  else
    grep -aq 'network: .* no link started' "$1" || { echo "the network line (network: ...)"; return; }
  fi
  case "$2" in
    sim) grep -aq 'engine: ' "$1" || echo "the engine line (engine: ...)" ;;
    radio) fw_missing_radio_line "$1" ;;
    *) grep -aq 'radio: none' "$1" || echo "the radio line (radio: none)" ;;
  esac
}

# The first line of the radio image that is missing, named ($1 the log); empty when all show.
# The link's lines first (spec 023 R8.1): a `wifi:` line, unless the setting `ethernet` started no
# link; after a `wifi: connected` line, a `serving: http://` line.
fw_missing_radio_line() {
  if ! grep -aq 'network: ethernet (setting ' "$1"; then
    grep -aq 'wifi: ' "$1" || { echo "the Wi-Fi line (wifi: ...)"; return; }
  fi
  connected=$(grep -anF -m 1 'wifi: connected' "$1" | cut -d: -f1)
  if [ -n "$connected" ]; then
    tail -n +"$((connected + 1))" "$1" | grep -aq 'serving: http://' \
      || { echo "the serving line (serving: http://...)"; return; }
  fi
  grep -aq 'radio: uart [0-9]* baud' "$1" || { echo "the radio line (radio: uart <baud> baud)"; return; }
  grep -aq 'ezsp: ' "$1" || { echo "the ezsp line (ezsp: ...)"; return; }
  grep -aqE 'network: (formed|resumed)' "$1" || { echo "the network line (network: formed or resumed)"; return; }
  grep -aqE 'pairing: (open for|not opened)' "$1" || { echo "the pairing line (pairing: open for ... or pairing: not opened ...)"; return; }
  grep -aq 'serial: [0-9]* baud' "$1" || { echo "the serial line (serial: ... baud, due 240 s after the radio starts: run again with a larger --seconds)"; return; }
  grep -aq 'heap: internal' "$1" || echo "the heap line (heap: internal ..., due with the serial line: run again with a larger --seconds)"
}

# The radio image's log against its checks ($1 the log). Prints the reason and returns 1 when one
# fails; returns 0 when all hold. The banner counts a reboot even when the ROM's `rst:` lines are
# silenced (GPIO15 held low at reset).
fw_check_radio() {
  grep -aq 'no answer at any baud rate' "$1" && { echo "the radio answers at no speed (115200, 460800, 921600): see the failure ladder of spec 021 R8.3."; return 1; }
  grep -aq 'radio: stopped after' "$1" && { echo "the radio was restarted too often: $(grep -a 'radio: stopped after' "$1" | tail -n 1)"; return 1; }
  failed=$(grep -a 'pairing: failed:' "$1" | tail -n 1)
  [ -z "$failed" ] || { echo "the radio refused to open joining: $failed"; return 1; }
  grep -aq 'task_wdt' "$1" && { echo "a task watchdog fired (task_wdt)."; return 1; }
  refused=$(grep -a 'wifi: settings refused' "$1" | tail -n 1)
  [ -z "$refused" ] || { echo "the Wi-Fi settings were refused: $refused"; return 1; }
  [ "$(grep -acF "solite $(fw_version)" "$1")" -le 1 ] || { echo "the box rebooted (the banner shows more than once)."; return 1; }
  heap=$(grep -a 'heap: internal' "$1" | tail -n 1)
  [ -n "$heap" ] || return 0
  min=$(echo "$heap" | sed -n 's/.*heap: internal [0-9]* KB free (min \([0-9]*\) KB).*/\1/p')
  [ -n "$min" ] || { echo "cannot read the internal minimum from the heap line: $heap"; return 1; }
  [ "$min" -ge "$INTERNAL_MIN_KB" ] || { echo "the internal RAM minimum free is $min KB, below the $INTERNAL_MIN_KB KB reserve of decision D23."; return 1; }
  return 0
}

# The update's stack check of the sim image (D60, `firmware/src/update_check.rs`), $1 the log: the
# `update check:` line shows the three installations' statuses (200, 400, 400), and the `update`
# thread left at least UPDATE_STACK_MARGIN bytes of its stack unused, for the frames the box adds
# (the request's body reader and answer); and the pull's (see below). Prints the reason and returns 1 otherwise.
UPDATE_STACK_MARGIN=2048
PULL_STACK_MARGIN=1024
fw_check_update_stack() {
  check=$(grep -a 'update check: ' "$1" | tail -n 1)
  [ -n "$check" ] || { echo "the update check's line (update check: ...) is missing."; return 1; }
  echo "$check" | grep -qF 'valid 200, image changed 400, signature changed 400' \
    || { echo "the update check went wrong: ${check#*update check: }"; return 1; }
  unused=$(grep -aoE 'update: thread stack [0-9]+ of [0-9]+ bytes never used' "$1" | tail -n 1 \
    | sed -E 's/update: thread stack ([0-9]+) .*/\1/')
  [ -n "$unused" ] || { echo "the update thread's stack line (update: thread stack ...) is missing."; return 1; }
  [ "$unused" -ge "$UPDATE_STACK_MARGIN" ] || { echo "the update thread left $unused bytes of its stack unused, below the $UPDATE_STACK_MARGIN bytes kept for the box's request frames (D60)."; return 1; }
  # The pull of a release (spec 039 R10, spec 040 R6.8): one pull from an in-memory fetcher on an
  # `update-pull` thread, its first answer cut after 40 KB and resumed with a `Range` request,
  # status 200, and at least PULL_STACK_MARGIN of its 20480 bytes unused. The resume touches an
  # 8 KB stand-in for the TLS handshake (which QEMU cannot run) at its deepest point, so this one
  # rule replaces the 8192 bytes unused of the pull without a resume (D68).
  pull=$(grep -a 'update pull check: ' "$1" | tail -n 1)
  [ -n "$pull" ] || { echo "the pull check's line (update pull check: ...) is missing."; return 1; }
  echo "$pull" | grep -qF 'status 200' \
    || { echo "the pull check went wrong: ${pull#*update pull check: }"; return 1; }
  echo "$pull" | grep -qF 'resumes 1' \
    || { echo "the pull check did not resume its cut download once: ${pull#*update pull check: }"; return 1; }
  pull_unused=$(grep -aoE 'update-pull: thread stack [0-9]+ of [0-9]+ bytes never used' "$1" | tail -n 1 \
    | sed -E 's/update-pull: thread stack ([0-9]+) .*/\1/')
  [ -n "$pull_unused" ] || { echo "the update-pull thread's stack line (update-pull: thread stack ...) is missing."; return 1; }
  [ "$pull_unused" -ge "$PULL_STACK_MARGIN" ] || { echo "the update-pull thread left $pull_unused of its 20480 bytes unused, below the $PULL_STACK_MARGIN bytes kept after the resume's 8 KB handshake stand-in (spec 040 R6.8, D68)."; return 1; }
  return 0
}

# QuickJS in an image (spec 041): true when the ELF $1 holds it (`JS_NewRuntime`, which the linker
# keeps only when something calls it). The symbol table first (ESP-IDF's own `nm`, found beside its
# compiler); the bytes of the file when no `nm` is there.
fw_has_quickjs() {
  ESPRESSIF=${ESPRESSIF:-${SOLITE_ESPRESSIF:-$ROOT/firmware/.embuild/espressif}}
  nm=$(ls "$ESPRESSIF"/tools/xtensa-esp-elf/*/xtensa-esp-elf/bin/xtensa-esp32-elf-nm 2>/dev/null | tail -n 1)
  if [ -n "$nm" ]; then
    "$nm" "$1" 2>/dev/null | grep -q ' JS_NewRuntime'
  else
    grep -aq 'JS_NewRuntime' "$1"
  fi
}

# The measuring and the box's images carry no QuickJS, the radio image and the QEMU image's script
# build do: $1 the ELF, $2 the image's name, $3 `with` or `without`. Prints the reason and returns 1
# when it is not so (a search that finds nothing anywhere would pass for the wrong reason).
fw_check_quickjs() {
  if fw_has_quickjs "$1"; then found=with; else found=without; fi
  [ "$found" = "$3" ] || { echo "the $2 image is $found QuickJS (JS_NewRuntime in its symbols), it must be $3."; return 1; }
  return 0
}

# No image but the QEMU script build carries a script (spec 042 R3.10): $1 the ELF, $2 the image's
# name, $3 `with` or `without` the id `id: "motion-light"` of the script's source. The QEMU
# image reads that source for its own phase, so it is the positive control of the search (a search
# that finds nothing anywhere would pass for the wrong reason). Prints the reason and returns 1
# when it is not so.
fw_check_script_source() {
  if grep -aqF 'id: "motion-light"' "$1"; then found=with; else found=without; fi
  [ "$found" = "$3" ] || { echo "the $2 image is $found the source of motion-light (id: \"motion-light\" in its ELF), it must be $3: no script is built into an image."; return 1; }
  return 0
}

# The install step of the QEMU image's script phase (spec 042, `firmware/src/script_phase.rs`), $1
# the log: `script phase: install 200, boot load ok, uninstall 200` is there, and every `recipe:
# thread stack <n> of 20480 bytes never used` line (one per job) keeps at least RECIPE_STACK_MARGIN
# bytes unused, and `script phase: install schedule-on-off: internal RAM dip <n> KB` is at most
# RECIPE_LOAD_DIP_KB. Prints the reason and returns 1 otherwise.
RECIPE_STACK_MARGIN=2048
# The most the internal RAM falls while schedule-on-off (20 slots, texts in two languages) installs:
# 42 KB in QEMU (motion-light: 45 KB, the catalogue and the download), over 94 KB at its load alone before spec 056's
# fix of the definition's trees.
RECIPE_LOAD_DIP_KB=60
fw_check_recipe_install() {
  step=$(grep -a 'script phase: install ' "$1" | tail -n 1)
  [ -n "$step" ] || { echo "the install step's line (script phase: install ...) is missing."; return 1; }
  echo "$step" | grep -qF 'install 200, boot load ok, uninstall 200' \
    || { echo "the install step went wrong: ${step#*script phase: }"; return 1; }
  dip=$(grep -a 'script phase: install schedule-on-off: internal RAM dip ' "$1" | tail -n 1 | sed -E 's/.*dip ([0-9]+) KB.*/\1/')
  [ -n "$dip" ] || { echo "the install step's line for schedule-on-off (internal RAM dip) is missing."; return 1; }
  [ "$dip" -le "$RECIPE_LOAD_DIP_KB" ] || { echo "installing schedule-on-off took $dip KB of the internal RAM at its lowest, above the $RECIPE_LOAD_DIP_KB KB kept (the definition's trees, spec 056)."; return 1; }
  lines=$(grep -aoE 'recipe: thread stack [0-9]+ of 20480 bytes never used' "$1")
  [ -n "$lines" ] || { echo "the recipe thread's stack line (recipe: thread stack ...) is missing."; return 1; }
  least=$(echo "$lines" | sed -E 's/recipe: thread stack ([0-9]+) .*/\1/' | sort -n | head -n 1)
  [ "$least" -ge "$RECIPE_STACK_MARGIN" ] || { echo "the recipe thread left $least of its 20480 bytes unused, below the $RECIPE_STACK_MARGIN bytes kept."; return 1; }
  return 0
}

# The script phase of the QEMU image (spec 041 R6.2, `firmware/src/script_phase.rs`), $1 the log:
# the `recipes: script motion-light ...` line is there, the internal RAM's minimum is at
# least INTERNAL_MIN_KB, the engine thread and the `script` thread keep at least SCRIPT_STACK_MARGIN
# bytes of their stack unused, the home's heap with the realistic home's three instances running
# (`heap peak <n> KB` of the line: free before the engine started minus the lowest free of the phase)
# is at most SCRIPT_PHASE_HEAP_LIMIT_KB, the 32 KB script's instances (`script sizes: big-recipe`, spec 053) peak under the limit, the depth script called the engine at its deepest on both (and the
# create on the script thread answered 201), and the three instances' peak (`script sizes:` line, in bytes) is at
# most 75 % of the script memory limit (SCRIPT_MEMORY_LIMIT in the core). Prints the reason and
# returns 1 otherwise.
# D27's 350 KB for the engine plus the 224 KB ceiling of one script runtime (spec 046 R8, D81).
SCRIPT_PHASE_HEAP_LIMIT_KB=574
SCRIPT_STACK_MARGIN=2048
fw_check_script_phase() {
  line=$(grep -a 'recipes: script motion-light .* KB source; psram' "$1" | tail -n 1)
  [ -n "$line" ] || { echo "the script phase's line (recipes: script motion-light ... KB source; psram ...) is missing."; return 1; }
  min=$(echo "$line" | sed -n 's/.*internal +[0-9]* KB (min \([0-9]*\) KB free).*/\1/p')
  [ -n "$min" ] || { echo "cannot read the internal minimum from the script line: $line"; return 1; }
  [ "$min" -ge "$INTERNAL_MIN_KB" ] || { echo "the internal RAM minimum free is $min KB during the script phase, below the $INTERNAL_MIN_KB KB kept for the network."; return 1; }
  engine=$(echo "$line" | sed -n 's/.*engine stack \([0-9]*\) of [0-9]* unused.*/\1/p')
  script=$(echo "$line" | sed -n 's/.*script thread stack \([0-9]*\) of [0-9]* unused.*/\1/p')
  [ -n "$engine" ] && [ -n "$script" ] || { echo "cannot read the stacks from the script line: $line"; return 1; }
  heap=$(echo "$line" | sed -n 's/.*; heap peak \([0-9]*\) KB.*/\1/p')
  [ -n "$heap" ] || { echo "cannot read the heap peak from the script line: $line"; return 1; }
  [ "$heap" -le "$SCRIPT_PHASE_HEAP_LIMIT_KB" ] || { echo "the heap peaks at $heap KB with the home's instances running, above the $SCRIPT_PHASE_HEAP_LIMIT_KB KB of the script phase (spec 046)."; return 1; }
  [ "$engine" -ge "$SCRIPT_STACK_MARGIN" ] || { echo "the engine thread keeps $engine bytes of its stack unused with a script recursing, below $SCRIPT_STACK_MARGIN."; return 1; }
  [ "$script" -ge "$SCRIPT_STACK_MARGIN" ] || { echo "the script thread keeps $script bytes of its stack unused, below $SCRIPT_STACK_MARGIN."; return 1; }
  # The recursion reached the engine at its deepest on both threads, and the router's create on the
  # `script` thread answered 201: a stack that is only "unused" because the script never got there
  # would pass the margin for the wrong reason.
  for who in engine script; do
    depth_line=$(grep -a "script phase: $who thread, " "$1" | tail -n 1)
    called=$(echo "$depth_line" | sed -n 's/.*engine called at \([0-9]*\).*/\1/p')
    [ -n "$called" ] || { echo "cannot read where the depth script called the engine on the $who thread: $depth_line"; return 1; }
    [ "$called" -gt 0 ] || { echo "the depth script never called the engine at its deepest on the $who thread (engine called at 0)."; return 1; }
  done
  post=$(grep -a 'script phase: script thread, POST status' "$1" | tail -n 1 | sed -n 's/.*POST status \([0-9]*\),.*/\1/p')
  [ "$post" = 201 ] || { echo "the create of the depth script on the script thread answered ${post:-nothing}, not 201."; return 1; }
  limit=$(sed -n 's/^pub const SCRIPT_MEMORY_LIMIT: usize = \([0-9]*\) \* 1024;/\1/p' "$ROOT/crates/core/src/recipes/script/limits.rs")
  [ -n "$limit" ] || { echo "cannot read SCRIPT_MEMORY_LIMIT from the core."; return 1; }
  sizes=$(grep -a 'script sizes: motion-light' "$1" | tail -n 1)
  peak3=$(echo "$sizes" | sed -n 's/.*3 instances [0-9]* B (peak \([0-9]*\) B).*/\1/p')
  [ -n "$peak3" ] || { echo "cannot read the three instances' peak from the sizes line: $sizes"; return 1; }
  big=$(grep -a 'script sizes: big-recipe' "$1" | tail -n 1)
  peak_big=$(echo "$big" | sed -n 's/.*instances [0-9]* B (peak \([0-9]*\) B).*/\1/p')
  [ -n "$peak_big" ] || { echo "cannot read the peak of the 32 KB script's instances from its sizes line (script sizes: big-recipe): $big"; return 1; }
  [ "$peak_big" -le $((limit * 1024)) ] || { echo "the 32 KB script with its instances peaks at $peak_big bytes, above the $limit KB script memory limit (spec 053)."; return 1; }
  [ $((peak3 * 100)) -le $((limit * 1024 * 75)) ] || { echo "three motion-light instances peak at $peak3 bytes, above 75 % of the $limit KB script memory limit."; return 1; }
  return 0
}

# The engine's line against its limits ($1 the log). Prints the reason and returns 1 when a limit
# is crossed; returns 0 and sets ENGINE when all hold.
fw_check_engine() {
  ENGINE=$(grep -a 'engine: ' "$1" | tail -n 1)
  PEAK=$(echo "$ENGINE" | sed -n 's/.*(peak \([0-9]*\) KB).*/\1/p')
  INTERNAL_MIN=$(echo "$ENGINE" | sed -n 's/.*internal [0-9]* KB free (min \([0-9]*\) KB).*/\1/p')
  [ -n "$PEAK" ] && [ -n "$INTERNAL_MIN" ] || { echo "cannot read the peak and the internal minimum from the engine line."; return 1; }
  [ "$PEAK" -le "$PEAK_LIMIT_KB" ] || { echo "the engine's heap peak is $PEAK KB, above the $PEAK_LIMIT_KB KB of decision D27."; return 1; }
  DEVICES=$(echo "$ENGINE" | sed -n 's/^.*engine: \([0-9]*\) devices (\([0-9]*\) reported).*/\1/p')
  REPORTED=$(echo "$ENGINE" | sed -n 's/^.*engine: \([0-9]*\) devices (\([0-9]*\) reported).*/\2/p')
  [ -n "$DEVICES" ] && [ -n "$REPORTED" ] || { echo "cannot read the reported devices from the engine line."; return 1; }
  [ "$REPORTED" -eq "$DEVICES" ] || { echo "only $REPORTED of the $DEVICES devices got a measure into the engine."; return 1; }
  grep -aq 'scenario: started' "$1" || { echo "the scenario never started."; return 1; }
  DROPPED=$(sed -n '/scenario: started/,$p' "$1" | grep -ac 'callback queue full' || true)
  [ "$DROPPED" -eq 0 ] || { echo "the radio dropped $DROPPED callbacks while the scenario played."; return 1; }
  [ "$INTERNAL_MIN" -ge "$INTERNAL_MIN_KB" ] || { echo "the internal RAM minimum free is $INTERNAL_MIN KB, below the $INTERNAL_MIN_KB KB kept for the network."; return 1; }
  return 0
}
