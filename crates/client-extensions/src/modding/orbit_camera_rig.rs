pub const REQUEST: &str = "orbit.first-person-rig";
pub const READY: &str = "orbit.first-person-rig-ready";
pub const CAPABILITY: u16 = 32768;

pub fn requested(values: &[f32]) -> bool {
    values == [1.0]
}

pub const fn values() -> [f32; 1] {
    [1.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_current_version_is_accepted() {
        assert!(requested(&values()));
        for values in [vec![], vec![2.0], vec![1.0, 0.0], vec![f32::NAN]] {
            assert!(!requested(&values));
        }
    }
}
