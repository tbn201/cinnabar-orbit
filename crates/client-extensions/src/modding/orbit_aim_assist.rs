pub const REQUEST: &str = "orbit.aim-assist";
pub const READY: &str = "orbit.aim-assist-ready";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Policy {
    pub players: bool,
    pub mobs: bool,
    pub range: f32,
    pub fov: f32,
    pub strength: f32,
    pub held_only: bool,
    pub nearest: bool,
}

impl Policy {
    pub fn parse(values: &[f32]) -> Option<Self> {
        let [version, players, mobs, range, fov, strength, held, nearest] = values else {
            return None;
        };
        let flag = |value| match value {
            0.0 => Some(false),
            1.0 => Some(true),
            _ => None,
        };
        if *version != 1.0
            || !range.is_finite()
            || !(1.0..=7.0).contains(range)
            || !fov.is_finite()
            || !(10.0..=120.0).contains(fov)
            || !strength.is_finite()
            || !(0.1..=1.0).contains(strength)
        {
            return None;
        }
        Some(Self {
            players: flag(*players)?,
            mobs: flag(*mobs)?,
            range: *range,
            fov: *fov,
            strength: *strength,
            held_only: flag(*held)?,
            nearest: flag(*nearest)?,
        })
    }
    pub fn values(self) -> [f32; 8] {
        [
            1.0,
            u8::from(self.players) as f32,
            u8::from(self.mobs) as f32,
            self.range,
            self.fov,
            self.strength,
            u8::from(self.held_only) as f32,
            u8::from(self.nearest) as f32,
        ]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub id: u64,
    pub min: [f32; 3],
    pub max: [f32; 3],
}
#[derive(Clone, Copy, Debug)]
pub struct Choice {
    pub id: u64,
    pub point: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    score: f32,
}

pub fn choose(
    policy: Policy,
    eye: [f32; 3],
    rotation: [f32; 2],
    locked: Option<u64>,
    targets: &[Target],
    mut visible: impl FnMut(&Choice) -> bool,
) -> Option<Choice> {
    if !eye.into_iter().chain(rotation).all(f32::is_finite) {
        return None;
    }
    let mut choices = targets
        .iter()
        .filter_map(|target| {
            if target.id == 0
                || !(0..3).all(|axis| {
                    target.min[axis].is_finite()
                        && target.max[axis].is_finite()
                        && target.min[axis] < target.max[axis]
                })
            {
                return None;
            }
            let height = target.max[1] - target.min[1];
            let point = [
                (target.min[0] + target.max[0]) * 0.5,
                eye[1].clamp(target.min[1] + height * 0.2, target.max[1] - height * 0.2),
                (target.min[2] + target.max[2]) * 0.5,
            ];
            let [x, y, z] = std::array::from_fn(|i| point[i] - eye[i]);
            let distance = (x * x + y * y + z * z).sqrt();
            if !distance.is_finite() || distance <= 0.001 || distance > policy.range {
                return None;
            }
            let yaw = ((-x).atan2(-z) - rotation[0] + std::f32::consts::PI)
                .rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            let pitch = y.atan2(x.hypot(z)) - rotation[1];
            let angle = yaw.hypot(pitch);
            if angle > policy.fov.to_radians() * 0.5 {
                return None;
            }
            Some(Choice {
                id: target.id,
                point,
                yaw,
                pitch,
                distance,
                score: if policy.nearest { distance } else { angle },
            })
        })
        .collect::<Vec<_>>();
    choices.sort_by(|a, b| a.score.total_cmp(&b.score).then(a.id.cmp(&b.id)));
    let locked = choices
        .iter()
        .find(|choice| Some(choice.id) == locked)
        .copied();
    if let Some(locked) = locked {
        if choices
            .first()
            .is_some_and(|best| locked.score <= best.score * 1.35 + 0.002)
            && visible(&locked)
        {
            return Some(locked);
        }
    }
    choices.into_iter().take(16).find(|choice| visible(choice))
}

pub fn turn(choice: Choice, seconds: f32, strength: f32, maximum: f32) -> [f32; 2] {
    if !seconds.is_finite() || seconds <= 0.0 {
        return [0.0; 2];
    }
    let dt = seconds.min(0.1);
    let gain = 1.0 - (-dt * (4.0 + 20.0 * strength.clamp(0.1, 1.0))).exp();
    let limit = maximum
        .min(dt * (0.5 + 4.0 * strength.clamp(0.1, 1.0)))
        .max(0.0);
    [
        (choice.yaw * gain).clamp(-limit, limit),
        (choice.pitch * gain).clamp(-limit, limit),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> Policy {
        Policy {
            players: true,
            mobs: false,
            range: 7.0,
            fov: 120.0,
            strength: 0.5,
            held_only: true,
            nearest: false,
        }
    }
    fn target(id: u64, x: f32, z: f32) -> Target {
        Target {
            id,
            min: [x - 0.3, 0.0, z - 0.3],
            max: [x + 0.3, 1.8, z + 0.3],
        }
    }
    #[test]
    fn chooses_actual_box_points_and_skips_occluded_or_outside_targets() {
        let targets = [
            target(1, 0.0, -3.0),
            target(2, 0.5, -3.0),
            target(3, 0.0, 3.0),
            target(4, 0.0, -8.0),
        ];
        let chosen = choose(
            policy(),
            [0.0, 1.62, 0.0],
            [0.0, 0.0],
            None,
            &targets,
            |choice| choice.id != 1,
        )
        .unwrap();
        assert_eq!(chosen.id, 2);
        assert!(chosen.point[1] <= 1.8 && chosen.point[1] >= 0.0);
        assert!(
            choose(
                policy(),
                [0.0, 1.62, 0.0],
                [0.0, 0.0],
                None,
                &targets,
                |_| false
            )
            .is_none()
        );
    }
    #[test]
    fn target_lock_does_not_flicker_but_a_better_target_can_replace_it() {
        let targets = [target(1, 0.3, -3.0), target(2, 0.32, -3.0)];
        assert_eq!(
            choose(
                policy(),
                [0.0, 1.62, 0.0],
                [0.0, 0.0],
                Some(2),
                &targets,
                |_| true
            )
            .unwrap()
            .id,
            2
        );
        let targets = [target(1, 0.0, -3.0), target(2, 1.0, -3.0)];
        assert_eq!(
            choose(
                policy(),
                [0.0, 1.62, 0.0],
                [0.0, 0.0],
                Some(2),
                &targets,
                |_| true
            )
            .unwrap()
            .id,
            1
        );
    }
    #[test]
    fn invalid_policy_and_geometry_are_rejected_and_turns_are_bounded() {
        let values = policy().values();
        assert_eq!(Policy::parse(&values), Some(policy()));
        for (index, value) in [
            (0, 2.0),
            (1, 0.5),
            (3, 8.0),
            (4, f32::NAN),
            (5, 0.0),
            (6, 2.0),
        ] {
            let mut bad = values;
            bad[index] = value;
            assert_eq!(Policy::parse(&bad), None);
        }
        let choice = choose(
            policy(),
            [0.0, 1.62, 0.0],
            [0.0, 0.0],
            None,
            &[target(1, 1.0, -3.0)],
            |_| true,
        )
        .unwrap();
        for dt in [0.01, 0.02, 1.0] {
            assert!(
                turn(choice, dt, 0.5, 0.03)
                    .into_iter()
                    .all(|value| value.is_finite() && value.abs() <= 0.03)
            );
        }
        assert_eq!(turn(choice, f32::NAN, 0.5, 0.03), [0.0; 2]);
        assert!(
            choose(
                policy(),
                [f32::NAN, 0.0, 0.0],
                [0.0; 2],
                None,
                &[target(1, 0.0, -3.0)],
                |_| true
            )
            .is_none()
        );
    }
}
