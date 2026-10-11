use super::*;
use crate::hooks::{arm, take_session_drop, DropConditions, Point};
use crate::{locsim_abi_version, LOCSIM_ABI_VERSION};
use std::ptr::{null, null_mut};
use std::sync::mpsc;
use std::sync::{Barrier, RwLock, RwLockReadGuard, RwLockWriteGuard};

const FIXED: &str = include_str!("../../../../Examples/Scenarios/fixed.json");
const EXAMPLES: [&str; 8] = [
    include_str!("../../../../Examples/Scenarios/antimeridian_loop.json"),
    include_str!("../../../../Examples/Scenarios/circular.json"),
    include_str!("../../../../Examples/Scenarios/driving.json"),
    FIXED,
    include_str!("../../../../Examples/Scenarios/high_latitude.json"),
    include_str!("../../../../Examples/Scenarios/random_walk.json"),
    include_str!("../../../../Examples/Scenarios/route_replay.json"),
    include_str!("../../../../Examples/Scenarios/walking.json"),
];

// The registry is one per process and the tests run side by side. A test
// that needs to know exactly what the registry holds takes `alone`; every
// other test that creates a session takes `shared`.
static GLOBAL: RwLock<()> = RwLock::new(());

fn shared() -> RwLockReadGuard<'static, ()> {
    GLOBAL.read().unwrap_or_else(PoisonError::into_inner)
}

fn alone() -> RwLockWriteGuard<'static, ()> {
    GLOBAL.write().unwrap_or_else(PoisonError::into_inner)
}

fn live() -> usize {
    registry().live()
}

fn policy() -> LocsimPolicy {
    LocsimPolicy {
        struct_size: 64,
        max_recovery_attempts: 3,
        backoff_initial_s: 1.0,
        backoff_multiplier: 2.0,
        backoff_max_s: 60.0,
        stable_after_s: 10.0,
        stall_after_s: 5.0,
        missed_window_ticks: 10,
        max_missed_in_window: 2,
    }
}

/// A policy with two things wrong with it.
fn bad_policy() -> LocsimPolicy {
    LocsimPolicy {
        backoff_multiplier: f64::NAN,
        missed_window_ticks: 0,
        ..policy()
    }
}

// What the outputs hold before a call. Still there afterwards means the
// call did not write.
const SESSION_BEFORE: u64 = 0xDEAD_BEEF_DEAD_BEEF;
const LEN_BEFORE: usize = 0x5A5A_5A5A;
const BYTE_BEFORE: u8 = 0xA5;

struct Outcome {
    status: LocsimStatus,
    session: u64,
    err_len: usize,
    buf: Vec<u8>,
}

impl Outcome {
    fn text(&self) -> &str {
        std::str::from_utf8(&self.buf[..self.err_len]).unwrap()
    }

    fn buf_untouched(&self) -> bool {
        self.buf.iter().all(|byte| *byte == BYTE_BEFORE)
    }

    /// The row of the output table for every failure that has no text.
    fn assert_failed_without_text(&self, status: LocsimStatus) {
        assert_eq!(self.status, status);
        assert_eq!(self.session, 0);
        assert_eq!(self.err_len, 0);
        assert!(self.buf_untouched());
    }
}

/// Calls `locsim_session_create` with a buffer of `buf_len` bytes, of which
/// it is told `err_cap`.
fn create_raw(
    json: *const u8,
    json_len: usize,
    policy: *const LocsimPolicy,
    event_capacity: u32,
    buf_len: usize,
    err_cap: usize,
) -> Outcome {
    assert!(err_cap <= buf_len);
    let mut session = SESSION_BEFORE;
    let mut err_len = LEN_BEFORE;
    let mut buf = vec![BYTE_BEFORE; buf_len];
    // SAFETY: every pointer is to a live local or to the caller's slice,
    // with the lengths given, except where a test passes a length that the
    // function must refuse before reading.
    let status = unsafe {
        locsim_session_create(
            json,
            json_len,
            policy,
            event_capacity,
            &mut session,
            buf.as_mut_ptr(),
            err_cap,
            &mut err_len,
        )
    };
    Outcome {
        status,
        session,
        err_len,
        buf,
    }
}

fn create(json: &[u8], policy: &LocsimPolicy) -> Outcome {
    create_raw(json.as_ptr(), json.len(), policy, 16, 8192, 8192)
}

fn open() -> u64 {
    let made = create(FIXED.as_bytes(), &policy());
    assert_eq!(made.status, LOCSIM_OK);
    made.session
}

#[test]
fn the_version_is_the_constant() {
    assert_eq!(locsim_abi_version(), 1);
    assert_eq!(LOCSIM_ABI_VERSION, 1);
}

#[test]
fn the_policy_struct_has_the_documented_layout() {
    let value = policy();
    let base = std::ptr::addr_of!(value) as usize;
    let offset = |field: usize| field - base;
    assert_eq!(std::mem::size_of::<LocsimPolicy>(), 64);
    assert_eq!(std::mem::align_of::<LocsimPolicy>(), 8);
    assert_eq!(offset(std::ptr::addr_of!(value.struct_size) as usize), 0);
    assert_eq!(
        offset(std::ptr::addr_of!(value.max_recovery_attempts) as usize),
        4
    );
    assert_eq!(
        offset(std::ptr::addr_of!(value.backoff_initial_s) as usize),
        8
    );
    assert_eq!(
        offset(std::ptr::addr_of!(value.backoff_multiplier) as usize),
        16
    );
    assert_eq!(offset(std::ptr::addr_of!(value.backoff_max_s) as usize), 24);
    assert_eq!(
        offset(std::ptr::addr_of!(value.stable_after_s) as usize),
        32
    );
    assert_eq!(offset(std::ptr::addr_of!(value.stall_after_s) as usize), 40);
    assert_eq!(
        offset(std::ptr::addr_of!(value.missed_window_ticks) as usize),
        48
    );
    assert_eq!(
        offset(std::ptr::addr_of!(value.max_missed_in_window) as usize),
        56
    );
}

// --- Creation that succeeds ---------------------------------------------

#[test]
fn every_example_makes_a_session_that_can_be_destroyed_once() {
    let _guard = shared();
    let mut handles = Vec::new();
    for example in EXAMPLES {
        let made = create(example.as_bytes(), &policy());
        assert_eq!(made.status, LOCSIM_OK);
        assert_ne!(made.session, 0);
        assert_ne!(made.session, SESSION_BEFORE);
        assert_eq!(made.err_len, 0);
        assert!(made.buf_untouched());
        handles.push(made.session);
    }
    let mut distinct = handles.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), EXAMPLES.len());
    for handle in handles {
        assert_eq!(locsim_session_destroy(handle), LOCSIM_OK);
        assert_eq!(locsim_session_destroy(handle), LOCSIM_ERR_INVALID_HANDLE);
    }
}

#[test]
fn the_event_capacity_must_be_between_one_and_the_maximum() {
    let _guard = shared();
    let json = FIXED.as_bytes();
    for capacity in [0, LOCSIM_EVENT_CAPACITY_MAX + 1, u32::MAX] {
        create_raw(json.as_ptr(), json.len(), &policy(), capacity, 64, 64)
            .assert_failed_without_text(LOCSIM_ERR_INVALID_ARGUMENT);
    }
    for capacity in [1, LOCSIM_EVENT_CAPACITY_MAX] {
        let made = create_raw(json.as_ptr(), json.len(), &policy(), capacity, 64, 64);
        assert_eq!(made.status, LOCSIM_OK);
        assert_eq!(locsim_session_destroy(made.session), LOCSIM_OK);
    }
}

#[test]
fn a_document_of_exactly_the_maximum_length_is_read_and_one_byte_more_is_not() {
    let _guard = shared();
    // JSON allows white space after the value.
    let mut json = FIXED.as_bytes().to_vec();
    json.resize(LOCSIM_SCENARIO_MAX_BYTES, b' ');
    let made = create(&json, &policy());
    assert_eq!(made.status, LOCSIM_OK);
    assert_eq!(locsim_session_destroy(made.session), LOCSIM_OK);

    // Refused by its length alone: the extra byte does not exist.
    create_raw(json.as_ptr(), json.len() + 1, &policy(), 16, 64, 64)
        .assert_failed_without_text(LOCSIM_ERR_INVALID_ARGUMENT);
    create_raw(json.as_ptr(), usize::MAX, &policy(), 16, 64, 64)
        .assert_failed_without_text(LOCSIM_ERR_INVALID_ARGUMENT);
}

// --- Null pointers ------------------------------------------------------

#[test]
fn a_null_pointer_is_refused_and_nothing_at_all_is_written() {
    let json = FIXED.as_bytes();
    let policy = policy();
    for missing in [
        "scenario",
        "policy",
        "out_session",
        "err_buf",
        "out_err_len",
    ] {
        let mut session = SESSION_BEFORE;
        let mut err_len = LEN_BEFORE;
        let mut buf = vec![BYTE_BEFORE; 64];
        // SAFETY: every pointer that is not null is to a live local with
        // the length given.
        let status = unsafe {
            locsim_session_create(
                if missing == "scenario" {
                    null()
                } else {
                    json.as_ptr()
                },
                json.len(),
                if missing == "policy" { null() } else { &policy },
                16,
                if missing == "out_session" {
                    null_mut()
                } else {
                    &mut session
                },
                if missing == "err_buf" {
                    null_mut()
                } else {
                    buf.as_mut_ptr()
                },
                buf.len(),
                if missing == "out_err_len" {
                    null_mut()
                } else {
                    &mut err_len
                },
            )
        };
        assert_eq!(status, LOCSIM_ERR_NULL_POINTER, "{missing}");
        assert_eq!(session, SESSION_BEFORE, "{missing}");
        assert_eq!(err_len, LEN_BEFORE, "{missing}");
        assert!(buf.iter().all(|byte| *byte == BYTE_BEFORE), "{missing}");
    }
}

#[test]
fn a_null_pointer_is_reported_before_any_other_fault() {
    // Also wrong: the struct size, the capacity, the encoding, the policy.
    let json = [0xFF_u8, 0xFE];
    let policy = LocsimPolicy {
        struct_size: 1,
        ..bad_policy()
    };
    let mut session = SESSION_BEFORE;
    let mut buf = [BYTE_BEFORE; 16];
    // SAFETY: the pointers that are not null are to live locals.
    let status = unsafe {
        locsim_session_create(
            json.as_ptr(),
            json.len(),
            &policy,
            0,
            &mut session,
            buf.as_mut_ptr(),
            buf.len(),
            null_mut(),
        )
    };
    assert_eq!(status, LOCSIM_ERR_NULL_POINTER);
    assert_eq!(session, SESSION_BEFORE);
    assert_eq!(buf, [BYTE_BEFORE; 16]);
}

#[test]
fn a_null_scenario_or_buffer_is_allowed_with_a_length_of_zero() {
    let _guard = shared();
    // No document at all, and nowhere to put the explanation.
    let absent = create_raw(null(), 0, &policy(), 16, 0, 0);
    assert_eq!(absent.status, LOCSIM_ERR_SCENARIO);
    assert_eq!(absent.session, 0);
    assert!(absent.err_len > 0);

    // The same call with pointers that are not null gives the same answer.
    let empty = create_raw(FIXED.as_ptr(), 0, &policy(), 16, 8192, 8192);
    assert_eq!(empty.status, LOCSIM_ERR_SCENARIO);
    assert_eq!(empty.err_len, absent.err_len);
    assert!(!empty.text().is_empty());

    // And a null buffer of capacity zero does not stand in the way of success.
    let mut session = SESSION_BEFORE;
    let mut err_len = LEN_BEFORE;
    let policy = policy();
    // SAFETY: the pointers that are not null are to live locals.
    let status = unsafe {
        locsim_session_create(
            FIXED.as_ptr(),
            FIXED.len(),
            &policy,
            16,
            &mut session,
            null_mut(),
            0,
            &mut err_len,
        )
    };
    assert_eq!(status, LOCSIM_OK);
    assert_eq!(err_len, 0);
    assert_eq!(locsim_session_destroy(session), LOCSIM_OK);
}

// --- Order of checks ----------------------------------------------------

#[test]
fn several_faults_at_once_are_reported_in_the_documented_order() {
    let _guard = shared();
    let not_utf8 = [b'{', 0xFF, b'}'];
    let not_a_scenario = b"{}";
    let good = FIXED.as_bytes();
    let wrong_size = LocsimPolicy {
        struct_size: 65,
        ..bad_policy()
    };
    let too_long = LOCSIM_SCENARIO_MAX_BYTES + 1;

    // Struct size, before the length, the capacity, the encoding and the policy.
    create_raw(not_utf8.as_ptr(), too_long, &wrong_size, 0, 64, 64)
        .assert_failed_without_text(LOCSIM_ERR_STRUCT_SIZE);
    // The length, before the encoding and the policy.
    create_raw(not_utf8.as_ptr(), too_long, &bad_policy(), 16, 64, 64)
        .assert_failed_without_text(LOCSIM_ERR_INVALID_ARGUMENT);
    // The capacity, before the encoding and the policy.
    create_raw(not_utf8.as_ptr(), not_utf8.len(), &bad_policy(), 0, 64, 64)
        .assert_failed_without_text(LOCSIM_ERR_INVALID_ARGUMENT);
    // The encoding, before the policy.
    create(&not_utf8, &bad_policy()).assert_failed_without_text(LOCSIM_ERR_INVALID_UTF8);
    // The scenario, before the policy: the text is about the document only.
    let scenario_first = create(not_a_scenario, &bad_policy());
    assert_eq!(scenario_first.status, LOCSIM_ERR_SCENARIO);
    assert_eq!(scenario_first.session, 0);
    assert!(!scenario_first.text().contains("backoff_multiplier"));
    assert!(!scenario_first.text().contains("missed_window_ticks"));
    // The policy, once everything before it is right.
    let policy_last = create(good, &bad_policy());
    assert_eq!(policy_last.status, LOCSIM_ERR_POLICY);
    assert_eq!(policy_last.session, 0);
}

#[test]
fn a_struct_size_other_than_the_exact_one_is_refused() {
    let _guard = shared();
    for struct_size in [0, 63, 65, u32::MAX] {
        let policy = LocsimPolicy {
            struct_size,
            ..policy()
        };
        create(FIXED.as_bytes(), &policy).assert_failed_without_text(LOCSIM_ERR_STRUCT_SIZE);
    }
}

#[test]
fn bytes_that_are_not_utf8_are_refused_wherever_they_are() {
    let _guard = shared();
    let good = FIXED.as_bytes();
    let mut at_start = good.to_vec();
    at_start.insert(0, 0xFF);
    let mut in_middle = good.to_vec();
    in_middle.insert(good.len() / 2, 0xC0);
    // The first two bytes of a three-byte character, and then nothing.
    let mut cut_short = good.to_vec();
    cut_short.extend_from_slice(&[0xE2, 0x82]);
    for json in [at_start, in_middle, cut_short] {
        create(&json, &policy()).assert_failed_without_text(LOCSIM_ERR_INVALID_UTF8);
    }
}

// --- The explanation of a refusal ---------------------------------------

#[test]
fn the_text_is_copied_whole_or_not_at_all_and_its_length_is_always_reported() {
    let _guard = shared();
    let json = b"{";
    let reference = create(json, &policy());
    assert_eq!(reference.status, LOCSIM_ERR_SCENARIO);
    assert_eq!(reference.session, 0);
    let text = reference.text().to_owned();
    let len = text.len();
    assert!(len > 1);
    // Nothing beyond the text was written.
    assert!(reference.buf[len..].iter().all(|b| *b == BYTE_BEFORE));

    // No room at all.
    let none = create_raw(json.as_ptr(), json.len(), &policy(), 16, 0, 0);
    assert_eq!((none.status, none.err_len), (LOCSIM_ERR_SCENARIO, len));

    // One byte too little: the length is reported and nothing is copied.
    let short = create_raw(json.as_ptr(), json.len(), &policy(), 16, len + 8, len - 1);
    assert_eq!((short.status, short.err_len), (LOCSIM_ERR_SCENARIO, len));
    assert_eq!(short.session, 0);
    assert!(short.buf_untouched());

    // Exactly enough: the text, no terminator, and nothing after it.
    let exact = create_raw(json.as_ptr(), json.len(), &policy(), 16, len + 8, len);
    assert_eq!((exact.status, exact.err_len), (LOCSIM_ERR_SCENARIO, len));
    assert_eq!(&exact.buf[..len], text.as_bytes());
    assert!(exact.buf[len..].iter().all(|b| *b == BYTE_BEFORE));
}

#[test]
fn a_refused_policy_is_explained_one_line_per_problem() {
    let _guard = shared();
    let refused = create(FIXED.as_bytes(), &bad_policy());
    assert_eq!(refused.status, LOCSIM_ERR_POLICY);
    assert_eq!(refused.session, 0);
    let lines: Vec<&str> = refused.text().lines().collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("backoff_multiplier: "));
    assert!(lines[1].starts_with("missed_window_ticks: "));

    // What only the scenario makes checkable: fixed.json updates every second.
    let stalls_too_soon = LocsimPolicy {
        stall_after_s: 0.5,
        ..policy()
    };
    let refused = create(FIXED.as_bytes(), &stalls_too_soon);
    assert_eq!(refused.status, LOCSIM_ERR_POLICY);
    assert!(refused.text().starts_with("stall_after_s: "));
}

#[test]
fn a_long_explanation_is_cut_to_the_maximum_at_a_character() {
    let _guard = shared();
    let mut json = String::from("{\"schema_version\": 1");
    for n in 0..600 {
        json.push_str(&format!(", \"\u{e9}\u{20ac}{n}\": 0"));
    }
    json.push('}');
    let refused = create(json.as_bytes(), &policy());
    assert_eq!(refused.status, LOCSIM_ERR_SCENARIO);
    assert!(refused.err_len <= LOCSIM_CREATE_ERROR_TEXT_MAX);
    // Cut, and by less than one character short of the maximum.
    assert!(refused.err_len > LOCSIM_CREATE_ERROR_TEXT_MAX - 4);
    // `text` fails if the bytes are not UTF-8.
    assert!(refused.text().lines().count() > 100);
}

#[test]
fn cutting_never_splits_a_character_and_never_cuts_more_than_one() {
    // Characters of one, two, three and four bytes.
    let text = "a\u{e9}\u{20ac}\u{1F600}".repeat(3);
    for max in 0..=text.len() + 2 {
        let cut = truncated(text.clone(), max);
        assert!(cut.len() <= max);
        assert!(text.starts_with(&cut));
        if max >= text.len() {
            assert_eq!(cut, text);
        } else {
            assert!(cut.len() + 4 > max, "{max}: {}", cut.len());
        }
    }
}

// --- Handles ------------------------------------------------------------

#[test]
fn a_failed_creation_creates_nothing() {
    let _guard = alone();
    let before = live();
    let wrong_size = LocsimPolicy {
        struct_size: 0,
        ..policy()
    };
    assert_ne!(create(FIXED.as_bytes(), &wrong_size).status, LOCSIM_OK);
    assert_ne!(create(&[0xFF], &policy()).status, LOCSIM_OK);
    assert_ne!(create(b"{}", &policy()).status, LOCSIM_OK);
    assert_ne!(create(FIXED.as_bytes(), &bad_policy()).status, LOCSIM_OK);
    arm(Point::Create);
    assert_eq!(create(FIXED.as_bytes(), &policy()).status, LOCSIM_ERR_PANIC);
    assert_eq!(live(), before);
}

#[test]
fn handles_that_were_never_issued_cannot_be_destroyed() {
    let _guard = shared();
    for handle in [0, u64::MAX, 1 << 32, 0xFFFF_0000_0000_0001, 900_000] {
        assert_eq!(
            locsim_session_destroy(handle),
            LOCSIM_ERR_INVALID_HANDLE,
            "{handle:#x}"
        );
    }
}

#[test]
fn a_destroyed_handle_stays_invalid_when_its_slot_is_used_again() {
    let _guard = alone();
    let old = open();
    assert_eq!(locsim_session_destroy(old), LOCSIM_OK);
    let new = open();
    // Alone, the freed slot is the first free one: same slot, new handle.
    assert_eq!(new as u32, old as u32);
    assert_ne!(new, old);
    assert_eq!(locsim_session_destroy(old), LOCSIM_ERR_INVALID_HANDLE);
    // The stale handle did not reach the new session.
    assert!(registry().get(new).is_some());
    assert_eq!(locsim_session_destroy(new), LOCSIM_OK);
}

#[test]
fn the_session_limit_is_the_last_check_and_frees_up_on_destroy() {
    let _guard = alone();
    assert_eq!(live(), 0);
    let handles: Vec<u64> = (0..LOCSIM_SESSION_LIMIT).map(|_| open()).collect();
    let mut distinct = handles.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), LOCSIM_SESSION_LIMIT);

    create(FIXED.as_bytes(), &policy()).assert_failed_without_text(LOCSIM_ERR_LIMIT);
    // A refused policy is still reported as that.
    assert_eq!(
        create(FIXED.as_bytes(), &bad_policy()).status,
        LOCSIM_ERR_POLICY
    );
    assert_eq!(live(), LOCSIM_SESSION_LIMIT);

    assert_eq!(locsim_session_destroy(handles[7]), LOCSIM_OK);
    let again = open();
    assert!(!handles.contains(&again));
    create(FIXED.as_bytes(), &policy()).assert_failed_without_text(LOCSIM_ERR_LIMIT);

    assert_eq!(locsim_session_destroy(again), LOCSIM_OK);
    for (n, handle) in handles.into_iter().enumerate() {
        let expected = if n == 7 {
            LOCSIM_ERR_INVALID_HANDLE
        } else {
            LOCSIM_OK
        };
        assert_eq!(locsim_session_destroy(handle), expected);
    }
    assert_eq!(live(), 0);
}

// --- Destroying while something else holds the session ------------------

#[test]
fn destroy_is_busy_while_another_thread_holds_the_session_and_changes_nothing() {
    let _guard = shared();
    let handle = open();
    let cell = registry().get(handle).unwrap();
    let (locked, is_locked) = mpsc::channel();
    let (release, may_release) = mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        let guard = cell.lock().unwrap();
        locked.send(()).unwrap();
        may_release.recv().unwrap();
        matches!(*guard, State::Open(_))
    });
    // From here until `release`, the other thread provably holds the lock.
    is_locked.recv().unwrap();
    assert_eq!(locsim_session_destroy(handle), LOCSIM_ERR_BUSY);
    assert_eq!(locsim_session_destroy(handle), LOCSIM_ERR_BUSY);
    assert!(registry().get(handle).is_some());
    release.send(()).unwrap();
    // The holder saw an open session throughout.
    assert!(holder.join().unwrap());
    assert_eq!(locsim_session_destroy(handle), LOCSIM_OK);
    assert_eq!(locsim_session_destroy(handle), LOCSIM_ERR_INVALID_HANDLE);
}

#[test]
fn a_cell_reached_before_destroy_and_locked_after_it_finds_the_session_closed() {
    let _guard = shared();
    let handle = open();
    let early = registry().get(handle).unwrap();
    assert_eq!(locsim_session_destroy(handle), LOCSIM_OK);
    assert!(matches!(*early.lock().unwrap(), State::Closed));
}

#[test]
fn of_several_simultaneous_destroys_exactly_one_succeeds() {
    let _guard = shared();
    for _ in 0..50 {
        let handle = open();
        let start = Barrier::new(8);
        let statuses: Vec<LocsimStatus> = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        locsim_session_destroy(handle)
                    })
                })
                .collect();
            threads.into_iter().map(|t| t.join().unwrap()).collect()
        });
        // No destroy can see another one's lock: each tries the session
        // only while it holds the registry.
        let succeeded = statuses.iter().filter(|s| **s == LOCSIM_OK).count();
        let invalid = statuses
            .iter()
            .filter(|s| **s == LOCSIM_ERR_INVALID_HANDLE)
            .count();
        assert_eq!((succeeded, invalid), (1, 7), "{statuses:?}");
    }
}

// --- Panics and poisoned locks ------------------------------------------

#[test]
fn a_panic_during_creation_is_reported_and_the_outputs_are_zeroed() {
    let _guard = shared();
    arm(Point::Create);
    create(FIXED.as_bytes(), &policy()).assert_failed_without_text(LOCSIM_ERR_PANIC);
    // The library still works.
    let handle = open();
    assert_eq!(locsim_session_destroy(handle), LOCSIM_OK);
}

/// What must hold whenever a session is released: no lock is held and no
/// other panic is unwinding.
const RELEASED_ALONE: DropConditions = DropConditions {
    during_a_panic: false,
    registry_locked: false,
};

#[test]
fn a_session_is_released_after_the_locks_and_outside_any_panic() {
    // Alone, so that no other test can be holding the registry.
    let _guard = alone();
    let handle = open();
    take_session_drop();
    assert_eq!(locsim_session_destroy(handle), LOCSIM_OK);
    assert_eq!(take_session_drop(), Some(RELEASED_ALONE));
    // A refused destroy releases nothing.
    assert_eq!(locsim_session_destroy(handle), LOCSIM_ERR_INVALID_HANDLE);
    assert_eq!(take_session_drop(), None);
}

#[test]
fn a_session_whose_release_panics_is_reported_and_its_handle_is_invalid() {
    let _guard = alone();
    let handle = open();
    let other = open();
    let before = live();
    take_session_drop();
    arm(Point::SessionDrop);
    assert_eq!(locsim_session_destroy(handle), LOCSIM_ERR_PANIC);
    // The destructor that panicked was the first panic, not one met while
    // unwinding from another, and it ran with the registry released.
    assert_eq!(take_session_drop(), Some(RELEASED_ALONE));
    // The handle was invalidated before the release was attempted.
    assert_eq!(live(), before - 1);
    assert!(registry().get(handle).is_none());
    assert_eq!(locsim_session_destroy(handle), LOCSIM_ERR_INVALID_HANDLE);
    // Nothing is poisoned and other sessions are not affected.
    assert!(registry()
        .get(other)
        .is_some_and(|cell| !cell.is_poisoned()));
    assert_eq!(locsim_session_destroy(other), LOCSIM_OK);
    assert_eq!(take_session_drop(), Some(RELEASED_ALONE));
}

#[test]
fn a_session_refused_at_the_limit_is_released_after_the_registry() {
    let _guard = alone();
    let handles: Vec<u64> = (0..LOCSIM_SESSION_LIMIT).map(|_| open()).collect();
    take_session_drop();
    assert_eq!(create(FIXED.as_bytes(), &policy()).status, LOCSIM_ERR_LIMIT);
    assert_eq!(take_session_drop(), Some(RELEASED_ALONE));
    for handle in handles {
        assert_eq!(locsim_session_destroy(handle), LOCSIM_OK);
    }
}

#[test]
fn a_session_whose_lock_is_poisoned_can_still_be_destroyed() {
    let _guard = shared();
    let handle = open();
    let cell = registry().get(handle).unwrap();
    let panicked = std::thread::spawn(move || {
        let _guard = cell.lock().unwrap();
        panic!("poisons the session's mutex");
    })
    .join();
    assert!(panicked.is_err());
    assert!(registry().get(handle).unwrap().is_poisoned());
    assert_eq!(locsim_session_destroy(handle), LOCSIM_OK);
    assert_eq!(locsim_session_destroy(handle), LOCSIM_ERR_INVALID_HANDLE);
}

#[test]
fn a_poisoned_registry_keeps_working() {
    let _guard = shared();
    let before = open();
    let panicked = std::thread::spawn(|| {
        let _guard = registry();
        panic!("poisons the registry's mutex");
    })
    .join();
    assert!(panicked.is_err());
    assert!(REGISTRY.is_poisoned());
    let after = open();
    assert_ne!(after, before);
    assert_eq!(locsim_session_destroy(before), LOCSIM_OK);
    assert_eq!(locsim_session_destroy(after), LOCSIM_OK);
}
