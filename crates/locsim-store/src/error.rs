use crate::envelope::EnvelopeError;
use locsim_scenario::ScenarioError;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Which stored record an operation concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Record {
    Scenario,
    LastKnown,
}

impl Record {
    /// The record's file name inside the store directory.
    pub fn file_name(self) -> &'static str {
        match self {
            Record::Scenario => "scenario.locsim",
            Record::LastKnown => "last_known.locsim",
        }
    }
}

impl fmt::Display for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Record::Scenario => "scenario",
            Record::LastKnown => "last-known record",
        })
    }
}

/// The file-system step that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    /// Looking at the store directory.
    OpenDirectory,
    /// Creating the temporary file a record is first written to.
    CreateTemporary,
    /// Writing the record into the temporary file.
    Write,
    /// Asking the operating system to put the temporary file on disk.
    Sync,
    /// Reading the temporary file back to compare it with what was meant.
    ReadBack,
    /// Renaming the temporary file over the record.
    Replace,
    /// Asking the operating system to put the directory entry on disk,
    /// after the record was replaced.
    SyncDirectory,
    /// Reading a record.
    Read,
    /// Listing the store directory.
    List,
    /// Removing a temporary file.
    RemoveTemporary,
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Operation::OpenDirectory => "open the store directory",
            Operation::CreateTemporary => "create the temporary file",
            Operation::Write => "write the temporary file",
            Operation::Sync => "sync the temporary file",
            Operation::ReadBack => "read the temporary file back",
            Operation::Replace => "replace the record",
            Operation::SyncDirectory => "sync the store directory",
            Operation::Read => "read the record",
            Operation::List => "list the store directory",
            Operation::RemoveTemporary => "remove a temporary file",
        })
    }
}

/// Why a file that exists could not be loaded as a record. The file is
/// left exactly as it was found.
#[derive(Debug, Clone, PartialEq)]
pub enum Corruption {
    /// The file is not an intact envelope: wrong kind of file, unsupported
    /// envelope version, damaged header, wrong length or wrong digest.
    Envelope(EnvelopeError),
    /// The payload is intact but is not UTF-8 text.
    NotUtf8,
    /// The payload is intact but its document is refused: malformed,
    /// an unsupported document version, or an invalid scenario or sample.
    Document(Vec<ScenarioError>),
}

impl fmt::Display for Corruption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Corruption::Envelope(e) => write!(f, "{e}"),
            Corruption::NotUtf8 => write!(f, "the payload is not UTF-8 text"),
            Corruption::Document(errors) => {
                write!(f, "the stored document is refused:")?;
                for e in errors {
                    write!(f, " [{e}]")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StoreError {
    /// The store location does not exist or is not a directory.
    NotADirectory { path: PathBuf },
    /// A file-system step failed.
    ///
    /// For every operation except [`Operation::SyncDirectory`] the record on
    /// disk is what it was before the call. `cleanup` is set when the
    /// temporary file could not be removed afterwards; it then still exists
    /// (see `Store::stale_temporaries`).
    Io {
        operation: Operation,
        path: PathBuf,
        kind: io::ErrorKind,
        os_code: Option<i32>,
        message: String,
        cleanup: Option<String>,
    },
    /// The value to save is not valid. Nothing was written.
    Invalid {
        record: Record,
        errors: Vec<ScenarioError>,
    },
    /// The file exists but does not hold a loadable record. It was not
    /// changed, moved or deleted.
    Corrupt {
        record: Record,
        path: PathBuf,
        cause: Corruption,
    },
    /// The temporary file, read back, was not what had been written. The
    /// record was not replaced.
    VerifyMismatch {
        path: PathBuf,
        cleanup: Option<String>,
    },
    /// No unused temporary file name was found. The record was not
    /// replaced. Old temporary files are in the way; see
    /// `Store::remove_stale_temporaries`.
    TemporaryCollision { path: PathBuf },
}

impl StoreError {
    pub(crate) fn io(operation: Operation, path: &Path, error: &io::Error) -> Self {
        StoreError::Io {
            operation,
            path: path.to_path_buf(),
            kind: error.kind(),
            os_code: error.raw_os_error(),
            message: error.to_string(),
            cleanup: None,
        }
    }

    /// Whether the new record is in place despite the error. True only when
    /// the replacement itself succeeded and the step after it, syncing the
    /// directory, failed: the new record is visible, but the operating
    /// system did not confirm that the change of name is on disk.
    pub fn record_was_replaced(&self) -> bool {
        matches!(
            self,
            StoreError::Io {
                operation: Operation::SyncDirectory,
                ..
            }
        )
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cleanup = |f: &mut fmt::Formatter<'_>, cleanup: &Option<String>| match cleanup {
            Some(reason) => write!(f, "; the temporary file could not be removed: {reason}"),
            None => Ok(()),
        };
        match self {
            StoreError::NotADirectory { path } => {
                write!(f, "{} is not an existing directory", path.display())
            }
            StoreError::Io {
                operation,
                path,
                message,
                cleanup: left,
                ..
            } => {
                write!(f, "could not {operation} ({}): {message}", path.display())?;
                if self.record_was_replaced() {
                    write!(f, "; the new record is in place but not confirmed durable")?;
                }
                cleanup(f, left)
            }
            StoreError::Invalid { record, errors } => {
                write!(f, "the {record} is not valid and was not saved:")?;
                for e in errors {
                    write!(f, " [{e}]")?;
                }
                Ok(())
            }
            StoreError::Corrupt {
                record,
                path,
                cause,
            } => write!(
                f,
                "the stored {record} ({}) cannot be loaded: {cause}",
                path.display()
            ),
            StoreError::VerifyMismatch {
                path,
                cleanup: left,
            } => {
                write!(
                    f,
                    "{} did not read back as written; the record was not replaced",
                    path.display()
                )?;
                cleanup(f, left)
            }
            StoreError::TemporaryCollision { path } => write!(
                f,
                "no free temporary file name (last tried {})",
                path.display()
            ),
        }
    }
}

impl std::error::Error for StoreError {}
