pub const REQUEST: &str = "orbit.autominer";
pub const READY: &str = "orbit.autominer-ready";
pub const CAPABILITY: u8 = 128;

pub const ORES: &[(&str, u8)] = &[
    ("minecraft:diamond_ore", 1),
    ("minecraft:deepslate_diamond_ore", 1),
    ("minecraft:iron_ore", 2),
    ("minecraft:deepslate_iron_ore", 2),
    ("minecraft:gold_ore", 3),
    ("minecraft:deepslate_gold_ore", 3),
    ("minecraft:nether_gold_ore", 3),
    ("minecraft:redstone_ore", 4),
    ("minecraft:deepslate_redstone_ore", 4),
    ("minecraft:lit_redstone_ore", 4),
    ("minecraft:lit_deepslate_redstone_ore", 4),
    ("minecraft:coal_ore", 5),
    ("minecraft:deepslate_coal_ore", 5),
    ("minecraft:emerald_ore", 6),
    ("minecraft:deepslate_emerald_ore", 6),
    ("minecraft:lapis_ore", 6),
    ("minecraft:deepslate_lapis_ore", 6),
    ("minecraft:copper_ore", 6),
    ("minecraft:deepslate_copper_ore", 6),
    ("minecraft:quartz_ore", 6),
    ("minecraft:ancient_debris", 7),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Policy {
    pub nearby: bool,
    pub ore: u8,
    pub range: f32,
    pub delay_ms: u32,
    pub auto_tool: bool,
    pub protect_tools: bool,
    pub durability: u8,
}

impl Policy {
    pub fn parse(values: &[f32]) -> Option<Self> {
        let [
            version,
            nearby,
            ore,
            range,
            delay,
            auto_tool,
            protect,
            durability,
        ] = values
        else {
            return None;
        };
        let flag = |value: f32| match value {
            0.0 => Some(false),
            1.0 => Some(true),
            _ => None,
        };
        let integer = |value: f32, min: f32, max: f32| {
            value.is_finite() && (min..=max).contains(&value) && value.fract() == 0.0
        };
        if *version != 1.0
            || !integer(*ore, 0.0, 7.0)
            || !range.is_finite()
            || !(1.0..=4.0).contains(range)
            || !integer(*delay, 0.0, 1000.0)
            || !integer(*durability, 1.0, 25.0)
        {
            return None;
        }
        Some(Self {
            nearby: flag(*nearby)?,
            ore: *ore as u8,
            range: *range,
            delay_ms: *delay as u32,
            auto_tool: flag(*auto_tool)?,
            protect_tools: flag(*protect)?,
            durability: *durability as u8,
        })
    }
    pub fn values(self) -> [f32; 8] {
        [
            1.0,
            u8::from(self.nearby) as f32,
            self.ore as f32,
            self.range,
            self.delay_ms as f32,
            u8::from(self.auto_tool) as f32,
            u8::from(self.protect_tools) as f32,
            self.durability as f32,
        ]
    }
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        ORES.iter()
            .filter(move |(_, ore)| self.ore == 0 || self.ore == *ore)
            .map(|(name, _)| *name)
    }
}

#[derive(Default)]
pub struct Delay {
    scope: Option<(u64, i32)>,
    target: Option<([i32; 3], u32)>,
    ready_at: f64,
    last: f64,
}

impl Delay {
    pub fn ready(
        &mut self,
        now: f64,
        scope: (u64, i32),
        target: Option<([i32; 3], u32)>,
        delay_ms: u32,
    ) -> bool {
        if !now.is_finite() || now < 0.0 {
            self.reset();
            return false;
        }
        if self.scope != Some(scope) || now < self.last {
            self.reset();
            self.scope = Some(scope);
        }
        self.last = now;
        if self.target != target {
            self.target = target;
            self.ready_at = now + f64::from(delay_ms.min(1000)) / 1000.0;
        }
        target.is_some() && now >= self.ready_at
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

pub fn scan_window(count: usize, cursor: usize) -> impl Iterator<Item = usize> {
    (0..count.min(16)).map(move |offset| (cursor + offset) % count)
}

pub fn tool_safe(maximum: Option<u32>, damage: Option<u32>, threshold: u8) -> bool {
    match (maximum, damage) {
        (None, _) => true,
        (Some(0), _) | (Some(_), None) => false,
        (Some(maximum), Some(damage)) => {
            u64::from(maximum.saturating_sub(damage)) * 100
                > u64::from(maximum) * u64::from(threshold)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> Policy {
        Policy {
            nearby: false,
            ore: 0,
            range: 4.0,
            delay_ms: 100,
            auto_tool: true,
            protect_tools: true,
            durability: 10,
        }
    }
    #[test]
    fn protocol_rejects_invalid_flags_ranges_and_versions() {
        let values = policy().values();
        assert_eq!(Policy::parse(&values), Some(policy()));
        for (index, value) in [
            (0, 2.0),
            (1, 0.5),
            (2, 11.0),
            (3, f32::NAN),
            (3, 4.5),
            (4, 1001.0),
            (5, 2.0),
            (6, -1.0),
            (7, 0.0),
        ] {
            let mut bad = values;
            bad[index] = value;
            assert_eq!(Policy::parse(&bad), None);
        }
        assert_eq!(Policy::parse(&[]), None);
    }
    #[test]
    fn ore_filter_keeps_variants_and_excludes_other_groups() {
        let all = policy().names().collect::<Vec<_>>();
        assert!(all.contains(&"minecraft:ancient_debris"));
        let diamond = Policy { ore: 1, ..policy() }.names().collect::<Vec<_>>();
        assert_eq!(
            diamond,
            vec!["minecraft:diamond_ore", "minecraft:deepslate_diamond_ore"]
        );
        assert!(!all.contains(&"minecraft:chest"));
    }
    #[test]
    fn delay_resets_on_new_blocks_world_changes_and_reentry() {
        let mut delay = Delay::default();
        let block = Some(([1, 2, 3], 5));
        assert!(!delay.ready(1.0, (1, 0), block, 100));
        assert!(delay.ready(1.11, (1, 0), block, 100));
        assert!(!delay.ready(1.2, (1, 0), Some(([1, 2, 3], 6)), 100));
        assert!(!delay.ready(2.0, (1, 1), block, 100));
        assert!(!delay.ready(3.0, (2, 1), block, 100));
        assert!(!delay.ready(4.0, (2, 1), None, 100));
        assert!(!delay.ready(5.0, (2, 1), block, 100));
        assert!(!delay.ready(f64::NAN, (2, 1), block, 100));
    }
    #[test]
    fn bounded_scan_windows_do_not_starve_occluded_candidates() {
        let mut visited = std::collections::BTreeSet::new();
        for cursor in [0, 16, 32] {
            let window = scan_window(39, cursor).collect::<Vec<_>>();
            assert!(window.len() <= 16);
            visited.extend(window);
        }
        assert_eq!(visited.len(), 39);
        assert_eq!(scan_window(0, 0).count(), 0);
        assert_eq!(scan_window(2, 0).collect::<Vec<_>>(), vec![0, 1]);
    }
    #[test]
    fn durability_protection_uses_remaining_percentage_and_unknown_damage_pauses() {
        assert!(tool_safe(Some(100), Some(89), 10));
        assert!(!tool_safe(Some(100), Some(90), 10));
        assert!(!tool_safe(Some(100), None, 10));
        assert!(!tool_safe(Some(100), Some(120), 10));
        assert!(tool_safe(None, None, 10));
    }
}
