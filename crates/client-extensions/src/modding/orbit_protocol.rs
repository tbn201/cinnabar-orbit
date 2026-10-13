pub const POLICY: &str = "orbit.assist-policy";
pub const READY: &str = "orbit.assist-ready";
pub const COSMETICS: &str = "orbit.closet-open";
pub const HOTBAR: u8 = 1;
pub const SPRINT: u8 = 2;
pub const CLOSET: u8 = 4;
pub const DEBOUNCE: u8 = 8;
pub const DC_PREVENT: &str = "orbit.dc-prevent";

pub fn debounce_delay(values: &[f32]) -> Option<f32> {
    let [version, delay] = values else {
        return None;
    };
    (*version == 1.0 && delay.is_finite() && (1.0..=100.0).contains(delay) && delay.fract() == 0.0)
        .then_some(*delay)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    pub auto_stack: bool,
    pub auto_tool: bool,
    pub toggle_sprint: bool,
}

impl Policy {
    pub fn values(self) -> [f32; 4] {
        [
            1.0,
            u8::from(self.auto_stack) as f32,
            u8::from(self.auto_tool) as f32,
            u8::from(self.toggle_sprint) as f32,
        ]
    }

    pub fn parse(values: &[f32]) -> Option<Self> {
        let [auto_stack, auto_tool, toggle_sprint] = flags(values)?;
        Some(Self {
            auto_stack,
            auto_tool,
            toggle_sprint,
        })
    }

    pub fn allowed(self, capabilities: u8) -> Self {
        Self {
            auto_stack: self.auto_stack && capabilities & HOTBAR != 0,
            auto_tool: self.auto_tool && capabilities & HOTBAR != 0,
            toggle_sprint: self.toggle_sprint && capabilities & SPRINT != 0,
        }
    }
}

fn flags(values: &[f32]) -> Option<[bool; 3]> {
    let [version, first, second, third] = values else {
        return None;
    };
    if *version != 1.0 {
        return None;
    }
    let flag = |value: f32| match value {
        0.0 => Some(false),
        1.0 => Some(true),
        _ => None,
    };
    Some([flag(*first)?, flag(*second)?, flag(*third)?])
}

pub fn capabilities(values: &[f32]) -> u8 {
    let (base, debounce) = match values {
        [version, hotbar, sprint, closet, debounce]
            if *version == 2.0 && (*debounce == 0.0 || *debounce == 1.0) =>
        {
            ([1.0, *hotbar, *sprint, *closet], *debounce == 1.0)
        }
        [version, hotbar, sprint, closet] => ([*version, *hotbar, *sprint, *closet], false),
        _ => return 0,
    };
    let Some([hotbar, sprint, closet]) = flags(&base) else {
        return 0;
    };
    (u8::from(hotbar) * HOTBAR)
        | (u8::from(sprint) * SPRINT)
        | (u8::from(closet) * CLOSET)
        | (u8::from(debounce) * DEBOUNCE)
}

pub fn ready_values(capabilities: u8) -> [f32; 5] {
    [
        2.0,
        f32::from(u8::from(capabilities & HOTBAR != 0)),
        f32::from(u8::from(capabilities & SPRINT != 0)),
        f32::from(u8::from(capabilities & CLOSET != 0)),
        f32::from(u8::from(capabilities & DEBOUNCE != 0)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_policy_rejects_unknown_versions_and_invalid_flags() {
        let policy = Policy {
            auto_stack: true,
            auto_tool: false,
            toggle_sprint: true,
        };
        assert_eq!(Policy::parse(&policy.values()), Some(policy));
        for values in [
            [2.0, 1.0, 0.0, 0.0],
            [1.0, 0.5, 0.0, 0.0],
            [1.0, f32::NAN, 0.0, 0.0],
        ] {
            assert_eq!(Policy::parse(&values), None);
            assert_eq!(capabilities(&values), 0);
        }
        assert_eq!(Policy::parse(&[]), None);
        assert_eq!(
            capabilities(&[1.0, 1.0, 1.0, 1.0]),
            HOTBAR | SPRINT | CLOSET
        );
    }
    #[test]
    fn permissions_mask_each_feature_without_expanding_authority() {
        let policy = Policy {
            auto_stack: true,
            auto_tool: true,
            toggle_sprint: true,
        };
        for bits in 0..16 {
            assert_eq!(capabilities(&ready_values(bits)), bits);
            let allowed = policy.allowed(bits);
            assert_eq!(allowed.auto_stack, bits & HOTBAR != 0);
            assert_eq!(allowed.auto_tool, bits & HOTBAR != 0);
            assert_eq!(allowed.toggle_sprint, bits & SPRINT != 0);
        }
    }
}

#[cfg(test)]
mod debounce_tests {
    use super::*;

    #[test]
    fn debounce_wire_requires_a_versioned_finite_bounded_integer() {
        assert_eq!(debounce_delay(&[1.0, 20.0]), Some(20.0));
        for values in [
            [2.0, 20.0],
            [1.0, 0.0],
            [1.0, 101.0],
            [1.0, 1.5],
            [1.0, f32::NAN],
        ] {
            assert_eq!(debounce_delay(&values), None);
        }
        assert_eq!(debounce_delay(&[]), None);
        assert_eq!(capabilities(&[2.0, 1.0, 1.0, 1.0, 0.5]), 0);
        assert_eq!(
            capabilities(&[1.0, 1.0, 1.0, 1.0]),
            HOTBAR | SPRINT | CLOSET
        );
    }
}

#[cfg(test)]
mod event_name_tests {
    use super::*;

    #[test]
    fn bridge_event_names_fit_the_hosts_wire_alphabet_and_budget() {
        for name in [POLICY, READY, COSMETICS, DC_PREVENT] {
            assert!(name.len() <= 32);
            assert!(name.bytes().all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || b"._-".contains(&byte)));
        }
    }
}
