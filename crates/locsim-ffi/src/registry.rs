//! The table that turns a handle into a session, and back into nothing.
//!
//! A handle is `generation << 32 | (slot index + 1)`. Zero is never issued.
//! Closing a handle empties its slot and moves the slot to the next
//! generation, so the same number is never issued twice: a slot whose
//! generation cannot advance is retired instead of reused.
//!
//! Locking. The registry sits behind one mutex (see `session.rs`) and each
//! value behind its own. The order is registry, then value, and a value's
//! mutex is only ever taken with `try_lock`. So the registry's mutex is the
//! only one anything waits for, and nothing that holds it waits for
//! anything: no cycle of waiting threads can form.
//!
//! Nothing here panics while the registry is borrowed: slots are reached
//! with `get_mut`, and the arithmetic is checked or saturating.

use std::sync::{Arc, Mutex, TryLockError};

/// The most values that may exist at once.
pub(crate) const LIMIT: usize = 1024;

struct Slot<T> {
    generation: u32,
    /// The generation ran out. The slot is never used again.
    retired: bool,
    cell: Option<Arc<Mutex<T>>>,
}

pub(crate) struct Registry<T> {
    slots: Vec<Slot<T>>,
    live: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseError {
    /// Never issued, or already closed.
    Invalid,
    /// The value's mutex is held. Nothing changed; the handle stays valid.
    Busy,
}

fn encode(number: u32, generation: u32) -> u64 {
    u64::from(generation) << 32 | u64::from(number)
}

/// The slot index and generation a handle names, if it can name any.
fn decode(handle: u64) -> Option<(usize, u32)> {
    let number = handle as u32;
    let index = usize::try_from(number.checked_sub(1)?).ok()?;
    Some((index, (handle >> 32) as u32))
}

impl<T> Registry<T> {
    pub(crate) const fn new() -> Self {
        Self {
            slots: Vec::new(),
            live: 0,
        }
    }

    /// Issues a handle for `cell`. At the limit the cell is handed back, so
    /// that the caller drops it after releasing the registry.
    pub(crate) fn insert(&mut self, cell: Arc<Mutex<T>>) -> Result<u64, Arc<Mutex<T>>> {
        if self.live >= LIMIT {
            return Err(cell);
        }
        let free = self
            .slots
            .iter()
            .position(|slot| slot.cell.is_none() && !slot.retired);
        let index = match free {
            Some(index) => index,
            None => {
                self.slots.push(Slot {
                    generation: 0,
                    retired: false,
                    cell: None,
                });
                self.slots.len() - 1
            }
        };
        let number = index.checked_add(1).and_then(|n| u32::try_from(n).ok());
        let (Some(number), Some(slot)) = (number, self.slots.get_mut(index)) else {
            return Err(cell);
        };
        slot.cell = Some(cell);
        self.live = self.live.saturating_add(1);
        Ok(encode(number, slot.generation))
    }

    /// Ends the handle: puts `tombstone` where the value was, empties the
    /// slot, and returns the value. From then on the handle is invalid, and
    /// whoever still holds the cell finds the tombstone in it.
    ///
    /// If the value's mutex is held, nothing is changed. A poisoned mutex
    /// does not prevent closing.
    pub(crate) fn close(&mut self, handle: u64, tombstone: T) -> Result<T, CloseError> {
        let (index, generation) = decode(handle).ok_or(CloseError::Invalid)?;
        let slot = self.slots.get_mut(index).ok_or(CloseError::Invalid)?;
        if slot.generation != generation {
            return Err(CloseError::Invalid);
        }
        let cell = slot.cell.as_ref().ok_or(CloseError::Invalid)?;
        let value = {
            let mut guard = match cell.try_lock() {
                Ok(guard) => guard,
                Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
                Err(TryLockError::WouldBlock) => return Err(CloseError::Busy),
            };
            std::mem::replace(&mut *guard, tombstone)
        };
        // Emptied before the generation moves: at every step in between,
        // the handle is already invalid.
        slot.cell = None;
        match slot.generation.checked_add(1) {
            Some(next) => slot.generation = next,
            None => slot.retired = true,
        }
        self.live = self.live.saturating_sub(1);
        Ok(value)
    }

    /// The cell a handle names. The caller locks it after letting go of the
    /// registry, and must look at what it then holds: the handle may have
    /// been closed in between.
    pub(crate) fn get(&self, handle: u64) -> Option<Arc<Mutex<T>>> {
        let (index, generation) = decode(handle)?;
        let slot = self.slots.get(index)?;
        (slot.generation == generation)
            .then(|| slot.cell.clone())
            .flatten()
    }

    #[cfg(test)]
    pub(crate) fn live(&self) -> usize {
        self.live
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(value: u32) -> Arc<Mutex<u32>> {
        Arc::new(Mutex::new(value))
    }

    const GONE: u32 = u32::MAX;

    #[test]
    fn zero_and_handles_never_issued_are_invalid() {
        let mut registry = Registry::new();
        let issued = registry.insert(cell(7)).unwrap();
        assert_ne!(issued, 0);
        for handle in [
            0,
            // Slot 2 does not exist.
            2,
            // The right slot in a generation it has not reached.
            issued | 1 << 32,
            u64::MAX,
            // Slot number zero in a later generation.
            1 << 32,
        ] {
            assert_eq!(
                registry.close(handle, GONE),
                Err(CloseError::Invalid),
                "{handle:#x}"
            );
            assert!(registry.get(handle).is_none(), "{handle:#x}");
        }
        assert_eq!(registry.live(), 1);
        assert_eq!(registry.close(issued, GONE), Ok(7));
    }

    #[test]
    fn a_closed_handle_is_invalid_and_closing_it_again_changes_nothing() {
        let mut registry = Registry::new();
        let first = registry.insert(cell(1)).unwrap();
        let second = registry.insert(cell(2)).unwrap();
        assert_eq!(registry.close(first, GONE), Ok(1));
        assert_eq!(registry.close(first, GONE), Err(CloseError::Invalid));
        assert!(registry.get(first).is_none());
        assert_eq!(registry.live(), 1);
        assert_eq!(*registry.get(second).unwrap().lock().unwrap(), 2);
    }

    #[test]
    fn a_reused_slot_gets_a_new_handle_and_the_old_one_stays_invalid() {
        let mut registry = Registry::new();
        let old = registry.insert(cell(1)).unwrap();
        registry.close(old, GONE).unwrap();
        let new = registry.insert(cell(2)).unwrap();
        // The same slot, the next generation.
        assert_eq!(new as u32, old as u32);
        assert_eq!(new >> 32, (old >> 32) + 1);
        assert_eq!(registry.close(old, GONE), Err(CloseError::Invalid));
        assert_eq!(*registry.get(new).unwrap().lock().unwrap(), 2);
        assert_eq!(registry.close(new, GONE), Ok(2));
    }

    #[test]
    fn a_held_value_cannot_be_closed_and_its_handle_stays_valid() {
        let mut registry = Registry::new();
        let handle = registry.insert(cell(5)).unwrap();
        let held = registry.get(handle).unwrap();
        let guard = held.lock().unwrap();
        assert_eq!(registry.close(handle, GONE), Err(CloseError::Busy));
        assert_eq!(registry.close(handle, GONE), Err(CloseError::Busy));
        assert_eq!(*guard, 5);
        assert_eq!(registry.live(), 1);
        drop(guard);
        assert_eq!(registry.close(handle, GONE), Ok(5));
    }

    #[test]
    fn a_cell_obtained_before_closing_holds_the_tombstone_afterwards() {
        let mut registry = Registry::new();
        let handle = registry.insert(cell(5)).unwrap();
        let early = registry.get(handle).unwrap();
        registry.close(handle, GONE).unwrap();
        // The slot is reused; the early cell is not the new value's.
        let new = registry.insert(cell(6)).unwrap();
        assert_eq!(*early.lock().unwrap(), GONE);
        assert_eq!(*registry.get(new).unwrap().lock().unwrap(), 6);
    }

    #[test]
    fn a_poisoned_value_can_still_be_closed() {
        let mut registry = Registry::new();
        let handle = registry.insert(cell(5)).unwrap();
        let shared = registry.get(handle).unwrap();
        let panicked = std::thread::spawn(move || {
            let _guard = shared.lock().unwrap();
            panic!("poisons the value's mutex");
        })
        .join();
        assert!(panicked.is_err());
        assert!(registry.get(handle).unwrap().is_poisoned());
        assert_eq!(registry.close(handle, GONE), Ok(5));
        assert_eq!(registry.close(handle, GONE), Err(CloseError::Invalid));
        assert_eq!(registry.live(), 0);
    }

    #[test]
    fn a_slot_whose_generation_ran_out_is_retired() {
        let mut registry = Registry::new();
        let first = registry.insert(cell(1)).unwrap();
        registry.close(first, GONE).unwrap();
        registry.slots[0].generation = u32::MAX;
        let last = registry.insert(cell(2)).unwrap();
        assert_eq!(last, u64::from(u32::MAX) << 32 | 1);
        assert_eq!(registry.close(last, GONE), Ok(2));
        assert_eq!(registry.close(last, GONE), Err(CloseError::Invalid));
        // The next value goes to a new slot: no handle of slot 1 can recur.
        let next = registry.insert(cell(3)).unwrap();
        assert_eq!(next, 2);
        assert!(registry.slots[0].retired && registry.slots[0].cell.is_none());
    }

    #[test]
    fn the_limit_refuses_one_more_and_frees_up_when_one_is_closed() {
        let mut registry = Registry::new();
        let handles: Vec<u64> = (0..LIMIT as u32)
            .map(|n| registry.insert(cell(n)).unwrap())
            .collect();
        let mut distinct = handles.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), LIMIT);

        let refused = registry.insert(cell(9)).unwrap_err();
        // Handed back whole, for the caller to drop.
        assert_eq!(Arc::strong_count(&refused), 1);
        assert_eq!(registry.live(), LIMIT);

        assert_eq!(registry.close(handles[400], GONE), Ok(400));
        let reused = registry.insert(cell(9)).unwrap();
        assert!(!handles.contains(&reused));
        assert!(registry.insert(cell(10)).is_err());
    }
}
