pub const REQUEST: &str = "orbit.auto-totem";
pub const READY: &str = "orbit.auto-totem-ready";
pub const CAPABILITY: u16 = 256;
pub const MIN_HEALTH: f32 = 1.0;
pub const MAX_HEALTH: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Policy {
    pub health: f32,
}

impl Policy {
    pub fn parse(values: &[f32]) -> Option<Self> {
        let [version, health] = values else {
            return None;
        };
        (*version == 1.0
            && health.is_finite()
            && (MIN_HEALTH..=MAX_HEALTH).contains(health)
            && health.fract() == 0.0)
            .then_some(Self { health: *health })
    }

    pub fn values(self) -> [f32; 2] {
        [1.0, self.health]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_round_trips_and_rejects_bad_values() {
        let policy = Policy { health: 12.0 };
        assert_eq!(Policy::parse(&policy.values()), Some(policy));
        for values in [
            vec![2.0, 12.0],
            vec![1.0, 0.0],
            vec![1.0, 21.0],
            vec![1.0, 12.5],
            vec![1.0, f32::NAN],
            vec![1.0],
        ] {
            assert_eq!(Policy::parse(&values), None);
        }
    }
}
