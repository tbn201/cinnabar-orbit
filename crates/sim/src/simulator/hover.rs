use crate::{CollisionWorld, Vec3};

use super::{
    ControlledTickResult, MovementInput, PlayerState, ProcessedControls, STEP_HEIGHT,
    SimulationError, TickResult, collision::resolve_motion, environment::sample,
};

pub(super) const GROUND_HINT_SPEED: f64 = 1.0e-3;

pub(super) fn tick(
    mut next: PlayerState,
    state: &mut PlayerState,
    input: MovementInput,
    controls: ProcessedControls,
    velocity: Vec3,
    world: &impl CollisionWorld,
) -> Result<ControlledTickResult, SimulationError> {
    let height = input.mode.hitbox_height(input.sneaking);
    let sampled = sample(
        world,
        next.position,
        velocity,
        height,
        input.liquid_contact_height,
    )?;
    let collision_box = next.collision_box(height);
    let motion = resolve_motion(world, collision_box, velocity, false, STEP_HEIGHT)?;
    let identity = sampled.identity.merge(&motion.identity)?;
    next.requested_movement = velocity;
    next.position = motion.position;
    next.retain_collision_box(motion.aabb, height);
    next.on_ground = motion.collisions.y && velocity.y < 0.0;
    next.movement = motion.resolved;
    next.velocity = Vec3::new(0.0, motion.resolved.y.min(-GROUND_HINT_SPEED), 0.0);
    next.jump_delay = 0;
    next.collisions = motion.collisions;
    if !next.position.is_finite() || !next.velocity.is_finite() {
        return Err(SimulationError::NonFiniteState {
            field: "hover_motion",
        });
    }
    let result = TickResult {
        tick: next.tick,
        position: next.position,
        velocity: next.velocity,
        movement: next.movement,
        collisions: motion.collisions,
        on_ground: next.on_ground,
        environment: sampled.movement,
        world_identity: identity,
    };
    *state = next;
    Ok(ControlledTickResult {
        tick_result: result,
        controls,
        jump_initiated: false,
    })
}

#[cfg(test)]
mod tests {
    use crate::{
        Aabb, CollisionQuery, CollisionWorld, MovementAssist, MovementInput, PlayerState,
        Simulator, Vec3, WorldQueryError,
    };

    struct Floor;

    impl CollisionWorld for Floor {
        fn collision_boxes(
            &self,
            query: Aabb,
        ) -> Result<CollisionQuery<Vec<Aabb>>, WorldQueryError> {
            let floor = Aabb::new(Vec3::new(-64.0, -1.0, -64.0), Vec3::new(64.0, 0.0, 64.0));
            Ok(CollisionQuery::synthetic(if floor.intersects(query) {
                vec![floor]
            } else {
                Vec::new()
            }))
        }
    }

    struct Ledge {
        top: f64,
    }

    impl CollisionWorld for Ledge {
        fn collision_boxes(
            &self,
            query: Aabb,
        ) -> Result<CollisionQuery<Vec<Aabb>>, WorldQueryError> {
            let shapes = [
                Aabb::new(Vec3::new(-64.0, -1.0, -64.0), Vec3::new(64.0, 0.0, 64.0)),
                Aabb::new(Vec3::new(1.0, 0.0, -64.0), Vec3::new(64.0, self.top, 64.0)),
            ];
            Ok(CollisionQuery::synthetic(
                shapes.into_iter().filter(|shape| shape.intersects(query)).collect(),
            ))
        }
    }

    fn walking_into_ledge(assist: MovementAssist, sneaking: bool, top: f64, ticks: usize) -> f64 {
        let mut state = PlayerState::new(Vec3::new(0.65, 0.0, 0.0));
        state.on_ground = true;
        let simulator = Simulator::default();
        for _ in 0..ticks {
            state.velocity.x = 0.3;
            let input = MovementInput {
                sneaking,
                assist,
                ..MovementInput::default()
            };
            simulator.tick(&mut state, input, &Ledge { top }).unwrap();
        }
        state.position.y
    }

    #[test]
    fn step_height_lifts_the_player_onto_a_full_block_only_when_requested() {
        let step = MovementAssist {
            step_height: Some(1.0),
            ..MovementAssist::default()
        };
        assert!((walking_into_ledge(step, false, 1.0, 1) - 1.0).abs() < 1.0e-6);
        assert!(walking_into_ledge(MovementAssist::default(), false, 1.0, 1).abs() < 1.0e-6);
    }

    #[test]
    fn wall_climb_ascends_a_wall_and_sneaking_holds_position() {
        let spider = MovementAssist {
            wall_climb: true,
            ..MovementAssist::default()
        };
        let climbed = walking_into_ledge(spider, false, 10.0, 6);
        assert!(climbed > 0.8, "{climbed}");
        assert!(walking_into_ledge(MovementAssist::default(), false, 10.0, 6).abs() < 1.0e-6);
        let mut state = PlayerState::new(Vec3::new(0.65, 0.0, 0.0));
        let simulator = Simulator::default();
        let world = Ledge { top: 10.0 };
        for sneaking in [false, false, false, false, true, true, true] {
            state.velocity.x = 0.3;
            let input = MovementInput {
                sneaking,
                assist: spider,
                ..MovementInput::default()
            };
            simulator.tick(&mut state, input, &world).unwrap();
        }
        let held = state.position.y;
        for _ in 0..4 {
            state.velocity.x = 0.3;
            let input = MovementInput {
                sneaking: true,
                assist: spider,
                ..MovementInput::default()
            };
            simulator.tick(&mut state, input, &world).unwrap();
        }
        assert!(held > 0.2);
        assert!((state.position.y - held).abs() < 0.1, "{held} {}", state.position.y);
    }

    fn hover(velocity: [f64; 3]) -> MovementInput {
        MovementInput {
            assist: MovementAssist {
                fly_velocity: Some(velocity),
                ..MovementAssist::default()
            },
            ..MovementInput::default()
        }
    }

    #[test]
    fn hovering_ignores_gravity_and_moves_by_the_requested_velocity() {
        let mut state = PlayerState::new(Vec3::new(0.0, 10.0, 0.0));
        let simulator = Simulator::default();
        simulator.tick(&mut state, hover([0.0; 3]), &Floor).unwrap();
        assert!((state.position.y - 10.0).abs() < 1.0e-6);
        simulator.tick(&mut state, hover([0.5, 0.25, 0.0]), &Floor).unwrap();
        assert!((state.position.x - 0.5).abs() < 1.0e-6);
        assert!((state.position.y - 10.25).abs() < 1.0e-6);
        assert!(state.velocity.x.abs() < 1.0e-12);
    }

    #[test]
    fn hovering_descends_onto_terrain_without_passing_through() {
        let mut state = PlayerState::new(Vec3::new(0.0, 0.5, 0.0));
        let result = Simulator::default()
            .tick(&mut state, hover([0.0, -1.5, 0.0]), &Floor)
            .unwrap();
        assert!(state.position.y.abs() < 1.0e-6);
        assert!(result.on_ground);
    }

    #[test]
    fn no_fall_reports_a_vertical_collision_only_while_descending() {
        let assist = MovementAssist {
            no_fall: true,
            ..MovementAssist::default()
        };
        assert!(assist.reports_vertical_collision(false, -0.2));
        assert!(!assist.reports_vertical_collision(false, 0.1));
        assert!(!MovementAssist::default().reports_vertical_collision(false, -0.2));
        assert!(MovementAssist::default().reports_vertical_collision(true, 0.0));
    }
}
