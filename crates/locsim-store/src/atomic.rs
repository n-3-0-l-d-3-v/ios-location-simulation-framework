//! Replacing a file so that a reader finds either the old content or the
//! new, and a failure leaves the old.
//!
//! # Sequence
//!
//! 1. Create a temporary file **in the same directory** (so the rename in
//!    step 5 never crosses a file system), with a name no record uses, and
//!    fail rather than reuse an existing one.
//! 2. Write the bytes.
//! 3. Sync the file, then close it.
//! 4. Read the file back and compare it with what was meant.
//! 5. Rename it over the destination.
//! 6. Sync the directory, where the platform allows.
//!
//! The destination is not opened, truncated or written at any point; step 5
//! is the only one that touches it. If any of steps 1–5 fails, the
//! destination is what it was and the temporary file is removed. If step 6
//! fails, the destination is already the new content; that is reported as
//! its own case (`StoreError::record_was_replaced`).
//!
//! # What is and is not guaranteed
//!
//! **Visibility** (what a concurrent reader can see):
//!
//! - Unix-like systems: `rename(2)` replaces the destination atomically; a
//!   reader finds the old file or the new one, never neither and never a
//!   mixture. This is specified by POSIX.
//! - Windows: the standard library's rename is one call to `MoveFileExW`
//!   with `MOVEFILE_REPLACE_EXISTING` (falling back to a POSIX-semantics
//!   rename if that is denied). Microsoft does not document the replacement
//!   as atomic, so this crate does not claim it. The destination is never
//!   written in place, so a reader cannot find a half-written record; what
//!   is not promised is that the name always resolves during the call. A
//!   reader may get an error and can retry.
//!
//! **Durability** (what survives a crash or power loss):
//!
//! - The file's content is synced before the rename (`File::sync_all`:
//!   `fsync`, `F_FULLFSYNC` on Apple systems, `FlushFileBuffers` on
//!   Windows), so the rename never makes a file visible whose content is
//!   still only in memory.
//! - Unix-like systems: the directory is synced after the rename.
//! - Windows: a directory cannot be synced through the standard library,
//!   and its rename does not request write-through. After a successful
//!   save, a power loss may therefore leave the *old* record in place. The
//!   file system's own journal decides; this crate does not.
//!
//! None of the durability statements has been tested with a real power
//! loss. They describe what is requested of the operating system.

use crate::error::{Operation, StoreError};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Whether this build syncs the directory after replacing a record. Where
/// it is `false`, a successful save does not include a request to make the
/// change of name durable.
pub const DIRECTORY_SYNC: bool = cfg!(unix);

/// How many temporary names are tried before giving up.
const NAME_ATTEMPTS: u32 = 16;

/// The file-system steps of a replacement, one method each, so that tests
/// can make any single one fail.
pub(crate) trait Files: Send + Sync {
    /// A suffix that makes a temporary name unique among live writers.
    fn temporary_suffix(&self) -> String;
    /// Creates `path` for writing; fails if it exists.
    fn create_new(&self, path: &Path) -> io::Result<File>;
    fn write_all(&self, file: &mut File, bytes: &[u8]) -> io::Result<()>;
    fn sync(&self, file: &File) -> io::Result<()>;
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn sync_directory(&self, directory: &Path) -> io::Result<()>;
    fn remove(&self, path: &Path) -> io::Result<()>;
}

/// The real file system.
pub(crate) struct RealFiles;

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

impl Files for RealFiles {
    fn temporary_suffix(&self) -> String {
        // The process id separates processes; the counter separates the
        // writes of one process. No randomness and no clock.
        let n = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        format!("{}-{n}", std::process::id())
    }

    fn create_new(&self, path: &Path) -> io::Result<File> {
        OpenOptions::new().write(true).create_new(true).open(path)
    }

    fn write_all(&self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
        file.write_all(bytes)
    }

    fn sync(&self, file: &File) -> io::Result<()> {
        file.sync_all()
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    #[cfg(unix)]
    fn sync_directory(&self, directory: &Path) -> io::Result<()> {
        File::open(directory)?.sync_all()
    }

    #[cfg(not(unix))]
    fn sync_directory(&self, _directory: &Path) -> io::Result<()> {
        // Not available through the standard library here; see
        // `DIRECTORY_SYNC` and the module documentation.
        Ok(())
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }
}

/// The part of a temporary file's name that comes before the suffix.
pub(crate) fn temporary_prefix(name: &str) -> String {
    format!(".{name}.tmp-")
}

fn create_temporary(
    files: &dyn Files,
    directory: &Path,
    name: &str,
) -> Result<(File, PathBuf), StoreError> {
    let mut path = directory.to_path_buf();
    for _ in 0..NAME_ATTEMPTS {
        path = directory.join(format!(
            "{}{}",
            temporary_prefix(name),
            files.temporary_suffix()
        ));
        match files.create_new(&path) {
            Ok(file) => return Ok((file, path)),
            // Somebody's leftover, or a live writer's file: not ours to
            // touch. Try another name.
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(StoreError::io(Operation::CreateTemporary, &path, &e)),
        }
    }
    Err(StoreError::TemporaryCollision { path })
}

/// Removes the temporary file after a failure and records in `error`
/// whether that worked.
fn abandon(files: &dyn Files, temporary: &Path, mut error: StoreError) -> StoreError {
    if let Err(e) = files.remove(temporary) {
        let reason = Some(format!("{} ({e})", temporary.display()));
        match &mut error {
            StoreError::Io { cleanup, .. } | StoreError::VerifyMismatch { cleanup, .. } => {
                *cleanup = reason
            }
            _ => {}
        }
    }
    error
}

/// Makes `directory/name` hold exactly `bytes`, by the sequence in the
/// module documentation.
pub(crate) fn replace(
    files: &dyn Files,
    directory: &Path,
    name: &str,
    bytes: &[u8],
) -> Result<(), StoreError> {
    let destination = directory.join(name);
    let (mut file, temporary) = create_temporary(files, directory, name)?;

    let written = files
        .write_all(&mut file, bytes)
        .map_err(|e| StoreError::io(Operation::Write, &temporary, &e))
        .and_then(|()| {
            files
                .sync(&file)
                .map_err(|e| StoreError::io(Operation::Sync, &temporary, &e))
        });
    // Closed before it is read back, renamed or removed.
    drop(file);
    if let Err(error) = written {
        return Err(abandon(files, &temporary, error));
    }

    match files.read(&temporary) {
        Ok(read) if read == bytes => {}
        Ok(_) => {
            let error = StoreError::VerifyMismatch {
                path: temporary.clone(),
                cleanup: None,
            };
            return Err(abandon(files, &temporary, error));
        }
        Err(e) => {
            let error = StoreError::io(Operation::ReadBack, &temporary, &e);
            return Err(abandon(files, &temporary, error));
        }
    }

    if let Err(e) = files.rename(&temporary, &destination) {
        let error = StoreError::io(Operation::Replace, &destination, &e);
        return Err(abandon(files, &temporary, error));
    }

    // The record is replaced. A failure from here on must say so.
    files
        .sync_directory(directory)
        .map_err(|e| StoreError::io(Operation::SyncDirectory, directory, &e))
}

#[cfg(test)]
pub(crate) mod testing {
    //! A scratch directory and a file system that fails on request.

    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    /// A fresh directory under the system's temporary directory, removed
    /// when dropped.
    pub(crate) struct Scratch(pub(crate) PathBuf);

    impl Scratch {
        pub(crate) fn new(label: &str) -> Self {
            let n = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "locsim-store-test-{}-{n}-{label}",
                std::process::id()
            ));
            std::fs::create_dir(&path).expect("fresh scratch directory");
            Scratch(path)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }

        /// File names in the directory, sorted.
        pub(crate) fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The step at which a [`Faulty`] file system fails.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Fault {
        None,
        Create,
        /// Half the bytes reach the file, then the write fails (a full disk).
        WritePartial,
        Sync,
        ReadBack,
        /// The file reads back with one byte changed.
        ReadBackDiffers,
        /// The file reads back one byte short.
        ReadBackShort,
        Rename,
        SyncDirectory,
    }

    pub(crate) const FAULTS_BEFORE_REPLACEMENT: [Fault; 7] = [
        Fault::Create,
        Fault::WritePartial,
        Fault::Sync,
        Fault::ReadBack,
        Fault::ReadBackDiffers,
        Fault::ReadBackShort,
        Fault::Rename,
    ];

    /// The real file system, except that one chosen step fails, and
    /// temporary names are predictable (`t0`, `t1`, …).
    pub(crate) struct Faulty {
        pub(crate) fault: Mutex<Fault>,
        pub(crate) remove_fails: AtomicBool,
        next_name: AtomicU64,
    }

    impl Faulty {
        pub(crate) fn new(fault: Fault) -> Self {
            Self {
                fault: Mutex::new(fault),
                remove_fails: AtomicBool::new(false),
                next_name: AtomicU64::new(0),
            }
        }

        fn fails(&self, fault: Fault) -> io::Result<()> {
            if *self.fault.lock().unwrap() == fault {
                Err(io::Error::other(format!("injected: {fault:?}")))
            } else {
                Ok(())
            }
        }
    }

    impl Files for Faulty {
        fn temporary_suffix(&self) -> String {
            format!("t{}", self.next_name.fetch_add(1, Ordering::Relaxed))
        }

        fn create_new(&self, path: &Path) -> io::Result<File> {
            self.fails(Fault::Create)?;
            RealFiles.create_new(path)
        }

        fn write_all(&self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
            if *self.fault.lock().unwrap() == Fault::WritePartial {
                RealFiles.write_all(file, &bytes[..bytes.len() / 2])?;
                return Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "injected: disk full",
                ));
            }
            RealFiles.write_all(file, bytes)
        }

        fn sync(&self, file: &File) -> io::Result<()> {
            self.fails(Fault::Sync)?;
            RealFiles.sync(file)
        }

        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.fails(Fault::ReadBack)?;
            let mut bytes = RealFiles.read(path)?;
            match *self.fault.lock().unwrap() {
                Fault::ReadBackDiffers => {
                    let middle = bytes.len() / 2;
                    bytes[middle] ^= 0x01;
                }
                Fault::ReadBackShort => {
                    bytes.pop();
                }
                _ => {}
            }
            Ok(bytes)
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.fails(Fault::Rename)?;
            RealFiles.rename(from, to)
        }

        fn sync_directory(&self, directory: &Path) -> io::Result<()> {
            self.fails(Fault::SyncDirectory)?;
            RealFiles.sync_directory(directory)
        }

        fn remove(&self, path: &Path) -> io::Result<()> {
            if self.remove_fails.load(Ordering::Relaxed) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected: remove refused",
                ));
            }
            RealFiles.remove(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    const NAME: &str = "record.locsim";
    const OLD: &[u8] = b"the old record\n";
    const NEW: &[u8] = b"the new record, somewhat longer than the old one\n";

    fn read(scratch: &Scratch) -> Option<Vec<u8>> {
        match std::fs::read(scratch.path().join(NAME)) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => panic!("{e}"),
        }
    }

    fn expected_operation(fault: Fault) -> Option<Operation> {
        match fault {
            Fault::Create => Some(Operation::CreateTemporary),
            Fault::WritePartial => Some(Operation::Write),
            Fault::Sync => Some(Operation::Sync),
            Fault::ReadBack => Some(Operation::ReadBack),
            Fault::Rename => Some(Operation::Replace),
            Fault::SyncDirectory => Some(Operation::SyncDirectory),
            Fault::ReadBackDiffers | Fault::ReadBackShort | Fault::None => None,
        }
    }

    fn assert_is(error: &StoreError, fault: Fault) {
        match (error, expected_operation(fault)) {
            (StoreError::Io { operation, .. }, Some(expected)) => assert_eq!(*operation, expected),
            (StoreError::VerifyMismatch { .. }, None) => {}
            other => panic!("{fault:?}: {other:?}"),
        }
    }

    #[test]
    fn creates_a_record_and_replaces_it_leaving_nothing_else_behind() {
        let scratch = Scratch::new("replace");
        replace(&RealFiles, scratch.path(), NAME, OLD).unwrap();
        assert_eq!(read(&scratch).as_deref(), Some(OLD));
        replace(&RealFiles, scratch.path(), NAME, NEW).unwrap();
        assert_eq!(read(&scratch).as_deref(), Some(NEW));
        // Shorter content replaces longer: nothing of the old file remains.
        replace(&RealFiles, scratch.path(), NAME, b"").unwrap();
        assert_eq!(read(&scratch).as_deref(), Some(&b""[..]));
        assert_eq!(scratch.names(), [NAME]);
    }

    #[test]
    fn a_failure_at_any_step_before_the_replacement_keeps_the_old_record() {
        for fault in FAULTS_BEFORE_REPLACEMENT {
            let scratch = Scratch::new("keep-old");
            replace(&RealFiles, scratch.path(), NAME, OLD).unwrap();
            let error = replace(&Faulty::new(fault), scratch.path(), NAME, NEW).unwrap_err();
            assert_is(&error, fault);
            assert!(!error.record_was_replaced(), "{fault:?}");
            assert_eq!(read(&scratch).as_deref(), Some(OLD), "{fault:?}");
            // And the temporary file is gone.
            assert_eq!(scratch.names(), [NAME], "{fault:?}");
            match &error {
                StoreError::Io { cleanup, .. } | StoreError::VerifyMismatch { cleanup, .. } => {
                    assert_eq!(*cleanup, None, "{fault:?}")
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_failure_at_any_step_before_the_replacement_creates_no_record() {
        for fault in FAULTS_BEFORE_REPLACEMENT {
            let scratch = Scratch::new("no-record");
            let error = replace(&Faulty::new(fault), scratch.path(), NAME, NEW).unwrap_err();
            assert_is(&error, fault);
            assert_eq!(read(&scratch), None, "{fault:?}");
            assert!(
                scratch.names().is_empty(),
                "{fault:?}: {:?}",
                scratch.names()
            );
        }
    }

    #[test]
    fn a_partial_write_is_reported_as_the_disk_error_it_was() {
        let scratch = Scratch::new("partial");
        let error = replace(&Faulty::new(Fault::WritePartial), scratch.path(), NAME, NEW);
        match error.unwrap_err() {
            StoreError::Io {
                operation: Operation::Write,
                kind: io::ErrorKind::StorageFull,
                path,
                ..
            } => assert!(path.ends_with(".record.locsim.tmp-t0"), "{path:?}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_directory_sync_failure_says_the_record_was_replaced() {
        let scratch = Scratch::new("dir-sync");
        replace(&RealFiles, scratch.path(), NAME, OLD).unwrap();
        let error = replace(
            &Faulty::new(Fault::SyncDirectory),
            scratch.path(),
            NAME,
            NEW,
        )
        .unwrap_err();
        assert_is(&error, Fault::SyncDirectory);
        assert!(error.record_was_replaced());
        assert!(error
            .to_string()
            .contains("in place but not confirmed durable"));
        assert_eq!(read(&scratch).as_deref(), Some(NEW));
        assert_eq!(scratch.names(), [NAME]);
    }

    #[test]
    fn a_temporary_file_that_cannot_be_removed_is_reported_with_the_first_error() {
        for fault in FAULTS_BEFORE_REPLACEMENT {
            if fault == Fault::Create {
                continue; // nothing was created, so nothing is left
            }
            let scratch = Scratch::new("cleanup");
            replace(&RealFiles, scratch.path(), NAME, OLD).unwrap();
            let files = Faulty::new(fault);
            files.remove_fails.store(true, Ordering::Relaxed);
            let error = replace(&files, scratch.path(), NAME, NEW).unwrap_err();
            // Still the original failure, with the leftover named beside it.
            assert_is(&error, fault);
            match &error {
                StoreError::Io { cleanup, .. } | StoreError::VerifyMismatch { cleanup, .. } => {
                    let reason = cleanup.as_deref().expect("cleanup failure recorded");
                    assert!(reason.contains(".record.locsim.tmp-t0"), "{reason}");
                    assert!(reason.contains("injected: remove refused"), "{reason}");
                }
                other => panic!("{other:?}"),
            }
            assert!(error.to_string().contains("could not be removed"));
            assert_eq!(read(&scratch).as_deref(), Some(OLD), "{fault:?}");
            assert_eq!(
                scratch.names(),
                [".record.locsim.tmp-t0", NAME],
                "{fault:?}"
            );
        }
    }

    #[test]
    fn an_existing_temporary_name_is_skipped_and_never_touched() {
        let scratch = Scratch::new("collision");
        for n in 0..3 {
            std::fs::write(
                scratch.path().join(format!(".record.locsim.tmp-t{n}")),
                b"someone else's",
            )
            .unwrap();
        }
        replace(&Faulty::new(Fault::None), scratch.path(), NAME, NEW).unwrap();
        assert_eq!(read(&scratch).as_deref(), Some(NEW));
        assert_eq!(
            scratch.names(),
            [
                ".record.locsim.tmp-t0",
                ".record.locsim.tmp-t1",
                ".record.locsim.tmp-t2",
                NAME
            ]
        );
        for n in 0..3 {
            let other = scratch.path().join(format!(".record.locsim.tmp-t{n}"));
            assert_eq!(std::fs::read(other).unwrap(), b"someone else's");
        }
    }

    #[test]
    fn running_out_of_temporary_names_is_an_error_not_an_overwrite() {
        let scratch = Scratch::new("exhausted");
        replace(&RealFiles, scratch.path(), NAME, OLD).unwrap();
        for n in 0..NAME_ATTEMPTS {
            std::fs::write(
                scratch.path().join(format!(".record.locsim.tmp-t{n}")),
                b"x",
            )
            .unwrap();
        }
        let error = replace(&Faulty::new(Fault::None), scratch.path(), NAME, NEW).unwrap_err();
        match error {
            StoreError::TemporaryCollision { path } => {
                assert!(path.ends_with(".record.locsim.tmp-t15"), "{path:?}")
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(read(&scratch).as_deref(), Some(OLD));
        assert_eq!(scratch.names().len(), NAME_ATTEMPTS as usize + 1);
    }

    #[test]
    fn real_temporary_names_do_not_repeat_within_a_process() {
        let a = RealFiles.temporary_suffix();
        let b = RealFiles.temporary_suffix();
        assert_ne!(a, b);
        assert!(a.starts_with(&format!("{}-", std::process::id())));
    }

    #[test]
    fn a_missing_directory_is_an_error_at_the_first_step() {
        let scratch = Scratch::new("missing");
        let gone = scratch.path().join("not-here");
        match replace(&RealFiles, &gone, NAME, NEW).unwrap_err() {
            StoreError::Io {
                operation: Operation::CreateTemporary,
                kind: io::ErrorKind::NotFound,
                ..
            } => {}
            other => panic!("{other:?}"),
        }
        assert!(scratch.names().is_empty());
    }
}
