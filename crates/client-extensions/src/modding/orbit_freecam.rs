pub const REQUEST: &str = "orbit.freecam";
pub const READY: &str = "orbit.freecam-ready";
pub const CAPABILITY: u16 = 512;
pub const MIN_SPEED: f32 = 1.0;
pub const MAX_SPEED: f32 = 40.0;

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
        let policy = Policy { speed: 10.0 };
        assert_eq!(Policy::parse(&policy.values()), Some(policy));
        for values in [
            vec![2.0, 10.0],
            vec![1.0, 0.0],
            vec![1.0, 41.0],
            vec![1.0, 2.5],
            vec![1.0, f32::INFINITY],
            vec![],
        ] {
            assert_eq!(Policy::parse(&values), None);
        }
    }
}
