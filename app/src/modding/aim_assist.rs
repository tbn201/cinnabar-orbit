use super::gameplay::GameplayContext;
use bevy::prelude::Vec3;
use client_extensions::modding::orbit_aim_assist::{Policy, Target, choose, turn};
use client_ui::ui_runtime::UiRuntime;

#[derive(Default)]
pub(super) struct Aim {
    scope: Option<(u64, i32)>,
    policy: Option<Policy>,
    locked: Option<u64>,
    last: f64,
}

pub(super) fn drive(
    context: &GameplayContext<'_>,
    ui: &UiRuntime,
    state: &mut Aim,
    policy: Option<Policy>,
) -> Option<mod_host::CameraDelta> {
    let Some(policy) = policy else {
        *state = Aim::default();
        return None;
    };
    let result = select(context, ui, state, policy);
    if result.is_none() {
        state.locked = None;
    }
    result
}

fn select(
    context: &GameplayContext<'_>,
    ui: &UiRuntime,
    state: &mut Aim,
    policy: Policy,
) -> Option<mod_host::CameraDelta> {
    let frame = context.snapshot(
        true,
        &mod_host::ModGrants {
            camera: true,
            ..Default::default()
        },
    )?;
    if policy.held_only && !frame.attack_held {
        return None;
    }
    let using = context.input.as_deref()?.phase(semantic_input::Action::Use);
    if using.held || using.pressed {
        return None;
    }
    let world = context.world.as_deref()?;
    if world.dimension_transfer.active() || world.respawn.input_held() {
        return None;
    }
    let stream = world.stream.as_ref()?;
    let player = context.player.as_deref()?;
    if player.facts.session_id() != ui.session_id() {
        return None;
    }
    let scope = (ui.session_id(), stream.current_dimension());
    let time = context.real_time.as_deref()?;
    let now = time.elapsed_secs_f64();
    if state.scope != Some(scope)
        || state.policy != Some(policy)
        || now < state.last
        || now - state.last > 0.5
    {
        *state = Aim {
            scope: Some(scope),
            policy: Some(policy),
            ..Default::default()
        };
    }
    state.last = now;
    let origin = context.origin.as_deref()?;
    let ray = origin.outbound_ray()?;
    if !crate::interaction_authority::ray_is_current(
        ray,
        simulation_clock::identity::SessionGeneration::new(scope.0),
        stream,
    ) {
        return None;
    }
    let collisions = context.collisions.as_deref()?;
    let selection = client_interaction::mining::hand_interaction_selection(player)?;
    let palette = sim::PaletteWorld::new(
        stream.collision_store(),
        collisions.registry(stream.network_id_mode()),
        scope.1,
        crate::interaction_authority::pick_context(
            stream,
            collisions,
            player.facts.player_game_mode(),
            Some(selection.item.block_runtime_id()),
        ),
    );
    if frame.attack_held && super::module_observations::current_target(context, ui, None)? == None {
        let origin = ray.origin();
        let direction = ray.direction();
        let mode =
            gameplay::mining::protocol_input_mode(context.input.as_deref()?.snapshot()?.input_mode);
        let range = gameplay::mining::survival_reach(mode);
        let vector = |v: Vec3| sim::Vec3::new(f64::from(v.x), f64::from(v.y), f64::from(v.z));
        if palette
            .block_interaction_ray_current(vector(origin), vector(direction), range)
            .ok()
            .flatten()
            .is_some()
        {
            return None;
        }
    }
    let eye = [frame.eye.x, frame.eye.y, frame.eye.z];
    let authority = stream.authority();
    let mount = ui.gameplay_hud().mount_unique_id();
    let mut actors = authority
        .remote_actors()
        .filter(|actor| {
            let is_player = matches!(actor.kind, protocol::ActorKind::Player { .. });
            let health = authority.actor_health_by_unique(actor.unique_id);
            (if is_player {
                policy.players && health.is_none_or(|(health, _)| health > 0.0)
            } else {
                policy.mobs && health.is_some_and(|(health, _)| health > 0.0)
            }) && Some(actor.unique_id) != mount
                && gameplay::melee::actor_is_pickable(actor)
                && actor.position.iter().all(|value| value.is_finite())
                && (0..3)
                    .map(|i| (actor.position[i] - eye[i]).powi(2))
                    .sum::<f32>()
                    <= (policy.range + 2.0).powi(2)
        })
        .collect::<Vec<_>>();
    actors.sort_by(|a, b| {
        let distance =
            |position: [f32; 3]| (0..3).map(|i| (position[i] - eye[i]).powi(2)).sum::<f32>();
        distance(a.position)
            .total_cmp(&distance(b.position))
            .then(a.runtime_id.cmp(&b.runtime_id))
    });
    let targets = actors
        .into_iter()
        .take(mod_api::MAX_OBSERVED_ACTORS)
        .flat_map(|actor| {
            authority
                .pick_hit_boxes(actor)
                .into_iter()
                .take(mod_api::MAX_OBSERVED_ACTOR_BOXES)
                .map(move |(min, max)| Target {
                    id: actor.runtime_id,
                    min,
                    max,
                })
        })
        .collect::<Vec<_>>();
    let vector = |v: [f32; 3]| sim::Vec3::new(f64::from(v[0]), f64::from(v[1]), f64::from(v[2]));
    let chosen = choose(
        policy,
        eye,
        [frame.yaw, frame.pitch],
        state.locked,
        &targets,
        |choice| {
            let delta = Vec3::from_array(choice.point) - Vec3::from_array(eye);
            palette
                .block_interaction_ray_current(
                    vector(eye),
                    vector(delta.to_array()),
                    f64::from(choice.distance),
                )
                .is_ok_and(|hit| {
                    hit.is_none_or(|hit| hit.distance + 0.001 >= f64::from(choice.distance))
                })
        },
    )?;
    state.locked = Some(chosen.id);
    let [yaw, pitch] = turn(
        chosen,
        time.delta_secs(),
        policy.strength,
        mod_api::MAX_CAMERA_DELTA_RADIANS,
    );
    Some(mod_host::CameraDelta { yaw, pitch })
}
