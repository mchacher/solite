//! Spec 035, the fix of 2026-11-25: on the box, `GET /api/v1/backup` answered 200 then the
//! ESP32 stopped on `assert failed: spi_flash_disable_interrupts_caches_and_other_cpu
//! cache_utils.c:127 (esp_task_stack_is_sane_cache_disabled())`. A flash access (a read too: it
//! turns the caches off) came from an API worker, whose stack is in PSRAM. The box runs the
//! backup's jobs on a thread whose stack is in internal RAM and hands the core its files behind
//! `Guarded`. ESP-IDF's assert is reproduced here by `FlashFiles`: a call made while the thread
//! is marked "stack in PSRAM" panics with ESP-IDF's message, as the box does.

mod support;

use std::cell::Cell;
use std::io::{self, Cursor, Read, Write};
use std::rc::Rc;

use solite_core::backup::{BackupFiles, Guarded, Phase, StagedFile, WRONG_STACK, apply_pending};
use solite_core::home::Home;
use support::backup_fake::{Clock, MemFiles};

const PASSWORD: &str = "correct%20horse%20battery";
const ITER: u32 = 1000;
const ESP_IDF_ASSERT: &str = "assert failed: spi_flash_disable_interrupts_caches_and_other_cpu \
                              cache_utils.c:127 (esp_task_stack_is_sane_cache_disabled())";

thread_local! {
    /// True while the test runs as an API worker of the box: a stack in PSRAM.
    static PSRAM_STACK: Cell<bool> = const { Cell::new(false) };
}

fn internal_stack() -> bool {
    !PSRAM_STACK.with(Cell::get)
}

/// Runs `job` as an API worker would: the stack in PSRAM.
fn on_a_worker<T>(job: impl FnOnce() -> T) -> T {
    PSRAM_STACK.with(|flag| flag.set(true));
    let result = job();
    PSRAM_STACK.with(|flag| flag.set(false));
    result
}

/// The data partition as the ESP32 sees it: every access turns the caches off, which ESP-IDF
/// refuses from a stack in PSRAM. Counts the accesses.
#[derive(Clone)]
struct FlashFiles {
    inner: MemFiles,
    touches: Rc<Cell<usize>>,
}

fn touch(touches: &Rc<Cell<usize>>) {
    assert!(internal_stack(), "{ESP_IDF_ASSERT}");
    touches.set(touches.get() + 1);
}

struct FlashRead {
    inner: Box<dyn Read>,
    touches: Rc<Cell<usize>>,
}

impl Read for FlashRead {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        touch(&self.touches);
        self.inner.read(buffer)
    }
}

struct FlashWrite {
    inner: Box<dyn StagedFile>,
    touches: Rc<Cell<usize>>,
}

impl Write for FlashWrite {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        touch(&self.touches);
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        touch(&self.touches);
        self.inner.flush()
    }
}

impl StagedFile for FlashWrite {
    fn finish(self: Box<Self>) -> Result<(), String> {
        touch(&self.touches);
        self.inner.finish()
    }
}

impl FlashFiles {
    fn new(inner: MemFiles) -> Self {
        FlashFiles {
            inner,
            touches: Rc::default(),
        }
    }

    fn touch(&self) {
        touch(&self.touches);
    }

    fn reader(&self, inner: Option<Box<dyn Read>>) -> Option<Box<dyn Read>> {
        let touches = Rc::clone(&self.touches);
        inner.map(|inner| Box::new(FlashRead { inner, touches }) as Box<dyn Read>)
    }
}

impl BackupFiles for FlashFiles {
    fn has_wifi(&self) -> bool {
        self.inner.has_wifi()
    }
    fn restarts(&self) -> bool {
        self.inner.restarts()
    }
    fn open(&mut self, name: &str) -> Result<Option<Box<dyn Read>>, String> {
        self.touch();
        let inner = self.inner.open(name)?;
        Ok(self.reader(inner))
    }
    fn stage(&mut self, name: &str) -> Result<Box<dyn StagedFile>, String> {
        self.touch();
        let inner = self.inner.stage(name)?;
        Ok(Box::new(FlashWrite {
            inner,
            touches: Rc::clone(&self.touches),
        }))
    }
    fn open_staged(&mut self, name: &str) -> Result<Option<Box<dyn Read>>, String> {
        self.touch();
        let inner = self.inner.open_staged(name)?;
        Ok(self.reader(inner))
    }
    fn discard(&mut self) -> Result<(), String> {
        self.touch();
        self.inner.discard()
    }
    fn commit(&mut self, marker: &str) -> Result<(), String> {
        self.touch();
        self.inner.commit(marker)
    }
    fn pending(&mut self) -> Result<Option<String>, String> {
        self.touch();
        self.inner.pending()
    }
    fn promote(&mut self, name: &str) -> Result<(), String> {
        self.touch();
        self.inner.promote(name)
    }
    fn remove(&mut self, name: &str) -> Result<(), String> {
        self.touch();
        self.inner.remove(name)
    }
    fn set_aside(&mut self, name: &str, label: &str) -> Result<(), String> {
        self.touch();
        self.inner.set_aside(name, label)
    }
    fn clear_pending(&mut self) -> Result<(), String> {
        self.touch();
        self.inner.clear_pending()
    }
    fn restore_mark(&mut self) -> Result<bool, String> {
        self.touch();
        self.inner.restore_mark()
    }
    fn set_restore_mark(&mut self) -> Result<(), String> {
        self.touch();
        self.inner.set_restore_mark()
    }
    fn clear_restore_mark(&mut self) -> Result<(), String> {
        self.touch();
        self.inner.clear_restore_mark()
    }
}

/// What the box hands the core (`firmware/src/backup_files.rs`): its files behind the guard.
fn box_files(flash: &FlashFiles) -> Guarded<FlashFiles> {
    Guarded::new(flash.clone(), internal_stack)
}

fn a_box() -> FlashFiles {
    let files = MemFiles::with(&[
        ("devices.json", "[{\"id\":\"d\"}]"),
        ("zones.json", "[{\"id\":\"z\"}]"),
        ("settings.json", "{\"home.name\":\"Annex\"}"),
        (
            "wifi.json",
            "{\"ssid\":\"Home\",\"password\":\"a wifi password\"}",
        ),
    ]);
    FlashFiles::new(files)
}

fn random() -> impl FnMut(&mut [u8]) -> Result<(), String> {
    let mut n = 0u8;
    move |buffer: &mut [u8]| {
        for byte in buffer.iter_mut() {
            n = n.wrapping_add(37);
            *byte = n;
        }
        Ok(())
    }
}

/// The export as the box runs it: the bytes, or the stream's error.
fn export(flash: &FlashFiles, home: &Home) -> io::Result<Vec<u8>> {
    let ticket = home.backup_begin(false).unwrap();
    let Ok(mut reader) = ticket.export_with_iterations(
        Some(PASSWORD),
        Box::new(box_files(flash)),
        Box::new(Clock::new()),
        &mut random(),
        ITER,
    ) else {
        panic!("the export was refused before its stream");
    };
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map(|_| bytes)
}

fn restore(flash: &FlashFiles, home: &Home, archive: &[u8]) -> u16 {
    let ticket = home.backup_begin(true).unwrap();
    ticket
        .restore(
            true,
            Some(archive.len() as u64),
            Some(PASSWORD),
            &mut Cursor::new(archive),
            &mut box_files(flash),
            &mut Clock::new(),
        )
        .status
}

#[test]
fn an_export_on_a_worker_reads_no_file_and_ends_its_stream_with_an_error() {
    let flash = a_box();
    let home = Home::new(0);
    let result = on_a_worker(|| export(&flash, &home));
    let error = result.expect_err("the stream ends with an error");
    assert!(error.to_string().contains(WRONG_STACK), "{error}");
    assert_eq!(
        flash.touches.get(),
        0,
        "the flash was touched from a PSRAM stack"
    );
    assert_eq!(home.backup_state().phase(), Phase::Idle, "the job ended");
}

#[test]
fn a_restore_on_a_worker_stages_nothing_and_answers_500() {
    let flash = a_box();
    let archive = export(&flash, &Home::new(0)).unwrap();
    let target = a_box();
    let before = target.inner.snapshot();
    let home = Home::new(0);
    let status = on_a_worker(|| restore(&target, &home, &archive));
    assert_eq!(status, 500);
    assert_eq!(
        target.touches.get(),
        0,
        "the flash was touched from a PSRAM stack"
    );
    assert_eq!(target.inner.snapshot(), before, "a file changed");
    assert_eq!(home.backup_state().phase(), Phase::Idle, "the job ended");
}

#[test]
fn finishing_a_restore_on_a_worker_touches_no_file() {
    let flash = a_box();
    let result = on_a_worker(|| apply_pending(&mut box_files(&flash)));
    assert!(result.is_err());
    assert_eq!(
        flash.touches.get(),
        0,
        "the flash was touched from a PSRAM stack"
    );
}

#[test]
fn on_a_stack_in_internal_ram_the_export_and_the_restore_work_through_the_guard() {
    let flash = a_box();
    let archive = export(&flash, &Home::new(0)).unwrap();
    assert!(flash.touches.get() > 0);
    assert_eq!(&archive[..8], b"SOLITEBK");
    let target = FlashFiles::new(MemFiles::new());
    assert_eq!(restore(&target, &Home::new(0), &archive), 200);
    apply_pending(&mut box_files(&target)).unwrap();
    assert_eq!(
        target.inner.text("zones.json").as_deref(),
        Some("[{\"id\":\"z\"}]")
    );
    assert_eq!(
        target.inner.text("wifi.json"),
        flash.inner.text("wifi.json"),
        "the encrypted file round-trips"
    );
}
