//! T09: how replacement behaves on the operating system the tests run on.
//!
//! The Windows tests were run (Windows 11, NTFS) and state what was
//! observed there. The Unix tests are written from the documented behaviour
//! of `rename(2)` and **have not been run**: no Unix-like system was
//! available. Until they are, the Unix column of the crate documentation is
//! a description of intent, not a result.

mod support;

use locsim_core::domain::Scenario;
use locsim_scenario::import_scenario;
use locsim_store::{Operation, Store, StoreError};
use support::scenario::example;
use support::Scratch;

fn scenario(name: &str) -> Scenario {
    import_scenario(example(name)).unwrap()
}

/// A store holding the walking scenario.
fn stored(label: &str) -> (Scratch, Store) {
    let scratch = Scratch::new(label);
    let store = Store::open(scratch.path()).unwrap();
    store.save_scenario(&scenario("walking")).unwrap();
    (scratch, store)
}

#[test]
fn the_build_says_whether_the_directory_is_synced() {
    assert_eq!(locsim_store::DIRECTORY_SYNC, cfg!(unix));
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::fs::OpenOptions;
    use std::io::Read;
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;

    /// Another program has the record open and does not allow it to be
    /// deleted or renamed over (the default of most Windows programs).
    #[test]
    fn a_record_held_open_without_delete_sharing_cannot_be_replaced_and_is_kept() {
        let (scratch, store) = stored("win-held");
        let before = std::fs::read(scratch.file("scenario.locsim")).unwrap();
        let holder = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(scratch.file("scenario.locsim"))
            .unwrap();

        match store.save_scenario(&scenario("driving")) {
            Err(StoreError::Io {
                operation: Operation::Replace,
                path,
                cleanup: None,
                os_code,
                ..
            }) => {
                assert!(path.ends_with("scenario.locsim"));
                // ERROR_ACCESS_DENIED (5) or ERROR_SHARING_VIOLATION (32).
                assert!(matches!(os_code, Some(5 | 32)), "{os_code:?}");
            }
            other => panic!("{other:?}"),
        }
        // The old record is intact and the temporary file is gone.
        assert_eq!(
            std::fs::read(scratch.file("scenario.locsim")).unwrap(),
            before
        );
        assert_eq!(scratch.names(), ["scenario.locsim"]);
        assert_eq!(store.load_scenario(), Ok(Some(scenario("walking"))));

        // Once the other program lets go, the same save succeeds.
        drop(holder);
        store.save_scenario(&scenario("driving")).unwrap();
        assert_eq!(store.load_scenario(), Ok(Some(scenario("driving"))));
    }

    /// A reader that opened the record the way the standard library does
    /// (sharing everything) does not block a replacement, and goes on
    /// reading the record it opened.
    #[test]
    fn a_reader_with_the_record_open_keeps_the_old_record_while_it_is_replaced() {
        let (scratch, store) = stored("win-reader");
        let old = std::fs::read(scratch.file("scenario.locsim")).unwrap();
        let mut reader = std::fs::File::open(scratch.file("scenario.locsim")).unwrap();
        let mut first_half = vec![0; old.len() / 2];
        reader.read_exact(&mut first_half).unwrap();

        store.save_scenario(&scenario("driving")).unwrap();
        assert_eq!(store.load_scenario(), Ok(Some(scenario("driving"))));

        let mut rest = Vec::new();
        reader.read_to_end(&mut rest).unwrap();
        first_half.extend(rest);
        assert_eq!(first_half, old, "the open handle saw a mixture");
        assert_eq!(scratch.names(), ["scenario.locsim"]);
    }

    /// A record marked read-only cannot be replaced on Windows: the rename
    /// over it is refused with "access is denied" (unlike Unix, where only
    /// the directory's permissions matter). The save fails cleanly and the
    /// record is kept. Observed on Windows 11 with Rust 1.98.1.
    #[test]
    fn a_read_only_record_is_not_replaced_and_is_kept() {
        let (scratch, store) = stored("win-readonly");
        let path = scratch.file("scenario.locsim");
        let before = std::fs::read(&path).unwrap();
        let set_read_only = |read_only: bool| {
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_readonly(read_only);
            std::fs::set_permissions(&path, permissions).unwrap();
        };
        set_read_only(true);

        match store.save_scenario(&scenario("driving")) {
            Err(StoreError::Io {
                operation: Operation::Replace,
                kind: std::io::ErrorKind::PermissionDenied,
                os_code: Some(5),
                cleanup: None,
                ..
            }) => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(scratch.names(), ["scenario.locsim"]);
        assert_eq!(store.load_scenario(), Ok(Some(scenario("walking"))));

        // With the attribute cleared the save goes through.
        set_read_only(false);
        store.save_scenario(&scenario("driving")).unwrap();
        assert_eq!(store.load_scenario(), Ok(Some(scenario("driving"))));
    }
}

/// NOT RUN. Written against POSIX semantics; see the top of this file.
#[cfg(unix)]
mod unix {
    use super::*;
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_reader_with_the_record_open_keeps_the_old_record_while_it_is_replaced() {
        let (scratch, store) = stored("unix-reader");
        let old = std::fs::read(scratch.file("scenario.locsim")).unwrap();
        let mut reader = std::fs::File::open(scratch.file("scenario.locsim")).unwrap();
        store.save_scenario(&scenario("driving")).unwrap();
        assert_eq!(store.load_scenario(), Ok(Some(scenario("driving"))));
        let mut seen = Vec::new();
        reader.read_to_end(&mut seen).unwrap();
        assert_eq!(seen, old);
        assert_eq!(scratch.names(), ["scenario.locsim"]);
    }

    #[test]
    fn a_read_only_record_in_a_writable_directory_is_replaced() {
        // rename(2) needs write permission on the directory, not the file.
        let (scratch, store) = stored("unix-readonly-file");
        let path = scratch.file("scenario.locsim");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        store.save_scenario(&scenario("driving")).unwrap();
        assert_eq!(store.load_scenario(), Ok(Some(scenario("driving"))));
    }

    #[test]
    fn a_directory_that_cannot_be_written_refuses_the_save_and_keeps_the_record() {
        let (scratch, store) = stored("unix-readonly-dir");
        let before = std::fs::read(scratch.file("scenario.locsim")).unwrap();
        std::fs::set_permissions(scratch.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = store.save_scenario(&scenario("driving"));
        std::fs::set_permissions(scratch.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        // A privileged user is not stopped by the permission bits.
        if result.is_ok() {
            return;
        }
        match result {
            Err(StoreError::Io {
                operation: Operation::CreateTemporary,
                kind: std::io::ErrorKind::PermissionDenied,
                ..
            }) => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(
            std::fs::read(scratch.file("scenario.locsim")).unwrap(),
            before
        );
        assert_eq!(scratch.names(), ["scenario.locsim"]);
    }
}
