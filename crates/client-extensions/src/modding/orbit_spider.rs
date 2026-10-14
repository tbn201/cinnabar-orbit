pub const REQUEST: &str = "orbit.spider";
pub const READY: &str = "orbit.spider-ready";
pub const CAPABILITY: u16 = 4096;

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
        for values in [vec![], vec![2.0], vec![1.0, 1.0], vec![f32::NAN]] {
            assert!(!requested(&values));
        }
    }
}
