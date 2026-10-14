use super::gameplay::GameplayContext;
use bevy::prelude::{EulerRot, Vec3};
use client_extensions::modding::orbit_autominer::{Delay, Policy, scan_window, tool_safe};
use client_ui::ui_runtime::UiRuntime;
use sim::PaletteWorld;

#[derive(Default)]
pub(super) struct Miner {
    scope: Option<(u64, i32)>,
    policy: Option<Policy>,
    target: Option<[i32; 3]>,
    scan_at: f64,
    scan_cursor: usize,
    delay: Delay,
    observed: bool,
    last_frame: f64,
}

pub(super) fn prepare(
    context: &mut GameplayContext<'_>,
    ui: &UiRuntime,
    miner: &mut Miner,
    policy: Option<Policy>,
) -> Option<([i32; 3], u32)> {
    let Some(policy) = policy else {
        *miner = Miner::default();
        return None;
    };
    miner.observed = false;
    let result = plan(context, ui, miner, policy);
    if !miner.observed {
        miner.delay.reset();
    }
    result
}

fn plan(
    context: &mut GameplayContext<'_>,
    ui: &UiRuntime,
    miner: &mut Miner,
    policy: Policy,
) -> Option<([i32; 3], u32)> {
    let input = context.input.as_deref()?;
    let using = input.phase(semantic_input::Action::Use);
    if using.held || using.pressed {
        return None;
    }
    let snapshot = input.snapshot()?;
    if super::module_observations::current_target(context, ui, None)?.is_some() {
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
    let movement = context.movement.as_deref()?;
    if !movement.accepts_block_interactions() {
        return None;
    }
    let caps = player.facts.game_mode_capabilities()?;
    if !gameplay::survival_mining::mining_active(Some(caps), true, true) {
        return None;
    }
    let scope = (ui.session_id(), stream.current_dimension());
    let time = context.real_time.as_deref()?;
    let now = time.elapsed_secs_f64();
    if miner.scope != Some(scope)
        || miner.policy != Some(policy)
        || now < miner.last_frame
        || now - miner.last_frame > 0.5
    {
        *miner = Miner {
            scope: Some(scope),
            policy: Some(policy),
            ..Miner::default()
        };
    }
    miner.last_frame = now;
    let ledger = ui.inventory_ledger(player);
    let selected = player.selected_stack_snapshot()?;
    if policy.protect_tools {
        if let inventory::PlayerInventorySlot::Present(stack) = selected.state {
            let maximum = stream
                .authority()
                .item_durability(stack.network_id)
                .maximum();
            if maximum.is_none()
                && ledger
                    .negotiated_item_entry(stack.network_id)
                    .is_none_or(|entry| sim::HeldTool::from_identifier(&entry.identifier).is_some())
            {
                return None;
            }
            let damage = ledger
                .predicted_slot_damage(selected.slot)
                .or_else(|| {
                    protocol::item_extra_damage(&stack.extra_data)
                        .and_then(|value| i32::try_from(value).ok())
                })
                .unwrap_or(0)
                .max(0) as u32;
            if !protocol::item_extra_unbreakable(&stack.extra_data)
                && !tool_safe(maximum, Some(damage), policy.durability)
            {
                return None;
            }
        }
    }
    if ledger.slot_gesture_pending(selected.slot)
        || player.inventory.pending_hotbar_selection().is_some()
    {
        return None;
    }
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
    let palette = PaletteWorld::new(
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
    let mode = gameplay::mining::protocol_input_mode(snapshot.input_mode);
    let reach = f64::from(policy.range).min(if caps.creative_reach {
        gameplay::mining::creative_reach(mode)
    } else {
        gameplay::mining::survival_reach(mode)
    });
    let eye = ray.origin();
    let feet = context.view.as_deref()?.feet_translation();
    if !eye.is_finite() || !feet.is_finite() {
        return None;
    }
    let vector = |v: Vec3| sim::Vec3::new(f64::from(v.x), f64::from(v.y), f64::from(v.z));
    let mut ids = policy
        .names()
        .flat_map(|name| stream.authority().block_ids_for_name(name).to_vec())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    let eligible = |position: [i32; 3]| -> Option<u32> {
        if position[1] as f32 + 1.0 <= feet.y + 0.05 {
            return None;
        }
        let id = palette.primary_runtime_id(position).ok()?;
        ids.binary_search(&id).ok()?;
        let facts = sim::BlockDestroyInfo::from_facts(
            stream
                .runtime_assets()
                .resolve(stream.network_id_mode(), id)
                .destroy(),
        )?;
        (facts.hardness >= 0.0).then_some(id)
    };
    let visible = |position: [i32; 3]| -> bool {
        let delta = Vec3::from_array(position.map(|v| v as f32)) + Vec3::splat(0.5) - eye;
        delta.length_squared() <= (reach as f32).powi(2)
            && palette
                .block_interaction_ray_current(vector(eye), vector(delta), reach)
                .ok()
                .flatten()
                .is_some_and(|hit| hit.block_pos == position)
    };
    let aimed = palette
        .block_interaction_ray_current(vector(eye), vector(ray.direction()), reach)
        .ok()
        .flatten();
    let target = if policy.nearby {
        if !miner
            .target
            .is_some_and(|position| eligible(position).is_some() && visible(position))
        {
            miner.target = None;
            if now >= miner.scan_at {
                miner.scan_at = now + 0.2;
                let base = eye.floor().as_ivec3().to_array();
                let mut candidates = Vec::new();
                for x in -4..=4 {
                    for y in -4..=4 {
                        for z in -4..=4 {
                            let Some(position) = base[0]
                                .checked_add(x)
                                .zip(base[1].checked_add(y))
                                .zip(base[2].checked_add(z))
                                .map(|((x, y), z)| [x, y, z])
                            else {
                                continue;
                            };
                            if eligible(position).is_none() {
                                continue;
                            }
                            let delta = Vec3::from_array(position.map(|v| v as f32))
                                + Vec3::splat(0.5)
                                - eye;
                            if delta.length_squared() <= (reach as f32).powi(2) {
                                candidates.push((delta.length_squared(), position));
                            }
                        }
                    }
                }
                candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
                miner.target = scan_window(candidates.len(), miner.scan_cursor)
                    .map(|index| candidates[index].1)
                    .find(|position| visible(*position));
                miner.scan_cursor = (miner.scan_cursor + 16) % candidates.len().max(1);
            }
        }
        miner.target?
    } else {
        aimed.as_ref()?.block_pos
    };
    let id = eligible(target)?;
    if policy.nearby {
        let delta = Vec3::from_array(target.map(|v| v as f32)) + Vec3::splat(0.5) - eye;
        let (yaw, pitch, _) = context.view.as_deref()?.rotation().to_euler(EulerRot::YXZ);
        let wanted_yaw = (-delta.x).atan2(-delta.z);
        let wanted_pitch = delta.y.atan2(delta.x.hypot(delta.z));
        let yaw_delta = (wanted_yaw - yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        let step = (time.delta_secs().clamp(0.0, 0.1) * 2.5).min(mod_api::MAX_CAMERA_DELTA_RADIANS);
        context.apply(mod_host::CameraDelta {
            yaw: yaw_delta.clamp(-step, step),
            pitch: (wanted_pitch - pitch).clamp(-step, step),
        });
    }
    let current = aimed
        .filter(|hit| hit.block_pos == target && hit.runtime_id == id)
        .map(|_| (target, id));
    miner.observed = true;
    miner
        .delay
        .ready(now, scope, current, policy.delay_ms)
        .then_some((target, id))
}
