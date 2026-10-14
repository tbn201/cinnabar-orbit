#[cfg(not(feature = "acceptance"))]
use crate::acceptance::AcceptanceRun;
#[cfg(feature = "acceptance")]
use ::acceptance::AcceptanceRun;
use std::time::Instant;

use crate::runtime::network::resource_packs::PackApplication;
use bevy::{
    prelude::{EulerRot, Local, Res, ResMut, Time, Vec3},
    time::Real,
};
use client_runtime::network::NetworkHandle;
use protocol::PlayerInputMode;
use semantic_input::Action;
use {
    crate::runtime::world::ClientWorld,
    client_presentation::{camera::AutoFly, local_player::LocalViewPose},
    client_state::{PlayerRuntime, RuntimeSettings, SemanticInputSnapshot},
};

use client_interaction::movement::{
    LocalMovementEffectTimeline, LocalMovementSpeedAuthority, LocalPhysicsController,
    MovementTicker, PhysicsCollisionRegistries,
};

use gameplay::movement::LocomotionState as LocomotionLocals;

#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct MovementNetwork<'w> {
    network: Option<Res<'w, NetworkHandle<PackApplication>>>,
    #[cfg(feature = "local-mods")]
    assist: Option<Res<'w, crate::modding::assist::ModAssist>>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn advance_local_physics(
    time: Res<Time<Real>>,
    input: Res<SemanticInputSnapshot>,
    auto_fly: Res<AutoFly>,
    client_world: Res<ClientWorld>,
    collisions: Res<PhysicsCollisionRegistries>,
    acceptance: Res<AcceptanceRun>,
    mut physics: ResMut<LocalPhysicsController>,
    mut movement_effects: ResMut<LocalMovementEffectTimeline>,
    mut movement_speed: ResMut<LocalMovementSpeedAuthority>,
    mut movement_ticker: ResMut<MovementTicker>,
    mut view: ResMut<LocalViewPose>,
    settings: Option<Res<RuntimeSettings>>,
    player: Option<Res<PlayerRuntime>>,
    item_use: Option<Res<client_interaction::item_use::ItemUseRuntime>>,
    mut locals: Local<LocomotionLocals>,
    network: MovementNetwork,
) {
    if acceptance.deadline_reached(Instant::now()) {
        movement_ticker.begin_terminal_drain();
    }
    if auto_fly.enabled() || !physics.is_active() {
        locals.reset();
        return;
    }
    let Some(stream) = client_world.stream.as_ref() else {
        return;
    };
    let semantic = input.snapshot();
    let active = semantic.is_some();
    let input_mode = semantic.map_or(PlayerInputMode::Mouse, |snapshot| {
        match snapshot.input_mode {
            semantic_input::InputMode::KeyboardMouse => PlayerInputMode::Mouse,
            semantic_input::InputMode::GamePad => PlayerInputMode::GamePad,
            semantic_input::InputMode::Touch => PlayerInputMode::Touch,
        }
    });
    let movement = input.movement();
    let raw_movement = input.raw_movement();
    let analogue_movement = input.analogue_movement();
    let movement_buttons =
        semantic.map_or_else(Default::default, |snapshot| snapshot.movement_buttons);
    let (jump, sprint, sneak) = (
        input.phase(Action::Jump),
        input.phase(Action::Sprint),
        input.phase(Action::Sneak),
    );
    #[cfg(feature = "local-mods")]
    let motion = network.assist.as_deref().map_or_else(Default::default, |assist| {
        player.as_deref().map_or_else(Default::default, |player| {
            assist.motion_for(player.facts.session_id(), stream.current_dimension())
        })
    });
    #[cfg(feature = "local-mods")]
    let assist = motion.assist(
        view.rotation(),
        crate::modding::motion::Steering::read(&input),
    );
    #[cfg(feature = "local-mods")]
    let (movement, raw_movement, analogue_movement, movement_buttons) = if motion.freecam.is_some()
    {
        ([0.0; 2], [0.0; 2], [0.0; 2], Default::default())
    } else {
        (movement, raw_movement, analogue_movement, movement_buttons)
    };
    #[cfg(feature = "local-mods")]
    let (jump, sprint, sneak) = if motion.freecam.is_some() || assist.fly_velocity.is_some() {
        Default::default()
    } else {
        (jump, sprint, sneak)
    };
    let (bevy_yaw, bevy_pitch, _) = view.rotation().to_euler(EulerRot::YXZ);
    let yaw = gameplay::movement::wire_yaw(180.0 - bevy_yaw.to_degrees());
    let facts = gameplay::movement::local_facts::read(
        player.as_deref().map(|player| &**player),
        &client_interaction::movement::world_view::GameplayWorldView(stream),
        item_use
            .as_deref()
            .is_some_and(|item_use| item_use.is_using()),
    );
    let gameplay = settings
        .as_deref()
        .map(|settings| settings.user_settings_update().1.gameplay)
        .unwrap_or_default();
    let toggle_sprint = gameplay.toggle_sprint;
    #[cfg(feature = "local-mods")]
    let toggle_sprint = toggle_sprint
        || network.assist.as_deref().is_some_and(|assist| {
            player.as_deref().is_some_and(|player| {
                assist.sprint_for(player.facts.session_id(), stream.current_dimension())
            })
        });
    let always_sprint = gameplay.always_sprint && input_mode == PlayerInputMode::Mouse;
    #[cfg(feature = "local-mods")]
    let always_sprint = always_sprint
        || network.assist.as_deref().is_some_and(|assist| {
            player.as_deref().is_some_and(|player| {
                assist.auto_sprint_for(player.facts.session_id(), stream.current_dimension())
            })
        });
    let network = network.network;
    let frame = gameplay::movement::PhysicsFrameInput {
        delta: time.delta(),
        now: time.elapsed(),
        active,
        input_authority_released: semantic.is_none_or(|snapshot| snapshot.authority_released),
        movement,
        raw_movement,
        analogue_movement,
        movement_buttons,
        yaw,
        pitch: -bevy_pitch.to_degrees(),
        camera_orientation: (view.rotation() * Vec3::NEG_Z).to_array(),
        input_mode,
        jump,
        sprint,
        sneak,
        toggle_sprint,
        always_sprint,
        toggle_sneak: gameplay.toggle_sneak,
        facts,
        item_use_modifier: item_use
            .as_deref()
            .and_then(|item_use| item_use.movement_modifier()),
        hold: (client_world.dimension_transfer.active() || client_world.respawn.input_held()).then(
            || gameplay::movement::PhysicsFrameHold {
                registry: collisions.registry(stream.network_id_mode()).identity(),
                withhold_input: client_world.respawn.input_held(),
            },
        ),
    };
    locals.observe_dismount(&frame, movement_ticker.session_generation());
    if !movement_ticker.can_advance_physics_frame() {
        // Retain presses and publish the pose while transport pauses simulation.
        publish_physics_view(&physics, &mut view);
        return;
    }
    let world = sim::PaletteWorld::for_movement(
        stream.collision_store(),
        collisions.registry(stream.network_id_mode()),
        stream.current_dimension(),
    );
    #[cfg(feature = "local-mods")]
    physics.set_movement_assist(assist);
    let advanced = locals.advance(
        frame,
        &mut physics,
        &mut movement_ticker,
        &mut movement_effects,
        &mut movement_speed,
        &world,
    );
    if !advanced {
        return;
    }
    if let (Some(position), Some(network)) = (physics.network_position(), network) {
        locals.flush_dismount(position, |packet| {
            network.send_inventory_packets(vec![packet])
        });
    }
    publish_physics_view(&physics, &mut view);
}

/// Publishes the retained interpolated pose even while transport pauses simulation.
fn publish_physics_view(physics: &LocalPhysicsController, view: &mut LocalViewPose) {
    if let (Some(eye), Some(feet)) = (
        physics.render_eye_position(),
        physics.render_feet_position(),
    ) {
        view.set_subject_position(Vec3::from_array(eye), Vec3::from_array(feet));
    }
}
