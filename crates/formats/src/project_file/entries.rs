//! Writing the zip entries of a project file.
//!
//! Compressing the baked maps dominates a save (deflate level 6 over gigabytes of `f32`), and every
//! entry is independent, so on native targets the entries are compressed on several threads. Each
//! worker builds a one-entry zip in memory with the same options the serial writer uses; the writing
//! thread copies those entries into the real file in the original order with
//! [`ZipWriter::raw_copy_file`]. The compressed bytes of every entry and the entry order are
//! therefore the same as in a serial save.
//!
//! Memory stays bounded: a worker only starts an entry while the entries started but not yet written
//! have an estimated compressed size under the RAM share the user allows (a single entry larger than
//! that runs alone), and entry bodies stream into the compressor instead of building a raw copy first.

use std::io::{Seek, Write};

use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use super::zip_err;
use crate::Result;

/// Produces the uncompressed bytes of one entry into the zip writer.
pub(super) type Body<'a> = Box<dyn FnOnce(&mut dyn Write) -> std::io::Result<()> + Send + 'a>;

pub(super) struct Entry<'a> {
    pub name: String,
    pub options: SimpleFileOptions,
    /// Upper bound of the compressed size in bytes (the uncompressed size is a safe choice).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub weight: u64,
    pub body: Body<'a>,
}

/// How much of the machine a save may use. Native targets only; the browser saves on one thread.
#[cfg(not(target_arch = "wasm32"))]
mod limits {
    use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

    /// Cores a save leaves to the UI (an autosave runs while the user keeps painting).
    static RESERVED_CORES: AtomicUsize = AtomicUsize::new(DEFAULT_RESERVED_CORES);
    /// Share of the available RAM, in percent, that entries in flight may hold.
    static RAM_PERCENT: AtomicU8 = AtomicU8::new(DEFAULT_RAM_PERCENT);

    pub const DEFAULT_RESERVED_CORES: usize = 1;
    pub const DEFAULT_RAM_PERCENT: u8 = 50;

    pub fn set(reserved_cores: usize, ram_percent: u8) {
        RESERVED_CORES.store(reserved_cores, Ordering::Relaxed);
        RAM_PERCENT.store(ram_percent.clamp(1, 100), Ordering::Relaxed);
    }

    pub fn get() -> (usize, u8) {
        (RESERVED_CORES.load(Ordering::Relaxed), RAM_PERCENT.load(Ordering::Relaxed))
    }

    /// RAM the system can give out now, in bytes (the total when the system does not say it; 8 GiB
    /// when the system tells nothing).
    pub fn available_ram() -> u64 {
        let Ok(mut sys) = sysinfo::System::new() else { return 8 << 30 };
        sys.refresh_memory();
        match sys.available_memory() {
            0 => sys.total_memory(),
            n => n,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use limits::{DEFAULT_RAM_PERCENT, DEFAULT_RESERVED_CORES};

/// Set what saves may use: cores left to the UI (any number; at least one worker always runs) and
/// the percentage (1 to 100) of the available RAM that entries in flight may hold.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_save_resources(reserved_cores: usize, ram_percent: u8) {
    limits::set(reserved_cores, ram_percent);
}

/// Worker threads and in-flight byte budget for a save of `entries` entries.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn plan(cpus: usize, reserved_cores: usize, entries: usize, available_ram: u64, ram_percent: u8) -> (usize, u64) {
    let workers = cpus.saturating_sub(reserved_cores).max(1).min(entries);
    let budget = u64::try_from(u128::from(available_ram) * u128::from(ram_percent.clamp(1, 100)) / 100).unwrap_or(u64::MAX);
    (workers, budget)
}

/// Write `entries` in order: in parallel where threads exist, else one after the other.
pub(super) fn write_entries<W: Write + Seek>(zip: &mut ZipWriter<W>, entries: Vec<Entry<'_>>) -> Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let cpus = std::thread::available_parallelism().map_or(1, usize::from);
        let (reserved, percent) = limits::get();
        let (workers, budget) = plan(cpus, reserved, entries.len(), limits::available_ram(), percent);
        if workers > 1 {
            return parallel::write(zip, entries, workers, budget);
        }
    }
    write_serial(zip, entries)
}

/// One entry after the other straight into `zip`.
pub(super) fn write_serial<W: Write + Seek>(zip: &mut ZipWriter<W>, entries: Vec<Entry<'_>>) -> Result<()> {
    for e in entries {
        zip.start_file(e.name, e.options).map_err(zip_err)?;
        (e.body)(zip)?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) mod parallel {
    use std::collections::BTreeMap;
    use std::io::{Cursor, Seek, Write};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{Condvar, Mutex, MutexGuard};

    use zip::{ZipArchive, ZipWriter};

    use super::Entry;
    use crate::project_file::zip_err;
    use crate::{FormatError, Result};

    struct State {
        /// Next entry to start.
        next: usize,
        /// Estimated bytes of the entries started and not yet written.
        in_flight: u64,
        /// Finished entries (one-entry zips) waiting for their turn.
        ready: BTreeMap<usize, Result<Vec<u8>>>,
        /// The writer failed: start nothing more.
        stop: bool,
    }

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wait<'a>(cv: &Condvar, g: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        cv.wait(g).unwrap_or_else(|e| e.into_inner())
    }

    /// Compress one entry into a zip of its own.
    fn compress(e: Entry<'_>) -> Result<Vec<u8>> {
        let mut z = ZipWriter::new(Cursor::new(Vec::new()));
        z.start_file(e.name, e.options).map_err(zip_err)?;
        (e.body)(&mut z)?;
        Ok(z.finish().map_err(zip_err)?.into_inner())
    }

    fn worker(slots: &[Mutex<Option<Entry<'_>>>], weights: &[u64], budget: u64, state: &Mutex<State>, cv: &Condvar) {
        loop {
            let i = {
                let mut st = lock(state);
                loop {
                    if st.stop || st.next >= slots.len() {
                        return;
                    }
                    let w = weights.get(st.next).copied().unwrap_or(0);
                    // The first entry in line always starts, however large.
                    if st.in_flight == 0 || st.in_flight.saturating_add(w) <= budget {
                        let i = st.next;
                        st.next += 1;
                        st.in_flight = st.in_flight.saturating_add(w);
                        break i;
                    }
                    st = wait(cv, st);
                }
            };
            let entry = slots.get(i).and_then(|s| lock(s).take());
            let res = match entry {
                Some(e) => {
                    catch_unwind(AssertUnwindSafe(|| compress(e))).unwrap_or_else(|_| Err(FormatError::Invalid("compressing a project entry failed".into())))
                }
                None => Err(FormatError::Invalid("project entry missing".into())),
            };
            lock(state).ready.insert(i, res);
            cv.notify_all();
        }
    }

    /// Copy the compressed entry of a one-entry zip into `zip` without recompressing.
    fn copy_entry<W: Write + Seek>(zip: &mut ZipWriter<W>, bytes: Vec<u8>) -> Result<()> {
        let mut a = ZipArchive::new(Cursor::new(bytes)).map_err(zip_err)?;
        let f = a.by_index_raw(0).map_err(zip_err)?;
        zip.raw_copy_file(f).map_err(zip_err)
    }

    pub(in crate::project_file) fn write<W: Write + Seek>(zip: &mut ZipWriter<W>, entries: Vec<Entry<'_>>, workers: usize, budget: u64) -> Result<()> {
        let weights: Vec<u64> = entries.iter().map(|e| e.weight).collect();
        let slots: Vec<Mutex<Option<Entry<'_>>>> = entries.into_iter().map(|e| Mutex::new(Some(e))).collect();
        let state = Mutex::new(State { next: 0, in_flight: 0, ready: BTreeMap::new(), stop: false });
        let cv = Condvar::new();
        std::thread::scope(|s| {
            let mut started = 0;
            for _ in 0..workers {
                // Thread creation can fail (process limits); the workers that did start drain the queue.
                if std::thread::Builder::new().spawn_scoped(s, || worker(&slots, &weights, budget, &state, &cv)).is_ok() {
                    started += 1;
                }
            }
            let res = (|| -> Result<()> {
                for (i, w) in weights.iter().enumerate() {
                    if started == 0 {
                        // No thread at all: stream the entry into the file like the serial writer does.
                        let entry = slots.get(i).and_then(|s| lock(s).take()).ok_or_else(|| FormatError::Invalid("project entry missing".into()))?;
                        zip.start_file(entry.name, entry.options).map_err(zip_err)?;
                        (entry.body)(zip)?;
                        continue;
                    }
                    let done = {
                        let mut st = lock(&state);
                        loop {
                            if let Some(r) = st.ready.remove(&i) {
                                break r;
                            }
                            st = wait(&cv, st);
                        }
                    };
                    let copied = done.and_then(|bytes| copy_entry(zip, bytes));
                    // Release the entry's share only once its bytes are gone.
                    {
                        let mut st = lock(&state);
                        st.in_flight = st.in_flight.saturating_sub(*w);
                    }
                    cv.notify_all();
                    copied?;
                }
                Ok(())
            })();
            if res.is_err() {
                lock(&state).stop = true;
                cv.notify_all();
            }
            res
        })
    }
}
