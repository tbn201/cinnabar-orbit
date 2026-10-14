pub const REQUEST: &str = "orbit.fly";
pub const READY: &str = "orbit.fly-ready";
pub const CAPABILITY: u16 = 1024;
pub const MIN_SPEED: f32 = 1.0;
pub const MAX_SPEED: f32 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Policy {
    pub speed: f32,
}

impl Policy {
    pub fn parse(values: &[f32]) -> Option<Self> {
        let [version, speed] = values else {
            return None;
        };
        (*version == 1.0
            && speed.is_finite()
            && (MIN_SPEED..=MAX_SPEED).contains(speed)
            && speed.fract() == 0.0)
            .then_some(Self { speed: *speed })
    }

    pub fn values(self) -> [f32; 2] {
        [1.0, self.speed]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_round_trips_and_rejects_bad_values() {
        let policy = Policy { speed: 11.0 };
        assert_eq!(Policy::parse(&policy.values()), Some(policy));
        for values in [
            vec![3.0, 11.0],
            vec![1.0, 0.0],
            vec![1.0, 31.0],
            vec![1.0, 1.5],
            vec![1.0, f32::NAN],
            vec![1.0, 1.0, 1.0],
        ] {
            assert_eq!(Policy::parse(&values), None);
        }
    }
}
