use super::{
    ControlledTickResult, MovementInput, PlayerState, ProcessedControls, SimulationError,
    TickResult, environment::sample,
};
use crate::{Aabb, CollisionWorld, Vec3};

pub(super) fn tick(
    mut next: PlayerState,
    state: &mut PlayerState,
    input: MovementInput,
    controls: ProcessedControls,
    destination: [f64; 3],
    world: &impl CollisionWorld,
) -> Result<Option<ControlledTickResult>, SimulationError> {
    let destination = Vec3::new(destination[0], destination[1], destination[2]);
    let delta = destination - state.position;
    if delta.length_squared() > 64.01
        || input.mode != super::MovementMode::Walking
        || input.spectator
    {
        return Ok(None);
    }
    let height = input.mode.hitbox_height(input.sneaking);
    let body = Aabb::player_with_height_at(destination, height);
    let bodies = world.collision_boxes(body)?;
    if bodies.value.iter().any(|shape| shape.intersects(body)) {
        return Ok(None);
    }
    let floor = Aabb::new(
        body.min - Vec3::new(0.0, 0.05, 0.0),
        Vec3::new(body.max.x, body.min.y, body.max.z),
    );
    let floors = world.collision_boxes(floor)?;
    let Some(ground) = floors
        .value
        .iter()
        .filter(|shape| shape.intersects(floor))
        .map(|shape| shape.max.y)
        .max_by(f64::total_cmp)
    else {
        return Ok(None);
    };
    let destination = Vec3::new(destination.x, ground, destination.z);
    let delta = destination - state.position;
    if delta.length_squared() > 64.01 {
        return Ok(None);
    }
    let body = Aabb::player_with_height_at(destination, height);
    let settled = world.collision_boxes(body)?;
    if settled.value.iter().any(|shape| shape.intersects(body)) {
        return Ok(None);
    }
    let sampled = sample(
        world,
        destination,
        Vec3::ZERO,
        height,
        input.liquid_contact_height,
    )?;
    let identity = sampled
        .identity
        .merge(&bodies.identity)?
        .merge(&floors.identity)?
        .merge(&settled.identity)?;
    if sampled.movement.in_water || sampled.movement.in_lava {
        return Ok(None);
    }
    next.position = destination;
    next.retain_collision_box(body, height);
    next.velocity = Vec3::ZERO;
    next.movement = delta;
    next.requested_movement = delta;
    next.on_ground = true;
    next.collisions = Default::default();
    let result = TickResult {
        tick: next.tick,
        position: next.position,
        velocity: next.velocity,
        movement: delta,
        collisions: next.collisions,
        on_ground: true,
        environment: sampled.movement,
        world_identity: identity,
    };
    *state = next;
    Ok(Some(ControlledTickResult {
        tick_result: result,
        controls,
        jump_initiated: false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CollisionQuery, Simulator, WorldQueryError};

    struct Terrain {
        floor: bool,
        blocked: bool,
    }
    impl CollisionWorld for Terrain {
        fn collision_boxes(
            &self,
            query: Aabb,
        ) -> Result<CollisionQuery<Vec<Aabb>>, WorldQueryError> {
            let mut shapes = Vec::new();
            if self.floor {
                shapes.push(Aabb::new(
                    Vec3::new(-16.0, -1.0, -16.0),
                    Vec3::new(16.0, 0.0, 16.0),
                ));
            }
            if self.blocked {
                shapes.push(Aabb::new(
                    Vec3::new(3.0, 0.0, -1.0),
                    Vec3::new(5.0, 3.0, 1.0),
                ));
            }
            Ok(CollisionQuery::synthetic(
                shapes
                    .into_iter()
                    .filter(|shape| shape.intersects(query))
                    .collect(),
            ))
        }
    }

    fn blink(destination: [f64; 3], terrain: Terrain) -> (PlayerState, TickResult) {
        let mut state = PlayerState::new(Vec3::ZERO);
        state.on_ground = true;
        let input = MovementInput {
            assist: super::super::MovementAssist {
                blink_to: Some(destination),
                ..Default::default()
            },
            ..Default::default()
        };
        let output = Simulator::default()
            .tick(&mut state, input, &terrain)
            .unwrap();
        (state, output)
    }

    #[test]
    fn blink_is_one_completed_tick_with_real_displacement_and_can_return() {
        let terrain = Terrain {
            floor: true,
            blocked: false,
        };
        let (mut state, output) = blink([4.0, 0.0, 0.0], terrain);
        assert_eq!(state.position, Vec3::new(4.0, 0.0, 0.0));
        assert_eq!(output.tick, 1);
        assert_eq!(output.movement, Vec3::new(4.0, 0.0, 0.0));
        assert!(output.on_ground);
        let input = MovementInput {
            assist: super::super::MovementAssist {
                blink_to: Some([0.0; 3]),
                ..Default::default()
            },
            ..Default::default()
        };
        let result = Simulator::default()
            .tick(
                &mut state,
                input,
                &Terrain {
                    floor: true,
                    blocked: false,
                },
            )
            .unwrap();
        assert_eq!(result.tick, 2);
        assert_eq!(state.position, Vec3::ZERO);
    }

    #[test]
    fn small_actor_height_rounding_errors_settle_onto_the_real_floor() {
        let (state, result) = blink(
            [4.0, 0.025, 0.0],
            Terrain {
                floor: true,
                blocked: false,
            },
        );
        assert_eq!(state.position, Vec3::new(4.0, 0.0, 0.0));
        assert!(result.on_ground);
    }

    #[test]
    fn occupied_unsupported_and_distant_landings_do_not_teleport() {
        for (destination, terrain) in [
            (
                [4.0, 0.0, 0.0],
                Terrain {
                    floor: true,
                    blocked: true,
                },
            ),
            (
                [4.0, 0.0, 0.0],
                Terrain {
                    floor: false,
                    blocked: false,
                },
            ),
            (
                [9.0, 0.0, 0.0],
                Terrain {
                    floor: true,
                    blocked: false,
                },
            ),
        ] {
            let (state, _) = blink(destination, terrain);
            assert!(state.position.x.abs() < 0.1);
        }
    }
}
