use crate::runtime::world::ClientWorld;
use bevy::{prelude::*, time::Real};
use client_shell::ui_resources::UiRuntimeResource;
use client_state::PlayerRuntime;
use inventory::{
    PlayerInventoryLedger,
    inventory_ledger::{InventoryTarget, TOTEM_IDENTIFIER},
};

const RETRY_SECONDS: f64 = 0.25;
const ALWAYS_READY_HEALTH: f32 = 20.0;

pub(super) fn configure(app: &mut App) {
    app.add_systems(
        Update,
        equip_totem
            .in_set(client_state::ClientFrameSet::NetworkSend)
            .before(crate::ui_runtime::flush_inventory_network),
    );
}

#[derive(Default)]
struct Equipper {
    retry_at: f64,
}

fn equip_totem(
    mut player: ResMut<PlayerRuntime>,
    mut ui: ResMut<UiRuntimeResource>,
    world: Res<ClientWorld>,
    policy: Option<Res<super::assist::ModAssist>>,
    time: Res<Time<Real>>,
    mut equipper: Local<Equipper>,
) {
    let Some(stream) = world.stream.as_ref() else {
        return;
    };
    let threshold = policy.as_deref().and_then(|policy| {
        policy.auto_totem_for(player.facts.session_id(), stream.current_dimension())
    });
    let Some(threshold) = threshold else {
        equipper.retry_at = 0.0;
        return;
    };
    let health = ui
        .hud()
        .health()
        .map(|health| f32::from(health.current()) / f32::from(health.scale().max(1)));
    if !health.is_some_and(|health| needs_totem(health, threshold))
        || ui.inventory_open()
        || ui.ui_focused(&player)
        || world.dimension_transfer.active()
    {
        return;
    }
    let now = time.elapsed_secs_f64();
    if now < equipper.retry_at {
        return;
    }
    let Some(slot) = totem_source(ui.inventory_ledger(&player), player.selected_hotbar_slot())
    else {
        return;
    };
    if ui
        .inventory_ledger_mut(&mut player)
        .begin_world_offhand_equip(slot)
        .is_ok()
    {
        equipper.retry_at = now + RETRY_SECONDS;
    }
}

fn needs_totem(health: f32, threshold: f32) -> bool {
    health > 0.0 && (threshold >= ALWAYS_READY_HEALTH || health <= threshold)
}

fn totem_source(ledger: &PlayerInventoryLedger, selected: Option<u8>) -> Option<u8> {
    let is_totem = |network_id: i32| {
        ledger
            .negotiated_item_entry(network_id)
            .is_some_and(|entry| entry.identifier.as_ref() == TOTEM_IDENTIFIER)
    };
    if ledger.pending_request_count() != 0
        || ledger.resync_required()
        || ledger.storage_generation().is_some()
        || ledger
            .target_stack(InventoryTarget::Offhand)
            .is_some_and(|stack| is_totem(stack.network_id))
    {
        return None;
    }
    let totems = (0..protocol::PLAYER_INVENTORY_SLOTS).filter(|slot| {
        !ledger.slot_pending(*slot)
            && ledger
                .displayed_stack(*slot)
                .is_some_and(|stack| stack.count > 0 && is_totem(stack.network_id))
    });
    preferred_slot(totems, selected)
}

fn preferred_slot(slots: impl Iterator<Item = u8>, selected: Option<u8>) -> Option<u8> {
    slots.min_by_key(|slot| {
        let hotbar = *slot < protocol::HOTBAR_SLOT_COUNT;
        (hotbar, Some(*slot) == selected, *slot)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totems_outside_the_hotbar_are_preferred_and_the_held_slot_is_last() {
        assert_eq!(preferred_slot([2, 30, 12].into_iter(), Some(2)), Some(12));
        assert_eq!(preferred_slot([4, 2].into_iter(), Some(2)), Some(4));
        assert_eq!(preferred_slot([2].into_iter(), Some(2)), Some(2));
        assert_eq!(preferred_slot(std::iter::empty(), None), None);
    }

    #[test]
    fn health_threshold_is_inclusive_and_twenty_always_keeps_a_totem() {
        assert!(needs_totem(8.0, 8.0));
        assert!(!needs_totem(8.5, 8.0));
        assert!(needs_totem(40.0, 20.0));
        assert!(!needs_totem(0.0, 20.0));
    }
}
