use crate::atomic::{self, Files, RealFiles};
use crate::envelope;
use crate::error::{Corruption, Operation, Record, StoreError};
use locsim_core::domain::Scenario;
use locsim_scenario::{export_scenario, import_scenario, ScenarioError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The records kept in one directory.
///
/// # Records
///
/// | Record | File | Payload |
/// |---|---|---|
/// | Scenario | `scenario.locsim` | the document `export_scenario` writes |
///
/// Each file is an envelope (see the crate documentation) around its
/// payload. The directory is the caller's: `open` creates nothing, and no
/// path is built in.
///
/// # Saving
///
/// The value is validated and encoded in memory first; an invalid value
/// never reaches the disk. The file is then replaced by the sequence
/// described under "Replacing a record" in the crate documentation, so a
/// failed save leaves the previous record.
///
/// # Loading
///
/// Loading never writes, repairs, moves or deletes anything.
///
/// - No file: `Ok(None)`. Absence is not an error and not corruption.
/// - A file that cannot be loaded: `Err(StoreError::Corrupt)`, saying why.
///   There is no fallback to a default and no older copy to fall back to.
/// - Otherwise the record, validated exactly as on import.
///
/// # Concurrency
///
/// Saves through one `Store` are serialised. More than one writer per
/// directory (two `Store`s, or two processes) is **not supported**: each
/// file would still be a complete record, but which save wins is not
/// defined, and different records could come from different writers.
/// Readers may load at any time; on Windows a load that coincides with a
/// replacement may fail with an I/O error and can be repeated.
pub struct Store {
    directory: PathBuf,
    files: Box<dyn Files>,
    /// Held for the whole of a save, and while temporaries are removed.
    writing: Mutex<()>,
}

impl Store {
    /// Uses an existing directory. Nothing is created, read or changed.
    pub fn open(directory: impl Into<PathBuf>) -> Result<Store, StoreError> {
        Self::with_files(directory.into(), Box::new(RealFiles))
    }

    pub(crate) fn with_files(
        directory: PathBuf,
        files: Box<dyn Files>,
    ) -> Result<Store, StoreError> {
        match std::fs::metadata(&directory) {
            Ok(metadata) if metadata.is_dir() => Ok(Store {
                directory,
                files,
                writing: Mutex::new(()),
            }),
            Ok(_) => Err(StoreError::NotADirectory { path: directory }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Err(StoreError::NotADirectory { path: directory })
            }
            Err(e) => Err(StoreError::io(Operation::OpenDirectory, &directory, &e)),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Where a record is, or would be, stored.
    pub fn path_of(&self, record: Record) -> PathBuf {
        self.directory.join(record.file_name())
    }

    /// Stores a scenario, replacing the stored one. An invalid scenario is
    /// refused and nothing is written.
    pub fn save_scenario(&self, scenario: &Scenario) -> Result<(), StoreError> {
        let text = export_scenario(scenario).map_err(|errors| StoreError::Invalid {
            record: Record::Scenario,
            errors,
        })?;
        self.save(Record::Scenario, &text)
    }

    /// The stored scenario, or `None` if none is stored.
    pub fn load_scenario(&self) -> Result<Option<Scenario>, StoreError> {
        self.load(Record::Scenario, import_scenario)
    }

    fn save(&self, record: Record, text: &str) -> Result<(), StoreError> {
        // A panic while saving cannot leave shared state half-changed (the
        // guarded value is empty), so a poisoned lock is still usable.
        let _writing = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        atomic::replace(
            self.files.as_ref(),
            &self.directory,
            record.file_name(),
            &envelope::seal(text.as_bytes()),
        )
    }

    fn load<T>(
        &self,
        record: Record,
        import: fn(&str) -> Result<T, Vec<ScenarioError>>,
    ) -> Result<Option<T>, StoreError> {
        let path = self.path_of(record);
        let bytes = match self.files.read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(StoreError::io(Operation::Read, &path, &e)),
        };
        interpret(record, &path, &bytes, import).map(Some)
    }

    /// Temporary files left in the directory, by an interrupted save of
    /// this or an earlier process. They are never read as records.
    ///
    /// While a save is in progress its own temporary file is listed too.
    pub fn stale_temporaries(&self) -> Result<Vec<PathBuf>, StoreError> {
        let list = |e: &io::Error| StoreError::io(Operation::List, &self.directory, e);
        let prefixes: Vec<String> = [Record::Scenario]
            .iter()
            .map(|r| atomic::temporary_prefix(r.file_name()))
            .collect();
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&self.directory).map_err(|e| list(&e))? {
            let entry = entry.map_err(|e| list(&e))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if prefixes.iter().any(|p| name.starts_with(p.as_str())) {
                found.push(entry.path());
            }
        }
        found.sort();
        Ok(found)
    }

    /// Removes the files [`Store::stale_temporaries`] lists and returns how
    /// many there were. Never called automatically.
    ///
    /// It waits for a save in progress on this `Store`. It must not be
    /// called while another process is saving into the same directory: that
    /// process's temporary file would be removed and its save would fail.
    pub fn remove_stale_temporaries(&self) -> Result<usize, StoreError> {
        let _writing = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        let stale = self.stale_temporaries()?;
        for path in &stale {
            self.files
                .remove(path)
                .map_err(|e| StoreError::io(Operation::RemoveTemporary, path, &e))?;
        }
        Ok(stale.len())
    }
}

/// Turns the bytes of a stored file into its record, or says why not.
fn interpret<T>(
    record: Record,
    path: &Path,
    bytes: &[u8],
    import: fn(&str) -> Result<T, Vec<ScenarioError>>,
) -> Result<T, StoreError> {
    let corrupt = |cause| StoreError::Corrupt {
        record,
        path: path.to_path_buf(),
        cause,
    };
    // The digest is verified before the payload is interpreted at all.
    let payload = envelope::open(bytes).map_err(|e| corrupt(Corruption::Envelope(e)))?;
    let text = std::str::from_utf8(payload).map_err(|_| corrupt(Corruption::NotUtf8))?;
    import(text).map_err(|errors| corrupt(Corruption::Document(errors)))
}

#[cfg(test)]
mod tests;
