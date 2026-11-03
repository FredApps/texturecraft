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
//! have an estimated compressed size under [`IN_FLIGHT_BUDGET`] (a single entry larger than that runs
//! alone), and entry bodies stream into the compressor instead of building a raw copy first.

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
    pub weight: u64,
    pub body: Body<'a>,
}

/// Compressed bytes (estimated) that may be held in memory at once while entries are in flight.
#[cfg(not(target_arch = "wasm32"))]
const IN_FLIGHT_BUDGET: u64 = 3 << 30;
/// More workers than this do not help: a project has a handful of large entries.
#[cfg(not(target_arch = "wasm32"))]
const MAX_WORKERS: usize = 8;

/// Write `entries` in order: in parallel where threads exist, else one after the other.
pub(super) fn write_entries<W: Write + Seek>(zip: &mut ZipWriter<W>, entries: Vec<Entry<'_>>) -> Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Leave one core to the UI: an autosave runs while the user keeps painting.
        let cpus = std::thread::available_parallelism().map_or(1, usize::from);
        let workers = cpus.saturating_sub(1).clamp(1, MAX_WORKERS).min(entries.len());
        if workers > 1 {
            return parallel::write(zip, entries, workers, IN_FLIGHT_BUDGET);
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
            for _ in 0..workers {
                s.spawn(|| worker(&slots, &weights, budget, &state, &cv));
            }
            let res = (|| -> Result<()> {
                for (i, w) in weights.iter().enumerate() {
                    let done = {
                        let mut st = lock(&state);
                        loop {
                            if let Some(r) = st.ready.remove(&i) {
                                st.in_flight = st.in_flight.saturating_sub(*w);
                                cv.notify_all();
                                break r;
                            }
                            st = wait(&cv, st);
                        }
                    };
                    copy_entry(zip, done?)?;
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
