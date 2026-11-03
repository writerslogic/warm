// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Fractional power encoding (FPE) of timestamps, a research prototype for issue #164.
//!
//! A time `t` becomes the phasor vector `z(t)_j = exp(i * theta_j * t)`: a random base phasor
//! `exp(i * theta_j)` raised to the fractional power `t`. The inner product of two encodings,
//! `mean_j cos(theta_j * (t1 - t2))`, estimates a shift-invariant kernel `k(t1 - t2)`, so
//! similarity decays smoothly with time difference. The kernel is set by the law of `theta`:
//! half-Cauchy frequencies give `exp(-|d| / sigma)` (the exponential decay of the recency policy)
//! and Gaussian frequencies give `exp(-d^2 / (2 sigma^2))`.
//!
//! A validity window `[a, b)` is the bundle (integral) of `z(s)` over `s` in `[a, b)`. Its inner
//! product with `z(t)` is the kernel mass inside the window, divided by the total kernel mass,
//! so it is a soft membership in `[0, 1]`: near 1 deep inside the window, near 0 far outside and
//! one half at an edge of a window much wider than `sigma`. The cut is blurred over about
//! `sigma`, so the exact `valid_from <= now < valid_to` rule of the memory API cannot be
//! reproduced by thresholding a window code.
//!
//! Resolution limit: the frequency grid has spacing about `(pi / 2) / (sigma * dim)` near zero, so a
//! window code is accurate only while `(end - start) * (pi / 2) / (sigma * dim)` is well below 1.
//! A far-away end (an open-ended validity) aliases the low frequencies and returns noise of order
//! 0.4; close the window a few `sigma` past the latest time of interest. A true half-line code (the
//! start-edge term plus a constant one half) was not built.
//!
//! The time unit is the caller's (the evaluation harness uses days); `sigma` is in the same unit.
//! Frequencies are drawn from a stratified quantile grid with a seeded jitter, so a given
//! `(kernel, sigma, dim, seed)` always yields the same encoder.

use crate::core::memory_api::{RecencyBasis, RecencyContext, RecencyPolicy};
use anyhow::{ensure, Result};
use std::f64::consts::{FRAC_PI_2, PI};

/// The similarity law an encoder approximates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kernel {
    /// `exp(-|d| / sigma)`.
    Laplace,
    /// `exp(-d^2 / (2 sigma^2))`.
    Gaussian,
}

/// Seeded SplitMix64: enough randomness for jitter, with no dependency.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in the open interval (0, 1).
    fn unit_open(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
}

/// A seeded FPE time encoder.
#[derive(Clone, Debug)]
pub struct FpeTime {
    kernel: Kernel,
    sigma: f64,
    /// Positive frequencies, radians per time unit. The kernel is even, so the sign is
    /// irrelevant and only the magnitude is sampled.
    freqs: Vec<f64>,
}

impl FpeTime {
    /// An encoder of `dim` frequencies approximating `kernel` with length scale `sigma`.
    pub fn new(kernel: Kernel, sigma: f64, dim: usize, seed: u64) -> Result<Self> {
        ensure!(sigma.is_finite() && sigma > 0.0, "sigma must be positive");
        ensure!(dim > 0, "dim must be positive");
        let mut rng = SplitMix64(seed);
        let freqs = (0..dim)
            .map(|j| {
                let u = (j as f64 + rng.unit_open()) / dim as f64;
                match kernel {
                    Kernel::Laplace => (FRAC_PI_2 * u).tan() / sigma,
                    Kernel::Gaussian => {
                        let radius = (-2.0 * u.ln()).sqrt();
                        (radius * (2.0 * PI * rng.unit_open()).cos()).abs() / sigma
                    }
                }
            })
            .collect();
        Ok(Self {
            kernel,
            sigma,
            freqs,
        })
    }

    pub fn dim(&self) -> usize {
        self.freqs.len()
    }

    pub fn sigma(&self) -> f64 {
        self.sigma
    }

    /// The kernel this encoder approximates, evaluated exactly.
    pub fn kernel_value(&self, delta: f64) -> f64 {
        match self.kernel {
            Kernel::Laplace => (-delta.abs() / self.sigma).exp(),
            Kernel::Gaussian => (-delta * delta / (2.0 * self.sigma * self.sigma)).exp(),
        }
    }

    /// `integral of kernel_value over the real line`.
    fn kernel_mass(&self) -> f64 {
        match self.kernel {
            Kernel::Laplace => 2.0 * self.sigma,
            Kernel::Gaussian => self.sigma * (2.0 * PI).sqrt(),
        }
    }

    /// The `2 * dim` real vector `[cos(theta_j t) / sqrt(dim)..., sin(theta_j t) / sqrt(dim)...]`:
    /// the real form of the unit-norm phasor `z(t)`.
    pub fn encode(&self, t: f64) -> Vec<f64> {
        let scale = 1.0 / (self.freqs.len() as f64).sqrt();
        let mut out = Vec::with_capacity(2 * self.freqs.len());
        out.extend(self.freqs.iter().map(|th| (th * t).cos() * scale));
        out.extend(self.freqs.iter().map(|th| (th * t).sin() * scale));
        out
    }

    /// `dot(encode(a), encode(b))` without materializing the vectors.
    pub fn similarity(&self, a: f64, b: f64) -> f64 {
        let delta = a - b;
        self.freqs.iter().map(|th| (th * delta).cos()).sum::<f64>() / self.freqs.len() as f64
    }

    /// The code of the validity window `[start, end)`, scaled so that
    /// `dot(window(a, b), encode(t))` is the soft membership of `t` in the window.
    pub fn window(&self, start: f64, end: f64) -> Result<Vec<f64>> {
        ensure!(
            start.is_finite() && end.is_finite() && start <= end,
            "a window needs finite ends with start <= end"
        );
        let scale = 1.0 / ((self.freqs.len() as f64).sqrt() * self.kernel_mass());
        let mut out = Vec::with_capacity(2 * self.freqs.len());
        // integral over [start, end) of exp(i theta s) ds = (sin(th b) - sin(th a)) / th
        //                                                 + i (cos(th a) - cos(th b)) / th
        out.extend(
            self.freqs
                .iter()
                .map(|th| ((th * end).sin() - (th * start).sin()) / th * scale),
        );
        out.extend(
            self.freqs
                .iter()
                .map(|th| ((th * start).cos() - (th * end).cos()) / th * scale),
        );
        Ok(out)
    }

    /// Soft membership of `t` in a window built by [`FpeTime::window`].
    pub fn membership(&self, window: &[f64], t: f64) -> f64 {
        debug_assert_eq!(window.len(), 2 * self.freqs.len());
        let (re, im) = window.split_at(self.freqs.len());
        // `window` carries 1 / sqrt(dim) per entry, like `encode`: restore the other factor.
        let sum: f64 = self
            .freqs
            .iter()
            .zip(re.iter().zip(im))
            .map(|(th, (re, im))| re * (th * t).cos() + im * (th * t).sin())
            .sum();
        sum / (self.freqs.len() as f64).sqrt()
    }
}

/// Dot product of two equal-length codes.
pub fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// A [`RecencyPolicy`] whose multiplier is `(1 - weight) + weight * similarity(now, at)`,
/// with the similarity read from an FPE encoder. A fact without a time keeps its score.
#[derive(Clone, Debug)]
pub struct FpeRecency {
    pub encoder: FpeTime,
    /// In `[0, 1]`: 0 leaves scores alone, 1 multiplies by the raw similarity.
    pub weight: f64,
    pub basis: RecencyBasis,
    /// Milliseconds per encoder time unit.
    pub unit_ms: f64,
}

impl RecencyPolicy for FpeRecency {
    fn factor(&self, fact: &RecencyContext) -> f64 {
        let at = match self.basis {
            RecencyBasis::Event => fact.event_time.or(fact.valid_from).or(fact.ingest_time),
            RecencyBasis::Ingest => fact.ingest_time,
        };
        let Some(at) = at else { return 1.0 };
        let sim = self
            .encoder
            .similarity(fact.now as f64 / self.unit_ms, at as f64 / self.unit_ms)
            .max(0.0);
        (1.0 - self.weight) + self.weight * sim
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::memory_api::FactStatus;

    const DIM: usize = 4096;
    const SIGMA: f64 = 30.0;

    fn laplace(seed: u64) -> FpeTime {
        FpeTime::new(Kernel::Laplace, SIGMA, DIM, seed).unwrap()
    }

    #[test]
    fn similarity_decays_monotonically_with_time_difference() {
        for kernel in [Kernel::Laplace, Kernel::Gaussian] {
            let enc = FpeTime::new(kernel, SIGMA, DIM, 164).unwrap();
            assert!((enc.similarity(10.0, 10.0) - 1.0).abs() < 1e-12);
            // Finite dimension adds noise of about 1/sqrt(dim); decay is monotone up to it,
            // over the range of four length scales.
            let tolerance = 0.02;
            let mut previous = 1.0;
            for step in 0..=120 {
                let sim = enc.similarity(0.0, f64::from(step));
                assert!(sim <= previous + tolerance, "{kernel:?} rose at {step}");
                previous = sim;
            }
            // The decay itself is large against that noise, and tracks the target kernel.
            assert!(enc.similarity(0.0, SIGMA) < enc.similarity(0.0, SIGMA / 4.0) - 0.2);
            for delta in [0.0, 7.5, 30.0, 60.0, 120.0] {
                let error = (enc.similarity(0.0, delta) - enc.kernel_value(delta)).abs();
                assert!(error < 0.03, "{kernel:?} off by {error} at {delta}");
            }
        }
    }

    #[test]
    fn similarity_is_symmetric_and_matches_the_explicit_encoding() {
        let enc = laplace(1);
        let (a, b) = (3.25, 41.5);
        assert!((enc.similarity(a, b) - enc.similarity(b, a)).abs() < 1e-12);
        assert!((dot(&enc.encode(a), &enc.encode(b)) - enc.similarity(a, b)).abs() < 1e-9);
        let norm = dot(&enc.encode(a), &enc.encode(a));
        assert!((norm - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_same_seed_gives_the_same_encoder_and_another_seed_a_different_one() {
        let (a, b, other) = (laplace(7), laplace(7), laplace(8));
        assert_eq!(a.encode(12.5), b.encode(12.5));
        assert_eq!(a.window(0.0, 90.0).unwrap(), b.window(0.0, 90.0).unwrap());
        assert_ne!(a.encode(12.5), other.encode(12.5));
    }

    #[test]
    fn a_window_edge_is_soft_and_ordered_at_limit_minus_one_limit_and_plus_one() {
        let enc = FpeTime::new(Kernel::Laplace, 3.0, DIM, 164).unwrap();
        let (start, end) = (1000.0, 2000.0);
        let window = enc.window(start, end).unwrap();
        let at = |t: f64| enc.membership(&window, t);
        // Start edge: rising through the edge, one half at it.
        assert!(at(start - 1.0) < at(start) && at(start) < at(start + 1.0));
        assert!((at(start) - 0.5).abs() < 0.05);
        // End edge: falling through the edge, one half at it.
        assert!(at(end - 1.0) > at(end) && at(end) > at(end + 1.0));
        assert!((at(end) - 0.5).abs() < 0.05);
        // Far inside and far outside.
        assert!(at(1500.0) > 0.95);
        assert!(at(start - 60.0) < 0.05 && at(end + 60.0) < 0.05);
        // Soft: the neighbours of an edge are not 0 and 1.
        assert!(at(start - 1.0) > 0.1 && at(start + 1.0) < 0.9);
    }

    #[test]
    fn membership_equals_the_explicit_dot_product_and_the_exact_kernel_integral() {
        let enc = laplace(3);
        let (start, end) = (100.0, 160.0);
        let window = enc.window(start, end).unwrap();
        for t in [50.0, 100.0, 130.0, 160.0, 400.0] {
            let membership = enc.membership(&window, t);
            assert!((membership - dot(&window, &enc.encode(t))).abs() < 1e-9);
            // Closed form of the Laplace kernel mass inside [start, end).
            let exact = |s: f64| {
                if s >= t {
                    1.0 - (-(s - t) / SIGMA).exp()
                } else {
                    (-(t - s) / SIGMA).exp() - 1.0
                }
            };
            let target = 0.5 * (exact(end) - exact(start));
            assert!(
                (membership - target).abs() < 0.03,
                "t={t}: {membership} vs {target}"
            );
        }
    }

    #[test]
    fn window_code_tracks_the_exact_integral_at_the_harness_scale() {
        // sigma = 7 days at dimension 1024 with a window of about 500 days, the harness regime:
        // (end - start) * (pi / 2) / (sigma * dim) is about 0.1.
        let sigma = 7.0;
        let enc = FpeTime::new(Kernel::Laplace, sigma, 1024, 164).unwrap();
        let (start, end) = (10_000.0, 10_500.0);
        let window = enc.window(start, end).unwrap();
        for t in [9_900.0, 9_990.0, 10_000.0, 10_003.0, 10_250.0, 10_497.0, 10_500.0, 10_600.0] {
            let f = |s: f64| {
                if s >= t {
                    1.0 - (-(s - t) / sigma).exp()
                } else {
                    (-(t - s) / sigma).exp() - 1.0
                }
            };
            let target = 0.5 * (f(end) - f(start));
            let got = enc.membership(&window, t);
            assert!((got - target).abs() < 0.06, "t={t}: {got} vs {target}");
        }
    }

    #[test]
    fn invalid_parameters_are_rejected() {
        assert!(FpeTime::new(Kernel::Laplace, 0.0, 16, 0).is_err());
        assert!(FpeTime::new(Kernel::Laplace, f64::NAN, 16, 0).is_err());
        assert!(FpeTime::new(Kernel::Gaussian, 1.0, 0, 0).is_err());
        let enc = laplace(0);
        assert!(enc.window(5.0, 4.0).is_err());
        assert!(enc.window(f64::NEG_INFINITY, 4.0).is_err());
        assert!(enc.window(5.0, f64::INFINITY).is_err());
    }

    #[test]
    fn the_recency_policy_decays_with_age_and_stays_in_range() {
        let day = 86_400_000.0;
        let policy = FpeRecency {
            encoder: laplace(164),
            weight: 0.5,
            basis: RecencyBasis::Event,
            unit_ms: day,
        };
        let now = 1_000 * 86_400_000i64;
        let context = |event_time: Option<i64>| RecencyContext {
            now,
            event_time,
            valid_from: None,
            ingest_time: None,
            status: FactStatus::Current,
        };
        let factors: Vec<f64> = [0i64, 5, 15, 30, 60, 120]
            .iter()
            .map(|age| policy.factor(&context(Some(now - age * 86_400_000))))
            .collect();
        assert!((factors[0] - 1.0).abs() < 1e-12);
        assert!(factors.windows(2).all(|pair| pair[1] <= pair[0] + 0.01));
        assert!(factors.iter().all(|f| (0.5 - 1e-9..=1.0).contains(f)));
        assert_eq!(policy.factor(&context(None)), 1.0);
    }
}
