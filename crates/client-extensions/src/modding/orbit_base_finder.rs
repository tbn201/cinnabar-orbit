pub const REQUEST: &str = "orbit.base-finder";
pub const READY: &str = "orbit.base-finder-ready";
pub const BASE: &str = "orbit.base";
pub const CAPABILITY: u16 = 16384;
pub const MAX_BASES: usize = 8;
pub const MIN_SENSITIVITY: f32 = 1.0;
pub const MAX_SENSITIVITY: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Policy {
    pub sensitivity: f32,
}

impl Policy {
    pub fn parse(values: &[f32]) -> Option<Self> {
        let [version, sensitivity] = values else {
            return None;
        };
        (*version == 1.0
            && sensitivity.is_finite()
            && (MIN_SENSITIVITY..=MAX_SENSITIVITY).contains(sensitivity)
            && sensitivity.fract() == 0.0)
            .then_some(Self { sensitivity: *sensitivity })
    }

    pub fn values(self) -> [f32; 2] {
        [1.0, self.sensitivity]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Base {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub score: u32,
    pub storage: u32,
}

impl Base {
    pub fn parse(values: &[f32]) -> Option<Self> {
        let [version, x, y, z, score, storage] = values else {
            return None;
        };
        let whole = |value: f32, low: f32, high: f32| {
            (value.is_finite() && value.fract() == 0.0 && (low..=high).contains(&value)).then_some(value)
        };
        if *version != 1.0 {
            return None;
        }
        Some(Self {
            x: whole(*x, -30_000_000.0, 30_000_000.0)? as i32,
            y: whole(*y, -64.0, 320.0)? as i32,
            z: whole(*z, -30_000_000.0, 30_000_000.0)? as i32,
            score: whole(*score, 0.0, 1_000_000.0)? as u32,
            storage: whole(*storage, 0.0, 1_000_000.0)? as u32,
        })
    }

    pub fn values(self) -> [f32; 6] {
        [1.0, self.x as f32, self.y as f32, self.z as f32, self.score as f32, self.storage as f32]
    }
}

pub fn decode<'a>(cues: impl Iterator<Item = (&'a str, &'a [f32])>) -> Vec<Base> {
    cues.filter(|(name, _)| *name == BASE)
        .filter_map(|(_, values)| Base::parse(values))
        .take(MAX_BASES)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_round_trips_and_rejects_bad_values() {
        let policy = Policy { sensitivity: 5.0 };
        assert_eq!(Policy::parse(&policy.values()), Some(policy));
        for values in [vec![2.0, 5.0], vec![1.0, 0.0], vec![1.0, 11.0], vec![1.0, 2.5]] {
            assert_eq!(Policy::parse(&values), None);
        }
    }

    #[test]
    fn bases_round_trip_and_bad_rows_are_dropped() {
        let base = Base { x: -1200, y: -40, z: 380, score: 57, storage: 12 };
        let good = base.values();
        let bad = [1.0, 0.5, 0.0, 0.0, 1.0, 1.0];
        let cues = [(BASE, &good[..]), (BASE, &bad[..]), ("orbit.other", &good[..])];
        assert_eq!(decode(cues.into_iter()), vec![base]);
    }

    #[test]
    fn at_most_eight_bases_are_kept() {
        let rows: Vec<[f32; 6]> = (0..12).map(|i| Base { x: i, y: 0, z: 0, score: 1, storage: 0 }.values()).collect();
        assert_eq!(decode(rows.iter().map(|row| (BASE, &row[..]))).len(), MAX_BASES);
    }
}
