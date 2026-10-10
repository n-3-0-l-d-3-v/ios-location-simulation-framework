//! Shared by the store's integration tests.
#![allow(dead_code)]

/// Examples, the provider driver and the core's stream re-check, as the
/// scenario crate's tests use them.
#[path = "../../../locsim-scenario/tests/support/mod.rs"]
pub mod scenario;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A fresh directory under the system's temporary directory, removed when
/// dropped.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(label: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "locsim-store-it-{}-{n}-{label}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("fresh scratch directory");
        Scratch(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// File names in the directory, sorted.
    pub fn names(&self) -> Vec<String> {
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

/// The payload of a stored file: everything after the four header lines.
pub fn payload_of(file: &[u8]) -> &[u8] {
    let mut rest = file;
    for _ in 0..4 {
        let end = rest.iter().position(|b| *b == b'\n').expect("header line");
        rest = &rest[end + 1..];
    }
    rest
}
