//! The allocator of the script recipes' QuickJS runtimes (feature `scripts`, spec 041 R3.4, D74)
//! and the firmware's `ScriptRuntimes`: every QuickJS block goes to PSRAM (`heap_caps_*` with
//! `MALLOC_CAP_SPIRAM`), where ESP-IDF's default `malloc` would keep every block of 512 bytes or less
//! (`CONFIG_SPIRAM_MALLOC_ALWAYSINTERNAL`) in the internal RAM, the tight resource.
//!
//! ESP-IDF's libc has no `malloc_usable_size`, so QuickJS's own accounting (what its memory limit
//! counts) would see every block as 0 bytes: the allocator reports `heap_caps_get_allocated_size`,
//! the size the heap really gave. QuickJS is given no limit of its own (`quickjs_limit` is 0): the
//! allocator holds the runtime to `SCRIPT_MEMORY_LIMIT` itself, and every refusal, whether the
//! script caught the out-of-memory error or not, sets the runtime's own flag (one per runtime),
//! which the engine reads after each call and clears before each call and each load (R3.5).
//!
//! This is the firmware's only `unsafe` for scripts (D11: none in the core): `unsafe` here is the
//! calls into ESP-IDF's heap and the allocator trait itself, each with its `SAFETY` comment.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use esp_idf_svc::sys::{
    MALLOC_CAP_8BIT, MALLOC_CAP_SPIRAM, free, heap_caps_calloc, heap_caps_get_allocated_size,
    heap_caps_malloc, heap_caps_realloc,
};
use rquickjs::Runtime;
use rquickjs::allocator::Allocator;
use solite_core::recipes::script::{HeldCounter, NewRuntime, SCRIPT_MEMORY_LIMIT, ScriptRuntimes};

/// The capabilities of every block: PSRAM, byte-addressable.
const CAPS: u32 = MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT;

/// The size of the block `ptr` points to, as the heap gave it (at least what was asked).
///
/// # Safety
///
/// `ptr` is null or a live block from `heap_caps_malloc`, `heap_caps_calloc` or
/// `heap_caps_realloc` below, not yet freed: the heap reads the header in front of the block.
#[allow(unsafe_code)]
unsafe fn block_size(ptr: *mut u8) -> usize {
    if ptr.is_null() {
        return 0;
    }
    // SAFETY: the caller's contract (a live block of this heap, non-null checked above); the
    // function only reads the heap's header of the block.
    unsafe { heap_caps_get_allocated_size(ptr.cast()) }
}

/// One runtime's allocator: PSRAM blocks, counted with their real size, refused above `limit`.
/// The count is shared with the engine, which reads it after each call (QuickJS's own accounting
/// walks the whole heap, in PSRAM: slow).
pub struct PsramAllocator {
    used: HeldCounter,
    limit: usize,
    refused: Arc<AtomicBool>,
}

impl PsramAllocator {
    fn new(limit: usize, refused: Arc<AtomicBool>, used: HeldCounter) -> Self {
        PsramAllocator {
            used,
            limit,
            refused,
        }
    }

    fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    fn set_used(&self, bytes: usize) {
        self.used.store(bytes, Ordering::Relaxed);
    }

    /// Records a refusal in the runtime's own flag.
    fn refuse(&self) {
        self.refused.store(true, Ordering::SeqCst);
    }
}

// SAFETY: every block comes from `heap_caps_malloc`/`heap_caps_calloc`/`heap_caps_realloc` and goes
// back through `free`, which ESP-IDF's heap accepts for any of them; those return null or a block
// of at least the size asked; `usable_size` is the heap's own size of the block, which is the
// memory available in it. Alignment: `heap_caps_malloc` aligns to 4 bytes, not the 8 of QuickJS's
// 64-bit values (its NaN boxing, on for 32-bit builds, makes a `JSValue` a `uint64_t`); harmless on
// Xtensa: it has no 64-bit load or store instruction, the compiler reads and writes such a value as
// two 32-bit accesses, which need 4-byte alignment only. (`heap_caps_aligned_alloc` has no
// `realloc` twin: moving to it would copy every grown block by hand, for nothing.)
#[allow(unsafe_code)]
unsafe impl Allocator for PsramAllocator {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        if self.used().saturating_add(size) > self.limit {
            self.refuse();
            return std::ptr::null_mut();
        }
        // SAFETY: plain allocation call; the result is checked for null before use.
        let ptr = unsafe { heap_caps_malloc(size, CAPS) }.cast::<u8>();
        if ptr.is_null() {
            self.refuse();
        } else {
            // SAFETY: `ptr` is the live non-null block `heap_caps_malloc` just returned.
            self.set_used(self.used() + unsafe { block_size(ptr) });
        }
        ptr
    }

    fn calloc(&mut self, count: usize, size: usize) -> *mut u8 {
        let total = count.saturating_mul(size);
        if self.used().saturating_add(total) > self.limit {
            self.refuse();
            return std::ptr::null_mut();
        }
        // SAFETY: plain allocation call; the result is checked for null before use.
        let ptr = unsafe { heap_caps_calloc(count, size, CAPS) }.cast::<u8>();
        if ptr.is_null() {
            self.refuse();
        } else {
            // SAFETY: `ptr` is the live non-null block `heap_caps_calloc` just returned.
            self.set_used(self.used() + unsafe { block_size(ptr) });
        }
        ptr
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8) {
        // SAFETY: the trait's contract: `ptr` is a live block of this allocator, freed just below.
        self.set_used(self.used().saturating_sub(unsafe { block_size(ptr) }));
        // SAFETY: the caller gives a block of this allocator (the trait's contract), which came
        // from the heap_caps functions and is freed once.
        unsafe { free(ptr.cast()) };
    }

    unsafe fn realloc(&mut self, ptr: *mut u8, new_size: usize) -> *mut u8 {
        // SAFETY: the trait's contract: `ptr` is null or a live block of this allocator.
        let old = unsafe { block_size(ptr) };
        if new_size > old && self.used().saturating_sub(old).saturating_add(new_size) > self.limit {
            // The old block stays as it is, as `realloc` does when it fails.
            self.refuse();
            return std::ptr::null_mut();
        }
        // SAFETY: `ptr` is null or a block of this allocator (the trait's contract); on a null
        // result the old block is untouched, which is handled below.
        let next = unsafe { heap_caps_realloc(ptr.cast(), new_size, CAPS) }.cast::<u8>();
        if next.is_null() {
            self.refuse();
            return next;
        }
        // SAFETY: `next` is the live non-null block `heap_caps_realloc` just returned.
        self.set_used(self.used().saturating_sub(old) + unsafe { block_size(next) });
        next
    }

    unsafe fn usable_size(ptr: *mut u8) -> usize {
        // SAFETY: the trait's contract: `ptr` is a live block of an allocator of this kind.
        unsafe { block_size(ptr) }
    }
}

/// The firmware's runtimes: QuickJS over `PsramAllocator`, each with its own refusal flag.
pub struct EspRuntimes;

impl ScriptRuntimes for EspRuntimes {
    fn new_runtime(&self) -> Result<NewRuntime, String> {
        let refused = Arc::new(AtomicBool::new(false));
        let held: HeldCounter = Arc::new(AtomicUsize::new(0));
        let runtime = Runtime::new_with_alloc(PsramAllocator::new(
            SCRIPT_MEMORY_LIMIT,
            Arc::clone(&refused),
            Arc::clone(&held),
        ))
        .map_err(|e| e.to_string())?;
        Ok(NewRuntime {
            runtime,
            refused: Some(refused),
            held: Some(held),
        })
    }

    fn memory_limit(&self) -> usize {
        SCRIPT_MEMORY_LIMIT
    }

    /// QuickJS holds no limit of its own: its count and the allocator's differ by the blocks'
    /// headers, and a request QuickJS refused itself would never reach the allocator, so the flag
    /// would stay unset. The allocator is the one that refuses, and says so.
    fn quickjs_limit(&self) -> usize {
        0
    }
}

/// The runtimes of the engine study (spec 057, the QEMU image only): the same allocator with a
/// limit that holds several recipes in one runtime (the layouts the study measures); never the
/// engine's own, so no recipe is ever held to it.
#[cfg(feature = "sim")]
pub struct StudyRuntimes;

#[cfg(feature = "sim")]
impl ScriptRuntimes for StudyRuntimes {
    fn new_runtime(&self) -> Result<NewRuntime, String> {
        let refused = Arc::new(AtomicBool::new(false));
        let held: HeldCounter = Arc::new(AtomicUsize::new(0));
        let runtime = Runtime::new_with_alloc(PsramAllocator::new(
            1536 * 1024,
            Arc::clone(&refused),
            Arc::clone(&held),
        ))
        .map_err(|e| e.to_string())?;
        Ok(NewRuntime {
            runtime,
            refused: Some(refused),
            held: Some(held),
        })
    }

    fn quickjs_limit(&self) -> usize {
        0
    }
}
