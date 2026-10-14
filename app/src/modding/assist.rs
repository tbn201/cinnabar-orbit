use crate::runtime::{network::resource_packs::PackApplication, world::ClientWorld};
use bevy::{input::mouse::AccumulatedMouseScroll, prelude::*, window::PrimaryWindow};
use client_interaction::{
    melee::MeleeRuntime,
    mining::hand_interaction_selection,
    movement::{MovementTicker, PhysicsCollisionRegistries},
};
use client_presentation::local_player::InteractionOriginSnapshot;
use client_shell::ui_resources::UiRuntimeResource;
use client_state::{PlayerRuntime, ScreenFocus, SemanticInputSnapshot};
use inventory::PlayerInventorySlot;
use protocol::NetworkItemStack;
use semantic_input::{Action, HOTBAR_DIGIT_ACTIONS};
use sim::{BlockDestroyInfo, DestroyConditions, HeldTool};

mod debounce;
mod selection;
use selection::{Frame, Item, ItemKey, Selection, Slot, ToolScore};

#[derive(Resource, Default)]
pub(crate) struct ModAssist {
    pub hotbar: Option<(bool, bool)>,
    pub sprint: bool,
    pub auto_sprint: bool,
    pub debounce_ms: Option<f32>,
    pub scope: Option<(u64, i32)>,
}

impl ModAssist {
    pub fn auto_sprint_for(&self, session: u64, dimension: i32) -> bool {
        self.auto_sprint && self.scope == Some((session, dimension))
    }

    pub fn sprint_for(&self, session: u64, dimension: i32) -> bool {
        self.sprint && self.scope == Some((session, dimension))
    }
}

pub(super) fn configure(app: &mut App) {
    app.add_systems(
        Update,
        filter_double_clicks
            .after(client_state::ShellFrameSet::FinalFocus)
            .before(crate::semantic_controls::finalize_semantic_input_after_ui_authority)
            .in_set(client_state::ClientFrameSet::SemanticFinalize),
    );
    app.init_resource::<ModAssist>().add_systems(
        Update,
        (select_hotbar, flush_selection)
            .chain()
            .after(crate::melee::produce_melee)
            .before(crate::survival_mining::produce_survival_mining)
            .in_set(client_state::ClientFrameSet::NetworkSend),
    );
}

fn flush_selection(
    player: ResMut<PlayerRuntime>,
    network: Res<client_runtime::network::NetworkHandle<PackApplication>>,
    world: ResMut<ClientWorld>,
) {
    crate::hotbar::flush_hotbar_selection(player, network, world);
}

#[allow(clippy::too_many_arguments)]
fn select_hotbar(
    mut player: ResMut<PlayerRuntime>,
    ui: Res<UiRuntimeResource>,
    world: Res<ClientWorld>,
    input: Res<SemanticInputSnapshot>,
    origin: Res<InteractionOriginSnapshot>,
    collisions: Res<PhysicsCollisionRegistries>,
    movement: Res<MovementTicker>,
    focus: Res<ScreenFocus>,
    windows: Query<&Window, With<PrimaryWindow>>,
    melee: Res<MeleeRuntime>,
    scroll: Res<AccumulatedMouseScroll>,
    policy: Res<ModAssist>,
    mut state: Local<Selection>,
    interaction: Option<Res<super::interaction::ModInteraction>>,
) {
    let Some(stream) = world.stream.as_ref() else {
        state.reset();
        return;
    };
    let scope = (ui.session_id(), stream.current_dimension());
    let Some(semantic) = input.snapshot().filter(|_| {
        policy.scope == Some(scope)
            && player.facts.session_id() == scope.0
            && !world.dimension_transfer.active()
            && !world.respawn.input_held()
            && !focus.absorbs_input
            && !focus.ui_focused
            && !focus.menu_visible
            && windows.single().is_ok_and(|window| window.focused)
            && player.inventory.pending_hotbar_selection().is_none()
    }) else {
        state.reset();
        return;
    };
    let Some((auto_stack, auto_tool)) = policy.hotbar else {
        state.reset();
        return;
    };
    let Some(selected) = player.selected_hotbar_slot() else {
        state.reset();
        return;
    };
    let caps = player.facts.game_mode_capabilities();
    if !gameplay::survival_mining::mining_active(caps, true, true) {
        state.reset();
        return;
    }
    let mut attack = input.phase(Action::Attack);
    attack.held |= interaction.as_deref().is_some_and(|policy| policy.mining.is_some());
    let using = input.phase(Action::Use);
    let manual = HOTBAR_DIGIT_ACTIONS
        .iter()
        .any(|action| input.phase(*action).pressed)
        || input.phase(Action::HotbarNext).pressed
        || input.phase(Action::HotbarPrevious).pressed
        || scroll.delta.y != 0.0;
    let observed = (auto_tool
        && (attack.held || attack.pressed)
        && !using.held
        && !using.pressed
        && !melee.actor_in_front()
        && !caps.is_some_and(|caps| caps.instant_break))
    .then(|| {
        crate::interaction_authority::observe_block(
            &origin,
            &ui,
            &world,
            &collisions,
            hand_interaction_selection(&player)?,
            player.facts.player_game_mode(),
            (
                gameplay::mining::protocol_input_mode(semantic.input_mode),
                gameplay::mining::survival_reach(gameplay::mining::protocol_input_mode(
                    semantic.input_mode,
                )),
                (semantic.authority_generation, semantic.frame_sequence),
                movement.interaction_authority_identity().1,
            ),
        )
        .filter(gameplay::interaction_authority::within_pick_range)
    })
    .flatten();
    let block = observed.as_ref().and_then(|observed| {
        sim::BlockDestroyInfo::from_facts(
            stream
                .runtime_assets()
                .resolve(stream.network_id_mode(), observed.target.runtime_id)
                .destroy(),
        )
        .filter(|block| block.hardness > 0.0)
    });
    let ledger = ui.inventory_ledger(&player);
    let slots = std::array::from_fn(|index| {
        let slot = index as u8;
        if ledger.slot_gesture_pending(slot) {
            return Slot::default();
        }
        let observed = player
            .selected_stack_snapshot()
            .filter(|selected| selected.slot == slot);
        let state = observed
            .map(|selected| selected.state)
            .or_else(|| ledger.slot_state(slot));
        match state {
            Some(PlayerInventorySlot::Empty) => Slot {
                known: true,
                item: None,
            },
            Some(PlayerInventorySlot::Present(stack)) if !stack.is_empty() => Slot {
                known: true,
                item: Some(Item {
                    key: ItemKey {
                        network_id: stack.network_id,
                        metadata: stack.metadata,
                        block_runtime_id: stack.block_runtime_id,
                        nbt_digest: stack.nbt_digest,
                    },
                    count: stack.count,
                    tool: block.and_then(|block| {
                        tool_score(
                            block,
                            stack,
                            ledger,
                            slot,
                            stream
                                .authority()
                                .item_durability(stack.network_id)
                                .maximum(),
                        )
                    }),
                }),
            },
            _ => Slot::default(),
        }
    });
    let choice = state.select(Frame {
        scope,
        selected,
        slots: &slots,
        target: observed
            .as_ref()
            .filter(|_| block.is_some())
            .map(|observed| observed.target.position),
        hand: block.and_then(|block| {
            let progress = sim::destroy_progress_per_tick(
                &block,
                &DestroyConditions {
                    on_ground: true,
                    ..Default::default()
                },
            )?;
            Some(ToolScore {
                harvest: block.harvestable_with(None),
                progress,
                usable: false,
                silk_touch: 0,
                fortune: 0,
            })
        }),
        attack: attack.held || attack.pressed,
        using: using.held || using.pressed,
        manual,
        auto_stack,
        auto_tool,
    });
    if let Some(slot) = choice {
        let mode = player.facts.player_game_mode();
        player.inventory.queue_local_hotbar_selection(slot, mode);
    }
}

fn tool_score(
    block: BlockDestroyInfo,
    stack: &NetworkItemStack,
    ledger: &inventory::PlayerInventoryLedger,
    slot: u8,
    maximum: Option<u32>,
) -> Option<ToolScore> {
    let entry = ledger.negotiated_item_entry(stack.network_id)?;
    if entry.component_based {
        return None;
    }
    let tool = HeldTool::from_identifier(&entry.identifier);
    let damage = ledger
        .predicted_slot_damage(slot)
        .or_else(|| {
            protocol::item_extra_damage(&stack.extra_data)
                .and_then(|value| i32::try_from(value).ok())
        })
        .unwrap_or(0)
        .max(0) as u32;
    let usable = tool.is_some_and(|tool| {
        maximum.is_some_and(|maximum| {
            maximum.saturating_sub(damage)
                > gameplay::survival_mining::tool_break_damage(tool.kind) as u32
        })
    });
    let progress = sim::destroy_progress_per_tick(
        &block,
        &DestroyConditions {
            tool,
            efficiency_level: protocol::item_enchantment_level(&stack.extra_data, 15).unwrap_or(0),
            on_ground: true,
            ..Default::default()
        },
    )?;
    Some(ToolScore {
        harvest: block.harvestable_with(tool),
        progress,
        usable,
        silk_touch: protocol::item_enchantment_level(&stack.extra_data, 16).unwrap_or(0),
        fortune: protocol::item_enchantment_level(&stack.extra_data, 18).unwrap_or(0),
    })
}

fn filter_double_clicks(
    policy: Res<ModAssist>,
    ui: Res<UiRuntimeResource>,
    world: Res<ClientWorld>,
    focus: Res<ScreenFocus>,
    windows: Query<&Window, With<PrimaryWindow>>,
    buttons: Res<ButtonInput<bevy::input::mouse::MouseButton>>,
    time: Res<Time>,
    mut runtime: ResMut<crate::semantic_controls::SemanticInputRuntime>,
    mut state: Local<(Option<(u64, i32)>, debounce::Debounce)>,
) {
    use bevy::input::mouse::MouseButton;
    let scope = world
        .stream
        .as_ref()
        .map(|stream| (ui.session_id(), stream.current_dimension()));
    let active = policy.debounce_ms.filter(|_| {
        scope.is_some()
            && policy.scope == scope
            && !world.dimension_transfer.active()
            && !world.respawn.input_held()
            && !focus.absorbs_input
            && !focus.ui_focused
            && !focus.menu_visible
            && windows.single().is_ok_and(|window| window.focused)
    });
    if active.is_none() || state.0 != scope {
        state.1 = debounce::Debounce::default();
    }
    state.0 = scope;
    let Some(delay) = active else { return };
    for (index, button, code) in [(0, MouseButton::Left, 1), (1, MouseButton::Right, 2)] {
        if buttons.just_pressed(button) && state.1.reject(index, time.elapsed_secs_f64(), delay) {
            runtime.consume_control(semantic_input::PhysicalControl::MouseButton(code));
        }
    }
}

#[cfg(test)]
mod sprint_scope_tests {
    use super::*;

    #[test]
    fn auto_sprint_cannot_cross_player_sessions_or_dimensions() {
        let mut policy = ModAssist {
            auto_sprint: true,
            scope: Some((7, 0)),
            ..ModAssist::default()
        };
        assert!(policy.auto_sprint_for(7, 0));
        assert!(!policy.auto_sprint_for(8, 0));
        assert!(!policy.auto_sprint_for(7, 1));
        policy.scope = None;
        assert!(!policy.auto_sprint_for(7, 0));
        policy.scope = Some((7, 0));
        policy.auto_sprint = false;
        assert!(!policy.auto_sprint_for(7, 0));
    }
}
