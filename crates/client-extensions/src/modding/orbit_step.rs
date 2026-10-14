pub const REQUEST: &str = "orbit.step";
pub const READY: &str = "orbit.step-ready";
pub const CAPABILITY: u16 = 8192;
pub const MIN_HEIGHT: f32 = 1.0;
pub const MAX_HEIGHT: f32 = 2.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Policy {
    pub height: f32,
}

impl Policy {
    pub fn parse(values: &[f32]) -> Option<Self> {
        let [version, height] = values else {
            return None;
        };
        (*version == 1.0
            && height.is_finite()
            && (MIN_HEIGHT..=MAX_HEIGHT).contains(height)
            && (height * 2.0).fract() == 0.0)
            .then_some(Self { height: *height })
    }

    pub fn values(self) -> [f32; 2] {
        [1.0, self.height]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_round_trips_and_rejects_bad_values() {
        let policy = Policy { height: 1.5 };
        assert_eq!(Policy::parse(&policy.values()), Some(policy));
        for values in [
            vec![2.0, 1.0],
            vec![1.0, 0.5],
            vec![1.0, 3.0],
            vec![1.0, 1.25],
            vec![1.0, f32::NAN],
        ] {
            assert_eq!(Policy::parse(&values), None);
        }
    }
}
