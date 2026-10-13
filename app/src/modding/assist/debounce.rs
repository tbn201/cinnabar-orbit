#[derive(Default)]
pub struct Debounce {
    accepted: [Option<f64>; 2],
}

impl Debounce {
    pub fn reject(&mut self, button: usize, now: f64, delay_ms: f32) -> bool {
        let Some(last) = self.accepted.get_mut(button) else {
            return false;
        };
        if !now.is_finite() || !delay_ms.is_finite() || !(1.0..=100.0).contains(&delay_ms) {
            *last = None;
            return false;
        }
        if last.is_some_and(|previous| {
            now >= previous && (now - previous) * 1000.0 + 1e-9 < f64::from(delay_ms)
        }) {
            return true;
        }
        *last = Some(now);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_bounce_without_extending_the_window_and_buttons_are_independent() {
        let mut filter = Debounce::default();
        assert!(!filter.reject(0, 1.0, 20.0));
        assert!(filter.reject(0, 1.005, 20.0));
        assert!(filter.reject(0, 1.019, 20.0));
        assert!(!filter.reject(1, 1.019, 20.0));
        assert!(!filter.reject(0, 1.020, 20.0));
        assert!(filter.reject(1, 1.020, 20.0));
    }

    #[test]
    fn reset_and_clock_changes_allow_the_next_press() {
        let mut filter = Debounce::default();
        assert!(!filter.reject(0, 100.0, 20.0));
        assert!(!filter.reject(0, 1.0, 20.0));
        filter = Debounce::default();
        assert!(!filter.reject(0, 1.001, 20.0));
        assert!(!filter.reject(0, f64::NAN, 20.0));
        assert!(!filter.reject(0, 1.002, 20.0));
    }
}
