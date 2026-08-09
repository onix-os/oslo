//! A small bounded cache, replacing the `cached` crate.
//!
//! Three functions here are worth memoising — tokenizing a line, parsing a word and parsing an
//! arithmetic expression — because a shell runs the same handful of strings round a loop. Upstream
//! reached for `cached` to say so, and `cached` brings thirty-one crates with it: a proc-macro,
//! `parking_lot`, `ahash`, `hashbrown`, `zerocopy`, `web-time`, and its own copy of `darling` and
//! `syn`. None of them survive dead-code elimination into the final binary. All of them compile.
//!
//! What is actually needed is sixty lines.
//!
//! # Cleared rather than evicted
//!
//! `cached` keeps a true LRU. This keeps a `HashMap` and empties it when it reaches the bound,
//! which is cruder and, at this size, very nearly the same thing: the working set of a shell loop
//! is a few distinct strings, so the map fills only when the workload has genuinely moved on, and
//! that is the moment an LRU would have evicted everything useful anyway. Tracking recency would
//! cost a counter update on every *hit*, which is the case being optimised.
//!
//! Only successes are stored. An error is cheap to reproduce and is usually about to end the
//! parse; caching failures would also mean holding an error value alive for the sake of failing
//! identically a second time.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;

/// The bound. Upstream's `size = 64`, kept so behaviour does not change with the dependency.
const CAPACITY: usize = 64;

/// A memo shared by every caller of one function.
pub(crate) struct Memo<K, V> {
    entries: Mutex<HashMap<K, V>>,
}

impl<K: Eq + Hash + Clone, V: Clone> Default for Memo<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq + Hash + Clone, V: Clone> Memo<K, V> {
    /// Not `const`, because `HashMap::new` is not: `RandomState` reads the thread's seed. The
    /// statics that hold these are `LazyLock` for that reason.
    pub(crate) fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// [`Memo::get_or_insert`], looked up by borrow so a hit costs no allocation.
    ///
    /// The point of a cache is the hit, and `cached` allocated an owned key on every call to reach
    /// one — the string was cloned, hashed, found, and the clone thrown away. Here the owned key is
    /// built only on the miss that stores it.
    ///
    /// Only usable where the key borrows as a whole, so the two-field caches still take the
    /// allocating path: `HashMap<(String, Options), _>` cannot be probed with `(&str, &Options)`.
    pub(crate) fn get_or_insert_by<Q, E>(
        &self,
        lookup: &Q,
        own: impl FnOnce() -> K,
        compute: impl FnOnce() -> Result<V, E>,
    ) -> Result<V, E>
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if let Ok(entries) = self.entries.lock()
            && let Some(hit) = entries.get(lookup)
        {
            return Ok(hit.clone());
        }
        let value = compute()?;
        if let Ok(mut entries) = self.entries.lock() {
            if entries.len() >= CAPACITY {
                entries.clear();
            }
            entries.insert(own(), value.clone());
        }
        Ok(value)
    }

    /// The stored value for `key`, or `compute`'s, stored if it succeeded.
    ///
    /// **The lock is not held while `compute` runs.** These are parsers, and holding a global lock
    /// across a parse would serialise every thread in the program on the slowest one — the shell
    /// has a completion thread and a `$PATH` scan that both parse.
    ///
    /// A poisoned lock computes rather than panics: a cache is an optimisation, and a shell that
    /// died because one had gone wrong would be trading a certainty for a speedup.
    pub(crate) fn get_or_insert<E>(
        &self,
        key: K,
        compute: impl FnOnce() -> Result<V, E>,
    ) -> Result<V, E> {
        if let Ok(entries) = self.entries.lock()
            && let Some(hit) = entries.get(&key)
        {
            return Ok(hit.clone());
        }
        let value = compute()?;
        if let Ok(mut entries) = self.entries.lock() {
            // At the bound, start again. See the module note on why this is not an LRU.
            if entries.len() >= CAPACITY {
                entries.clear();
            }
            entries.insert(key, value.clone());
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second call with the same key does not recompute.
    #[test]
    fn a_hit_does_not_recompute() {
        let memo: Memo<String, usize> = Memo::new();
        let mut calls = 0;
        let mut ask = |memo: &Memo<String, usize>, calls: &mut usize| {
            memo.get_or_insert::<()>("k".to_string(), || {
                *calls += 1;
                Ok(7)
            })
        };
        assert_eq!(ask(&memo, &mut calls), Ok(7));
        assert_eq!(ask(&memo, &mut calls), Ok(7));
        assert_eq!(calls, 1, "the second call should have been a hit");
    }

    /// A failure is not stored: it is cheap to reproduce, and holding it would mean keeping an
    /// error alive in order to fail identically again.
    #[test]
    fn a_failure_is_not_remembered() {
        let memo: Memo<String, usize> = Memo::new();
        let mut calls = 0;
        for _ in 0..3 {
            let got: Result<usize, &str> = memo.get_or_insert("k".to_string(), || {
                calls += 1;
                Err("no")
            });
            assert_eq!(got, Err("no"));
        }
        assert_eq!(calls, 3);
    }

    /// The bound holds, and the cache still answers after it has been reached.
    #[test]
    fn the_capacity_is_a_bound_not_a_suggestion() {
        let memo: Memo<usize, usize> = Memo::new();
        for i in 0..CAPACITY * 3 {
            let _: Result<usize, ()> = memo.get_or_insert(i, || Ok(i));
            assert!(memo.entries.lock().expect("lock").len() <= CAPACITY);
        }
        // And it is still usable rather than merely small.
        let mut computed = false;
        let _: Result<usize, ()> = memo.get_or_insert(9999, || {
            computed = true;
            Ok(1)
        });
        assert!(computed);
        let mut again = false;
        let _: Result<usize, ()> = memo.get_or_insert(9999, || {
            again = true;
            Ok(1)
        });
        assert!(!again, "the entry just inserted should be a hit");
    }
}
