use bevy::prelude::{EulerRot, Quat, Vec3};
use client_presentation::local_player::LocalViewPose;
use client_state::SemanticInputSnapshot;
use semantic_input::Action;

const TICKS_PER_SECOND: f32 = 20.0;
const MAX_FRAME_SECONDS: f32 = 0.1;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct MotionPolicy {
    pub freecam: Option<f32>,
    pub fly: Option<f32>,
    pub no_fall: bool,
    pub spider: bool,
    pub step: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Steering {
    pub movement: [f32; 2],
    pub ascend: bool,
    pub descend: bool,
}

impl Steering {
    pub(crate) fn read(input: &SemanticInputSnapshot) -> Self {
        Self {
            movement: input.movement(),
            ascend: input.phase(Action::Jump).held,
            descend: input.phase(Action::Sneak).held,
        }
    }

    fn vertical(self) -> f32 {
        f32::from(u8::from(self.ascend)) - f32::from(u8::from(self.descend))
    }

    pub(crate) fn moving(self) -> bool {
        self.movement != [0.0, 0.0]
    }
}

impl MotionPolicy {
    pub(crate) fn assist(self, rotation: Quat, steering: Steering) -> sim::MovementAssist {
        let fly_velocity = self.fly.map(|speed| {
            if self.freecam.is_some() {
                [0.0; 3]
            } else {
                fly_velocity(rotation, steering, speed)
            }
        });
        let grounded = fly_velocity.is_none();
        sim::MovementAssist {
            fly_velocity,
            step_height: self
                .step
                .filter(|_| grounded)
                .map(|height| f64::from(height).min(sim::MovementAssist::MAX_STEP_HEIGHT)),
            wall_climb: self.spider && grounded && self.freecam.is_none() && steering.moving(),
            no_fall: self.no_fall,
        }
    }
}

pub(crate) fn fly_velocity(rotation: Quat, steering: Steering, speed: f32) -> [f64; 3] {
    let (yaw, _, _) = rotation.to_euler(EulerRot::YXZ);
    let heading = Quat::from_rotation_y(yaw);
    let [right, forward] = steering.movement;
    let horizontal = (heading * Vec3::X * right + heading * Vec3::NEG_Z * forward).normalize_or_zero();
    let per_tick = speed / TICKS_PER_SECOND;
    let velocity = (horizontal + Vec3::Y * steering.vertical()) * per_tick;
    [velocity.x, velocity.y, velocity.z].map(f64::from)
}

pub(crate) fn freecam_offset(rotation: Quat, steering: Steering, speed: f32, seconds: f32) -> Vec3 {
    let [right, forward] = steering.movement;
    let direction =
        (rotation * Vec3::X * right + rotation * Vec3::NEG_Z * forward + Vec3::Y * steering.vertical())
            .normalize_or_zero();
    direction * speed * seconds.clamp(0.0, MAX_FRAME_SECONDS)
}

#[derive(Default)]
pub(super) struct Freecam {
    eye: Option<Vec3>,
}

impl Freecam {
    pub(super) fn update(
        &mut self,
        speed: Option<f32>,
        view: Option<&LocalViewPose>,
        steering: Steering,
        seconds: f32,
    ) -> Option<Vec3> {
        let (Some(speed), Some(view)) = (speed, view) else {
            self.eye = None;
            return None;
        };
        let eye = match self.eye {
            Some(eye) => eye + freecam_offset(view.camera_rotation(), steering, speed, seconds),
            None => view.eye_translation(),
        };
        self.eye = eye.is_finite().then_some(eye);
        self.eye
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steering(movement: [f32; 2], ascend: bool, descend: bool) -> Steering {
        Steering {
            movement,
            ascend,
            descend,
        }
    }

    fn close(actual: [f64; 3], expected: [f64; 3]) -> bool {
        actual
            .iter()
            .zip(expected)
            .all(|(actual, expected)| (actual - expected).abs() < 1.0e-5)
    }

    #[test]
    fn fly_moves_along_look_yaw_at_the_requested_speed_and_hovers_without_input() {
        let facing_west = Quat::from_euler(EulerRot::YXZ, std::f32::consts::FRAC_PI_2, -0.7, 0.0);
        let forward = fly_velocity(facing_west, steering([0.0, 1.0], false, false), 20.0);
        assert!(close(forward, [-1.0, 0.0, 0.0]), "{forward:?}");
        let diagonal = fly_velocity(Quat::IDENTITY, steering([1.0, 1.0], true, false), 10.0);
        let horizontal = (diagonal[0].powi(2) + diagonal[2].powi(2)).sqrt();
        assert!((horizontal - 0.5).abs() < 1.0e-5);
        assert!((diagonal[1] - 0.5).abs() < 1.0e-6);
        let hover = fly_velocity(Quat::IDENTITY, steering([0.0, 0.0], true, true), 30.0);
        assert!(close(hover, [0.0; 3]));
        let descend = fly_velocity(Quat::IDENTITY, steering([0.0, 0.0], false, true), 30.0);
        assert!(close(descend, [0.0, -1.5, 0.0]));
    }

    #[test]
    fn freecam_follows_camera_pitch_and_is_frame_rate_independent() {
        let looking_down = Quat::from_euler(EulerRot::YXZ, 0.0, -std::f32::consts::FRAC_PI_2, 0.0);
        let offset = freecam_offset(looking_down, steering([0.0, 1.0], false, false), 10.0, 0.1);
        assert!((offset - Vec3::new(0.0, -1.0, 0.0)).length() < 1.0e-4, "{offset:?}");
        let rotation = Quat::from_euler(EulerRot::YXZ, 0.4, 0.3, 0.0);
        let input = steering([0.3, -1.0], true, false);
        let whole = freecam_offset(rotation, input, 12.0, 0.05);
        let halves = freecam_offset(rotation, input, 12.0, 0.025) * 2.0;
        assert!((whole - halves).length() < 1.0e-5);
        assert!((whole.length() - 0.6).abs() < 1.0e-5);
        assert!((freecam_offset(rotation, input, 12.0, 5.0).length() - 1.2).abs() < 1.0e-5);
    }

    #[test]
    fn freecam_starts_at_the_eye_and_releases_when_the_request_stops() {
        let view = LocalViewPose::new(Vec3::new(1.0, 70.0, 2.0), Quat::IDENTITY);
        let mut freecam = Freecam::default();
        let up = steering([0.0, 0.0], true, false);
        assert_eq!(freecam.update(Some(4.0), Some(&view), up, 0.05), Some(view.eye_translation()));
        let moved = freecam.update(Some(4.0), Some(&view), up, 0.05).unwrap();
        assert!((moved - Vec3::new(1.0, 70.2, 2.0)).length() < 1.0e-4);
        assert_eq!(freecam.update(None, Some(&view), up, 0.05), None);
        assert_eq!(freecam.update(Some(4.0), Some(&view), up, 0.05), Some(view.eye_translation()));
    }

    #[test]
    fn assist_composes_freecam_fly_spider_step_and_no_fall() {
        let walk = steering([0.0, 1.0], false, false);
        let all = MotionPolicy {
            freecam: Some(10.0),
            fly: Some(10.0),
            no_fall: true,
            spider: true,
            step: Some(2.0),
        };
        let assist = all.assist(Quat::IDENTITY, walk);
        assert_eq!(assist.fly_velocity, Some([0.0; 3]));
        assert!(assist.no_fall && !assist.wall_climb && assist.step_height.is_none());
        let grounded = MotionPolicy {
            fly: None,
            freecam: None,
            ..all
        };
        let assist = grounded.assist(Quat::IDENTITY, walk);
        assert!(assist.wall_climb && assist.fly_velocity.is_none());
        assert_eq!(assist.step_height, Some(2.0));
        let idle = grounded.assist(Quat::IDENTITY, Steering::default());
        assert!(!idle.wall_climb);
        assert_eq!(MotionPolicy::default().assist(Quat::IDENTITY, walk), sim::MovementAssist::default());
    }
}
