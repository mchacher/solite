//! The free memory of the device, and the line that reports it in the logs.

/// Free memory, in bytes, as the platform reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryStats {
    /// Free internal RAM.
    pub internal_free: usize,
    /// Largest block of internal RAM that can still be allocated in one piece.
    pub internal_largest: usize,
    /// Free PSRAM (external RAM) and its total size; `None` when the device has no PSRAM.
    pub psram: Option<PsramStats>,
}

/// The PSRAM sizes, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PsramStats {
    pub free: usize,
    pub total: usize,
}

/// Bytes to KiB, rounded down.
fn kib(bytes: usize) -> usize {
    bytes / 1024
}

/// The line written after the banner: `memory: internal 245 KB free (largest block 110 KB), psram
/// 2039 KB free of 2048 KB`, or `... psram none` without PSRAM. Sizes in KiB, rounded down.
pub fn memory_line(stats: &MemoryStats) -> String {
    let psram = match stats.psram {
        Some(p) => format!("{} KB free of {} KB", kib(p.free), kib(p.total)),
        None => "none".to_string(),
    };
    format!(
        "memory: internal {} KB free (largest block {} KB), psram {}",
        kib(stats.internal_free),
        kib(stats.internal_largest),
        psram
    )
}

/// What the engine holds and what it cost, for the line the firmware logs after the scenario of
/// the QEMU image (spec 016 R3.5). Sizes in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineStats {
    pub devices: usize,
    /// The devices that got at least one measure into the engine while the scenario played.
    pub reported: usize,
    /// The zones made by the user (the root zone is not counted).
    pub zones: usize,
    pub equipments: usize,
    pub recipes: usize,
    /// Heap used by the engine: free before it started minus free after the scenario (internal
    /// RAM and PSRAM together).
    pub heap_used: usize,
    /// Free before it started minus the lowest free seen while it ran.
    pub heap_peak: usize,
    pub internal_free: usize,
    /// The lowest internal RAM free seen while the engine ran.
    pub internal_min: usize,
    pub psram_free: usize,
    pub psram_min: usize,
}

/// The line logged after the scenario: `engine: 50 devices (50 reported), 10 zones, 50 equipments, 3 recipes;
/// heap 160 KB (peak 190 KB); internal 120 KB free (min 90 KB); psram 1700 KB free (min 1650
/// KB)`. Sizes in KiB, rounded down.
pub fn engine_line(stats: &EngineStats) -> String {
    format!(
        "engine: {} devices ({} reported), {} zones, {} equipments, {} recipes; heap {} KB (peak {} KB); internal {} KB free (min {} KB); psram {} KB free (min {} KB)",
        stats.devices,
        stats.reported,
        stats.zones,
        stats.equipments,
        stats.recipes,
        kib(stats.heap_used),
        kib(stats.heap_peak),
        kib(stats.internal_free),
        kib(stats.internal_min),
        kib(stats.psram_free),
        kib(stats.psram_min),
    )
}

/// What the radio image reports every few minutes: the free memory with its minimums since boot,
/// and the engine thread's stack. Sizes in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapStats {
    pub internal_free: usize,
    /// The lowest internal RAM free since boot.
    pub internal_min: usize,
    /// The largest free block of internal RAM (fragmentation indicator).
    pub internal_largest: usize,
    pub psram_free: usize,
    /// The lowest PSRAM free since boot.
    pub psram_min: usize,
    /// The largest free block of PSRAM; 0 when there is none.
    pub psram_largest: usize,
    /// Stack bytes never used by the engine thread (its high-water mark).
    pub stack_unused: usize,
    /// The engine thread's whole stack.
    pub stack_total: usize,
    /// The HTTP workers' stack, `(unused, total)` in bytes: the smallest high-water mark of the
    /// workers. `None` when the image serves no HTTP (spec 022).
    pub http_stack: Option<(usize, usize)>,
}

/// The line the radio image logs every few minutes: `heap: internal 120 KB free (min 90 KB),
/// largest block 60 KB, psram 1700 KB free (min 1650 KB), largest block 1500 KB, engine stack 20 KB
/// unused of 32 KB`, followed by `, http stack 6 KB unused of 10 KB` when the image serves HTTP. Sizes in KiB, rounded down.
pub fn heap_line(stats: &HeapStats) -> String {
    let http = stats
        .http_stack
        .map_or_else(String::new, |(unused, total)| {
            format!(
                ", http stack {} KB unused of {} KB",
                kib(unused),
                kib(total)
            )
        });
    format!(
        "heap: internal {} KB free (min {} KB), largest block {} KB, psram {} KB free (min {} KB), largest block {} KB, engine stack {} KB unused of {} KB{http}",
        kib(stats.internal_free),
        kib(stats.internal_min),
        kib(stats.internal_largest),
        kib(stats.psram_free),
        kib(stats.psram_min),
        kib(stats.psram_largest),
        kib(stats.stack_unused),
        kib(stats.stack_total),
    )
}

/// One line per loaded script recipe, printed with each `heap:` line of the radio image (spec 041
/// R6.3): `scripts: <id> <n> instances, <n> calls, average <µs> µs, max <µs> µs, heap <KB> KB (peak
/// <KB> KB)`. `heap` is what its runtime holds now, `peak` the most it held at the end of a call.
pub fn scripts_lines(stats: &[(String, solite_recipe_sdk::RunStats)]) -> Vec<String> {
    stats
        .iter()
        .map(|(id, s)| {
            let average = s.total_us.checked_div(s.calls).unwrap_or(0);
            format!(
                "scripts: {id} {} instances, {} calls, average {average} µs, max {} µs, heap {} KB (peak {} KB)",
                s.instances,
                s.calls,
                s.max_us,
                kib(s.held),
                kib(s.peak),
            )
        })
        .collect()
}

/// What the QEMU image's script phase measured (spec 041 R6.2). Sizes in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptPhase {
    /// The script's source.
    pub source: usize,
    /// PSRAM taken by the phase at its end, and at the most (below the level before it).
    pub psram_used: usize,
    pub psram_peak: usize,
    /// The heap (internal RAM and PSRAM) of the home with its script loaded and its three
    /// instances running, at the most: free before the engine started minus the lowest free seen
    /// until the scenario had played again (spec 046 R8).
    pub heap_peak: usize,
    /// Internal RAM taken by the phase at its end (a negative change reads 0), and its lowest free.
    pub internal_used: usize,
    pub internal_min: usize,
    /// The engine thread's stack: `(unused, total)`, the lowest after the deep recursion.
    pub engine_stack: (usize, usize),
    /// The `script` job thread's stack: `(unused, total)`, after the deep recursion.
    pub script_stack: (usize, usize),
}

/// The line the QEMU image logs after its script phase: `recipes: script motion-light <KB> KB
/// source; psram +<KB> KB (peak +<KB> KB); internal +<KB> KB (min <KB> KB free); engine stack <n>
/// of <n> unused; script thread stack <n> of <n> unused; heap peak <KB> KB` (stacks in bytes).
pub fn script_phase_line(id: &str, phase: &ScriptPhase) -> String {
    format!(
        "recipes: script {id} {} KB source; psram +{} KB (peak +{} KB); internal +{} KB (min {} KB free); engine stack {} of {} unused; script thread stack {} of {} unused; heap peak {} KB",
        kib(phase.source),
        kib(phase.psram_used),
        kib(phase.psram_peak),
        kib(phase.internal_used),
        kib(phase.internal_min),
        phase.engine_stack.0,
        phase.engine_stack.1,
        phase.script_stack.0,
        phase.script_stack.1,
        kib(phase.heap_peak),
    )
}

// ---- What the recipe installs read (spec 042 R4, R5) ----

/// The memory as the platform reads it now, with the lowest each memory has been since boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryReading {
    pub stats: MemoryStats,
    /// The lowest internal RAM free since boot.
    pub internal_min: usize,
    /// The lowest PSRAM free since boot.
    pub psram_min: usize,
}

/// Reads the heap for the admission of a recipe install (R4.4): the firmware reads ESP-IDF's, the
/// host reads none (its admission always passes), a test gives its own figures.
pub trait MemoryProbe: Send + Sync {
    /// `None`: this platform reads no memory.
    fn read(&self) -> Option<MemoryReading>;
}

/// The probe of a platform that reads no memory (the Mac).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoProbe;

impl MemoryProbe for NoProbe {
    fn read(&self) -> Option<MemoryReading> {
        None
    }
}

/// The internal RAM of `GET /system/resources`, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalResources {
    pub free: usize,
    pub min_free: usize,
    pub largest_block: usize,
}

/// The PSRAM of `GET /system/resources`, in bytes; `reserve` is what an install leaves free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PsramResources {
    pub free: usize,
    pub min_free: usize,
    pub total: usize,
    pub reserve: usize,
}

/// The storage of `GET /system/resources`, in bytes: what the files use, and the partition's size
/// when the platform knows it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct StorageResources {
    pub used: u64,
    pub total: Option<u64>,
}

/// One script recipe of `GET /system/resources`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptResources {
    pub id: String,
    pub version: Option<String>,
    /// `builtIn` (given by `--recipe-script`) or `installed`.
    pub source: &'static str,
    /// `ready`, `failed` or `refused`.
    pub state: &'static str,
    pub reason: Option<String>,
    pub held: usize,
    pub limit: usize,
    pub instances: usize,
    pub calls: u64,
    pub average_us: u64,
    pub max_us: u64,
}

/// The answer of `GET /api/v1/system/resources` (a Solite route).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resources {
    pub internal: Option<InternalResources>,
    pub psram: Option<PsramResources>,
    pub storage: StorageResources,
    pub scripts: Vec<ScriptResources>,
    /// The recipe catalogue's last read (moved here from the store's answer, spec 059 R2).
    pub catalogue: CatalogueResources,
}

/// When the recipe catalogue was last read, and why the last read failed, if it did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogueResources {
    pub read_at: Option<String>,
    pub error: Option<String>,
}
