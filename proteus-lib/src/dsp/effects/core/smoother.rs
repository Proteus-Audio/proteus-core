//! Zero-allocation linear parameter ramp primitive.
//!
//! `ParamSmoother` provides glitch-free transitions between parameter values by
//! linearly interpolating from the current value to the target over a
//! configurable number of samples. The cost is one `f32` addition and one
//! `usize` decrement per sample during the ramp; zero cost when settled.

/// Default ramp duration in milliseconds used when no explicit value is given.
pub(crate) const DEFAULT_PARAMETER_RAMP_MS: f32 = 5.0;

/// Compute the number of ramp samples for a given duration and sample rate.
pub(crate) fn ramp_samples(ramp_ms: f32, sample_rate: u32) -> usize {
    ((ramp_ms / 1000.0) * sample_rate as f32).round() as usize
}

/// Zero-allocation linear parameter ramp.
///
/// Call [`set_target`](ParamSmoother::set_target) when the parameter changes,
/// then call [`next`](ParamSmoother::next) once per sample to get the smoothed
/// value. When the ramp completes, [`next`] returns the target with no
/// arithmetic overhead.
#[derive(Debug, Clone)]
pub(crate) struct ParamSmoother {
    current: f32,
    target: f32,
    increment: f32,
    remaining: usize,
}

impl ParamSmoother {
    /// Create a smoother initialised to `initial`.
    pub fn new(initial: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            increment: 0.0,
            remaining: 0,
        }
    }

    /// Set a new target value with a ramp over `ramp_samples` samples.
    ///
    /// If `ramp_samples` is zero, the value snaps immediately.
    pub fn set_target(&mut self, target: f32, ramp_samples: usize) {
        if ramp_samples == 0 || (self.current - target).abs() < f32::EPSILON {
            self.current = target;
            self.target = target;
            self.increment = 0.0;
            self.remaining = 0;
            return;
        }
        self.target = target;
        self.increment = (target - self.current) / ramp_samples as f32;
        self.remaining = ramp_samples;
    }

    /// Advance and return the next smoothed value.
    #[inline]
    pub fn next(&mut self) -> f32 {
        if self.remaining == 0 {
            return self.current;
        }
        self.remaining -= 1;
        if self.remaining == 0 {
            self.current = self.target;
        } else {
            self.current += self.increment;
        }
        self.current
    }

    /// The current smoothed value without advancing.
    pub fn current(&self) -> f32 {
        self.current
    }

    /// The ramp target value.
    pub fn target(&self) -> f32 {
        self.target
    }

    /// Whether the smoother has reached its target.
    pub fn is_settled(&self) -> bool {
        self.remaining == 0
    }

    /// Reset the smoother to a value with no ramp.
    #[cfg(test)]
    pub fn reset(&mut self, value: f32) {
        self.current = value;
        self.target = value;
        self.increment = 0.0;
        self.remaining = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_settled_at_initial() {
        let s = ParamSmoother::new(0.5);
        assert!(s.is_settled());
        assert_eq!(s.current(), 0.5);
        assert_eq!(s.target(), 0.5);
    }

    #[test]
    fn set_target_zero_ramp_snaps() {
        let mut s = ParamSmoother::new(0.0);
        s.set_target(1.0, 0);
        assert!(s.is_settled());
        assert_eq!(s.current(), 1.0);
    }

    #[test]
    fn linear_ramp_reaches_target_exactly() {
        let mut s = ParamSmoother::new(0.0);
        s.set_target(1.0, 4);
        let mut values = Vec::new();
        for _ in 0..4 {
            values.push(s.next());
        }
        assert!(!s.is_settled() || s.current() == 1.0);
        // After 4 calls the smoother must have settled at target.
        assert!(s.is_settled());
        assert_eq!(s.current(), 1.0);
        // Final sample must be exactly the target (no floating-point drift).
        assert_eq!(*values.last().unwrap(), 1.0);
    }

    #[test]
    fn ramp_values_are_monotonically_increasing() {
        let mut s = ParamSmoother::new(0.0);
        s.set_target(1.0, 100);
        let mut prev = f32::NEG_INFINITY;
        for _ in 0..100 {
            let v = s.next();
            assert!(v >= prev);
            prev = v;
        }
        assert!(s.is_settled());
    }

    #[test]
    fn ramp_values_are_monotonically_decreasing() {
        let mut s = ParamSmoother::new(1.0);
        s.set_target(0.0, 100);
        let mut prev = f32::INFINITY;
        for _ in 0..100 {
            let v = s.next();
            assert!(v <= prev);
            prev = v;
        }
        assert!(s.is_settled());
    }

    #[test]
    fn retarget_mid_ramp_updates_trajectory() {
        let mut s = ParamSmoother::new(0.0);
        s.set_target(1.0, 10);
        // Advance halfway.
        for _ in 0..5 {
            s.next();
        }
        let mid = s.current();
        assert!(mid > 0.0 && mid < 1.0);
        // Retarget to 0.0.
        s.set_target(0.0, 10);
        assert!(!s.is_settled());
        for _ in 0..10 {
            s.next();
        }
        assert!(s.is_settled());
        assert_eq!(s.current(), 0.0);
    }

    #[test]
    fn set_target_same_value_stays_settled() {
        let mut s = ParamSmoother::new(0.5);
        s.set_target(0.5, 100);
        assert!(s.is_settled());
    }

    #[test]
    fn reset_snaps_to_value() {
        let mut s = ParamSmoother::new(0.0);
        s.set_target(1.0, 100);
        s.next();
        s.reset(0.5);
        assert!(s.is_settled());
        assert_eq!(s.current(), 0.5);
        assert_eq!(s.target(), 0.5);
    }

    #[test]
    fn next_after_settled_returns_current() {
        let mut s = ParamSmoother::new(0.75);
        assert_eq!(s.next(), 0.75);
        assert_eq!(s.next(), 0.75);
    }

    #[test]
    fn ramp_samples_helper() {
        assert_eq!(ramp_samples(5.0, 48_000), 240);
        assert_eq!(ramp_samples(0.0, 48_000), 0);
        assert_eq!(ramp_samples(10.0, 44_100), 441);
    }

    #[test]
    fn gain_sweep_no_discontinuity() {
        let mut s = ParamSmoother::new(0.8);
        s.set_target(1.2, 240);
        let mut prev = s.current();
        let max_step = (1.2 - 0.8) / 240.0 * 1.01; // allow 1% tolerance
        for _ in 0..240 {
            let v = s.next();
            assert!((v - prev).abs() <= max_step + f32::EPSILON);
            prev = v;
        }
        assert_eq!(s.current(), 1.2);
    }
}
