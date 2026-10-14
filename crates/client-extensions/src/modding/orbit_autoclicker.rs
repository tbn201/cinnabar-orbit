pub const REQUEST: &str = "orbit.autoclicker";
pub const READY: &str = "orbit.autoclicker-ready";
pub const CAPABILITY: u8 = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Policy {
    pub min: f32,
    pub max: f32,
}

impl Policy {
    pub fn parse(values: &[f32]) -> Option<Self> {
        let [version, min, max] = values else {
            return None;
        };
        (*version == 1.0
            && min.is_finite()
            && max.is_finite()
            && (1.0..=20.0).contains(min)
            && (1.0..=20.0).contains(max)
            && min <= max
            && min.fract() == 0.0
            && max.fract() == 0.0)
            .then_some(Self {
                min: *min,
                max: *max,
            })
    }

    pub fn values(self) -> [f32; 3] {
        [1.0, self.min, self.max]
    }
}

#[cfg(any(test, not(target_arch = "wasm32")))]
#[derive(Default)]
pub struct Clicker {
    scope: Option<(u64, i32)>,
    policy: Option<Policy>,
    next: Option<f64>,
    last: f64,
    seed: u64,
}

#[cfg(any(test, not(target_arch = "wasm32")))]
impl Clicker {
    pub fn tick(
        &mut self,
        now: f64,
        scope: (u64, i32),
        held: bool,
        pressed: bool,
        mining: bool,
        policy: Option<Policy>,
    ) -> bool {
        let Some(policy) = policy.filter(|policy| Policy::parse(&policy.values()).is_some()) else {
            self.reset();
            return false;
        };
        if !now.is_finite() || now < 0.0 || !held || mining {
            self.reset();
            return false;
        }
        let changed = self.scope != Some(scope) || self.policy != Some(policy) || now < self.last;
        self.last = now;
        self.scope = Some(scope);
        self.policy = Some(policy);
        if changed || pressed || self.next.is_none() {
            self.seed = self.seed.wrapping_add(now.to_bits() ^ scope.0).max(1);
            self.schedule(now, policy);
            return false;
        }
        if self.next.is_some_and(|next| now >= next) {
            self.schedule(now, policy);
            return true;
        }
        false
    }

    pub fn reset(&mut self) {
        self.scope = None;
        self.policy = None;
        self.next = None;
    }

    fn schedule(&mut self, now: f64, policy: Policy) {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        let unit = (self.seed >> 40) as f64 / ((1u64 << 24) - 1) as f64;
        let cps = f64::from(policy.min) + unit * f64::from(policy.max - policy.min);
        self.next = Some(now + 1.0 / cps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_rates_are_rejected() {
        for values in [
            [2.0, 8.0, 12.0],
            [1.0, 0.0, 12.0],
            [1.0, 13.0, 12.0],
            [1.0, 8.0, 21.0],
            [1.0, f32::NAN, 12.0],
            [1.0, 8.5, 12.0],
        ] {
            assert_eq!(Policy::parse(&values), None);
        }
        assert_eq!(Policy::parse(&[]), None);
    }

    #[test]
    fn mining_release_and_missing_policy_stop_clicks_without_a_resume_burst() {
        let mut clicker = Clicker::default();
        let policy = Some(Policy {
            min: 10.0,
            max: 10.0,
        });
        assert!(!clicker.tick(0.0, (1, 0), true, true, false, policy));
        assert!(clicker.tick(0.1, (1, 0), true, false, false, policy));
        for (held, mining, enabled) in [
            (true, true, policy),
            (false, false, policy),
            (true, false, None),
        ] {
            assert!(!clicker.tick(10.0, (1, 0), held, false, mining, enabled));
            assert!(!clicker.tick(11.0, (1, 0), true, false, false, policy));
            assert!(clicker.tick(11.11, (1, 0), true, false, false, policy));
        }
    }

    #[test]
    fn intervals_stay_in_range_and_lag_does_not_queue_clicks() {
        let mut clicker = Clicker::default();
        let policy = Some(Policy {
            min: 8.0,
            max: 12.0,
        });
        let mut now = 0.0;
        assert!(!clicker.tick(now, (1, 0), true, true, false, policy));
        let mut intervals = Vec::new();
        for _ in 0..100 {
            let next = clicker.next.unwrap();
            let interval = next - now;
            assert!((1.0 / 12.0 - 1e-9..=1.0 / 8.0 + 1e-9).contains(&interval));
            intervals.push(interval);
            assert!(!clicker.tick(next - 0.00001, (1, 0), true, false, false, policy));
            assert!(clicker.tick(next, (1, 0), true, false, false, policy));
            now = next;
        }
        assert!(intervals.windows(2).any(|pair| pair[0] != pair[1]));
        assert!(clicker.tick(now + 10.0, (1, 0), true, false, false, policy));
        assert!(!clicker.tick(now + 10.0, (1, 0), true, false, false, policy));
        assert!(!clicker.tick(now + 20.0, (2, 0), true, false, false, policy));
        assert!(!clicker.tick(now + 30.0, (2, 1), true, false, false, policy));
    }
}
