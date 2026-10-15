use super::gameplay::GameplayContext;
use bevy::prelude::Vec3;
use client_extensions::modding::orbit_tpaura::{self as protocol, Action, Frame, Policy, Trip};
use client_ui::ui_runtime::UiRuntime;
use sim::CollisionWorld;

#[derive(Default)]
pub(super) struct Aura {
    trip: protocol::Aura,
    scope: Option<(u64, i32, u64)>,
    pending: Option<(u64, u64)>,
}

impl Aura {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

pub(super) fn drive(
    context: &mut GameplayContext<'_>,
    ui: &UiRuntime,
    aura: &mut Aura,
    policy: Option<Policy>,
    allowed: bool,
) -> Option<[f64; 3]> {
    let Some(frame) = observation(context, ui).filter(|_| allowed) else {
        aura.reset();
        return None;
    };
    if aura.scope != Some(frame.scope) {
        aura.scope = Some(frame.scope);
        aura.pending = None;
    }
    let admitted = context
        .melee
        .as_ref()
        .and_then(|melee| melee.admitted_attack());
    if aura
        .pending
        .zip(admitted)
        .is_some_and(|((id, before), attack)| {
            attack.sequence > before && attack.hit.runtime_id == id
        })
    {
        aura.trip.attacked(frame.now);
        aura.pending = None;
    }
    let action = aura
        .trip
        .tick(frame, policy, || select(context, ui, frame, policy?));
    if !matches!(action, Action::Aim(_)) {
        aura.pending = None;
    }
    match action {
        Action::Idle => None,
        Action::Move(destination) => {
            valid_landing(context, destination).then_some(destination.map(f64::from))
        }
        Action::Aim(id) => {
            if context.view.as_ref().is_none_or(|view| {
                !protocol::close(frame.feet, view.feet_translation().to_array(), 0.2)
            }) {
                return Some(frame.feet.map(f64::from));
            }
            let Some((yaw, pitch)) = aim(context, ui, id) else {
                aura.trip.abandon_target(frame.now);
                return None;
            };
            if yaw.abs() < 0.015 && pitch.abs() < 0.015 && aura.pending.is_none() {
                if super::module_observations::current_target(context, ui, None) == Some(Some(id))
                    && context.pulse_attack()
                {
                    aura.pending = Some((id, admitted.map_or(0, |attack| attack.sequence)));
                }
            } else {
                let seconds = context.real_time.as_ref()?.delta_secs().clamp(0.0, 0.1);
                let step = (seconds * 6.0).min(mod_api::MAX_CAMERA_DELTA_RADIANS);
                context.apply(mod_host::CameraDelta {
                    yaw: yaw.clamp(-step, step),
                    pitch: pitch.clamp(-step, step),
                });
            }
            Some(frame.feet.map(f64::from))
        }
    }
}

fn observation(context: &GameplayContext<'_>, ui: &UiRuntime) -> Option<Frame> {
    let snapshot = context.snapshot(
        true,
        &mod_host::ModGrants {
            camera: true,
            movement: true,
            ..Default::default()
        },
    )?;
    let world = context.world.as_ref()?;
    let movement = context.movement.as_ref()?;
    let player = context.player.as_ref()?;
    let state = context.physics.as_ref()?.state()?;
    let using = context.input.as_ref()?.phase(semantic_input::Action::Use);
    let jumping = context.input.as_ref()?.phase(semantic_input::Action::Jump);
    if world.dimension_transfer.active()
        || world.respawn.input_held()
        || !movement.physics_is_authorized()
        || player.facts.session_id() != ui.session_id()
        || player.facts.is_immobile()
        || player.facts.mount_unique_id().is_some()
        || !state.on_ground
        || jumping.held
        || jumping.pressed
        || using.held
        || using.pressed
        || ui.inventory_open()
        || ui.ui_focused(player)
        || ui
            .hud()
            .health()
            .is_some_and(|health| health.current() == 0)
    {
        return None;
    }
    let feet = [state.position.x, state.position.y, state.position.z].map(|v| v as f32);
    Some(Frame {
        scope: (
            snapshot.session,
            snapshot.dimension,
            movement.interaction_authority_identity().1.get(),
        ),
        now: context.real_time.as_ref()?.elapsed_secs_f64(),
        feet,
        held: snapshot.attack_held,
    })
}

fn select(
    context: &GameplayContext<'_>,
    ui: &UiRuntime,
    frame: Frame,
    policy: Policy,
) -> Option<Trip> {
    if !valid_landing(context, frame.feet) {
        return None;
    }
    let stream = context.world.as_ref()?.stream.as_ref()?;
    let authority = stream.authority();
    let mount = ui.gameplay_hud().mount_unique_id();
    let mut actors = authority
        .remote_actors()
        .filter(|actor| {
            let health = authority.actor_health_by_unique(actor.unique_id);
            let selected = match actor.kind {
                protocol_actor::ActorKind::Player { .. } => {
                    policy.players && health.is_none_or(|(health, _)| health > 0.0)
                }
                _ => policy.mobs && health.is_some_and(|(health, _)| health > 0.0),
            };
            selected
                && Some(actor.unique_id) != mount
                && gameplay::melee::actor_is_pickable(actor)
                && protocol::close(frame.feet, actor.position, policy.range + 2.0)
        })
        .collect::<Vec<_>>();
    actors.sort_by(|a, b| {
        Vec3::from_array(a.position)
            .distance_squared(Vec3::from_array(frame.feet))
            .total_cmp(&Vec3::from_array(b.position).distance_squared(Vec3::from_array(frame.feet)))
            .then(a.runtime_id.cmp(&b.runtime_id))
    });
    for actor in actors.into_iter().take(16) {
        let Some((min, max)) = authority.pick_hit_boxes(actor).into_iter().next() else {
            continue;
        };
        let center = [(min[0] + max[0]) * 0.5, min[1], (min[2] + max[2]) * 0.5];
        if !protocol::close(frame.feet, center, policy.range) {
            continue;
        }
        let angle = (frame.feet[2] - center[2]).atan2(frame.feet[0] - center[0]);
        let radius = ((max[0] - min[0]).max(max[2] - min[2]) * 0.5 + 0.9).max(1.4);
        for turn in [0.0, 0.5, -0.5, 1.0, -1.0, 1.5, -1.5, std::f32::consts::PI] {
            let landing = [
                center[0] + (angle + turn).cos() * radius,
                center[1],
                center[2] + (angle + turn).sin() * radius,
            ];
            if protocol::close(frame.feet, landing, policy.range) && valid_landing(context, landing)
            {
                return Some(Trip {
                    target: actor.runtime_id,
                    home: frame.feet,
                    landing,
                });
            }
        }
    }
    None
}

fn valid_landing(context: &GameplayContext<'_>, destination: [f32; 3]) -> bool {
    let Some(world) = context.world.as_ref() else {
        return false;
    };
    let Some(stream) = world.stream.as_ref() else {
        return false;
    };
    let Some(collisions) = context.collisions.as_ref() else {
        return false;
    };
    let Some(player) = context.player.as_ref() else {
        return false;
    };
    let Some(selection) = client_interaction::mining::hand_interaction_selection(player) else {
        return false;
    };
    let palette = sim::PaletteWorld::new(
        stream.collision_store(),
        collisions.registry(stream.network_id_mode()),
        stream.current_dimension(),
        crate::interaction_authority::pick_context(
            stream,
            collisions,
            player.facts.player_game_mode(),
            Some(selection.item.block_runtime_id()),
        ),
    );
    let feet = sim::Vec3::new(
        f64::from(destination[0]),
        f64::from(destination[1]),
        f64::from(destination[2]),
    );
    let body = sim::Aabb::player_with_height_at(feet, sim::PLAYER_HEIGHT);
    if !palette
        .collision_boxes(body)
        .is_ok_and(|query| !query.value.iter().any(|shape| shape.intersects(body)))
    {
        return false;
    }
    let floor = sim::Aabb::new(
        body.min - sim::Vec3::new(0.0, 0.05, 0.0),
        sim::Vec3::new(body.max.x, body.min.y, body.max.z),
    );
    palette
        .collision_boxes(floor)
        .is_ok_and(|query| query.value.iter().any(|shape| shape.intersects(floor)))
}

fn aim(context: &GameplayContext<'_>, ui: &UiRuntime, id: u64) -> Option<(f32, f32)> {
    let frame = context.snapshot(
        true,
        &mod_host::ModGrants {
            camera: true,
            ..Default::default()
        },
    )?;
    let world = context.world.as_ref()?;
    let stream = world.stream.as_ref()?;
    let authority = stream.authority();
    let actor = authority.actor(id)?;
    if !gameplay::melee::actor_is_pickable(actor)
        || authority
            .actor_health_by_unique(actor.unique_id)
            .is_some_and(|(health, _)| health <= 0.0)
        || Some(actor.unique_id) == ui.gameplay_hud().mount_unique_id()
    {
        return None;
    }
    let (min, max) = authority.pick_hit_boxes(actor).into_iter().next()?;
    let point = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let eye = [frame.eye.x, frame.eye.y, frame.eye.z];
    let delta = Vec3::from_array(point) - Vec3::from_array(eye);
    if !delta.is_finite() || delta.length() > 3.0 {
        return None;
    }
    let collisions = context.collisions.as_ref()?;
    let player = context.player.as_ref()?;
    let selection = client_interaction::mining::hand_interaction_selection(player)?;
    let palette = sim::PaletteWorld::new(
        stream.collision_store(),
        collisions.registry(stream.network_id_mode()),
        stream.current_dimension(),
        crate::interaction_authority::pick_context(
            stream,
            collisions,
            player.facts.player_game_mode(),
            Some(selection.item.block_runtime_id()),
        ),
    );
    let vector = |v: [f32; 3]| sim::Vec3::new(f64::from(v[0]), f64::from(v[1]), f64::from(v[2]));
    if palette
        .block_interaction_ray_current(
            vector(eye),
            vector(delta.to_array()),
            f64::from(delta.length()),
        )
        .ok()?
        .is_some_and(|hit| hit.distance + 0.001 < f64::from(delta.length()))
    {
        return None;
    }
    let yaw = ((-delta.x).atan2(-delta.z) - frame.yaw + std::f32::consts::PI)
        .rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI;
    let pitch = delta.y.atan2(delta.x.hypot(delta.z)) - frame.pitch;
    Some((yaw, pitch))
}

use ::protocol as protocol_actor;
