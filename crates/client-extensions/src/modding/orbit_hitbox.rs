pub const REQUEST: &str = "orbit.hitbox-policy";
pub const READY: &str = "orbit.hitbox-ready";
pub const CAPABILITY: u8 = 16;
pub const MAX_PADDING: f32 = 1.0;
pub const MAX_RANGE: f32 = 64.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitboxPolicy {
    pub expand: bool,
    pub outlines: bool,
    pub players: bool,
    pub mobs: bool,
    pub padding: f32,
    pub range: f32,
}

impl HitboxPolicy {
    pub fn parse(values: &[f32]) -> Option<Self> {
        let [version, expand, outlines, players, mobs, padding, range] = values else {
            return None;
        };
        let flag = |value| match value {
            0.0 => Some(false),
            1.0 => Some(true),
            _ => None,
        };
        if *version != 1.0
            || !padding.is_finite()
            || !(0.0..=MAX_PADDING).contains(padding)
            || !range.is_finite()
            || !(4.0..=MAX_RANGE).contains(range)
        {
            return None;
        }
        Some(Self {
            expand: flag(*expand)?,
            outlines: flag(*outlines)?,
            players: flag(*players)?,
            mobs: flag(*mobs)?,
            padding: *padding,
            range: *range,
        })
    }

    pub fn values(self) -> [f32; 7] {
        [
            1.0,
            u8::from(self.expand) as f32,
            u8::from(self.outlines) as f32,
            u8::from(self.players) as f32,
            u8::from(self.mobs) as f32,
            self.padding,
            self.range,
        ]
    }

    pub fn applies(self, player: bool, position: [f32; 3], origin: [f32; 3]) -> bool {
        (if player { self.players } else { self.mobs })
            && position
                .iter()
                .chain(origin.iter())
                .all(|value| value.is_finite())
            && (0..3)
                .map(|axis| (position[axis] - origin[axis]).powi(2))
                .sum::<f32>()
                <= self.range.powi(2)
    }

    pub fn bounds(self, min: [f32; 3], max: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        let padding = if self.expand { self.padding } else { 0.0 };
        (
            min.map(|value| value - padding),
            max.map(|value| value + padding),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> HitboxPolicy {
        HitboxPolicy {
            expand: true,
            outlines: true,
            players: true,
            mobs: false,
            padding: 0.25,
            range: 32.0,
        }
    }
    #[test]
    fn malformed_requests_cannot_change_hitboxes() {
        let values = policy().values();
        assert_eq!(HitboxPolicy::parse(&values), Some(policy()));
        assert_eq!(HitboxPolicy::parse(&values[..6]), None);
        for (index, value) in [
            (0, 2.0),
            (1, 0.5),
            (5, f32::NAN),
            (5, -1.0),
            (5, 1.1),
            (6, 65.0),
        ] {
            let mut bad = values;
            bad[index] = value;
            assert_eq!(HitboxPolicy::parse(&bad), None);
        }
    }
    #[test]
    fn filters_types_distance_and_nonfinite_positions() {
        assert!(policy().applies(true, [0.0; 3], [0.0; 3]));
        assert!(!policy().applies(false, [0.0; 3], [0.0; 3]));
        assert!(!policy().applies(true, [33.0, 0.0, 0.0], [0.0; 3]));
        assert!(!policy().applies(true, [f32::NAN; 3], [0.0; 3]));
    }
    #[test]
    fn outlines_use_original_bounds_when_expansion_is_disabled() {
        let min = [0.0; 3];
        let max = [1.0; 3];
        assert_eq!(policy().bounds(min, max), ([-0.25; 3], [1.25; 3]));
        assert_eq!(
            HitboxPolicy {
                expand: false,
                ..policy()
            }
            .bounds(min, max),
            (min, max)
        );
    }
}
