//! A small injectable clock abstraction so the engine's timing is deterministic in tests.
//!
//! The [`Engine`](crate::engine::Engine) is normally driven by the monotonic
//! `captured_at` timestamp carried on each [`RawFrame`](crate::frame::RawFrame),
//! but a [`Clock`] is provided for the convenience [`process_now`](crate::engine::Engine::process_now)
//! path and for embedders that want a single source of "now".

use std::cell::Cell;
use std::time::{Duration, Instant};

/// A source of monotonic time.
pub trait Clock {
    /// The current monotonic instant.
    fn now(&self) -> Instant;
}

/// The real, wall-driven monotonic clock ([`Instant::now`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A controllable clock for deterministic tests.
///
/// Starts at construction time and only advances when [`advance`](MockClock::advance)
/// is called, so settle / throttle behaviour can be tested without sleeping.
#[derive(Debug)]
pub struct MockClock {
    now: Cell<Instant>,
}

impl MockClock {
    /// Create a clock anchored at the current instant.
    pub fn new() -> Self {
        Self {
            now: Cell::new(Instant::now()),
        }
    }

    /// Advance the clock by `dur`.
    pub fn advance(&self, dur: Duration) {
        self.now.set(self.now.get() + dur);
    }

    /// Advance the clock by `ms` milliseconds.
    pub fn advance_ms(&self, ms: u64) {
        self.advance(Duration::from_millis(ms));
    }
}

impl Default for MockClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MockClock {
    fn now(&self) -> Instant {
        self.now.get()
    }
}

// Allow `&C: Clock` ergonomics where useful.
impl<C: Clock + ?Sized> Clock for &C {
    fn now(&self) -> Instant {
        (**self).now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_clock_advances_only_on_demand() {
        let c = MockClock::new();
        let t0 = c.now();
        assert_eq!(c.now(), t0, "does not advance on its own");
        c.advance(Duration::from_millis(250));
        assert_eq!(c.now().duration_since(t0), Duration::from_millis(250));
        c.advance_ms(750);
        assert_eq!(c.now().duration_since(t0), Duration::from_millis(1000));
    }

    #[test]
    fn mock_clock_default_matches_new() {
        let c = MockClock::default();
        let t0 = c.now();
        c.advance_ms(5);
        assert!(c.now() > t0);
    }

    #[test]
    fn system_clock_is_monotonic_nondecreasing() {
        let c = SystemClock;
        let a = c.now();
        let b = c.now();
        assert!(b >= a);
        // The `&C: Clock` blanket impl forwards.
        let r: &dyn Clock = &c;
        assert!(r.now() >= a);
    }
}
