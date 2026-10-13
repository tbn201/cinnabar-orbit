#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemKey {
    pub network_id: i32,
    pub metadata: u32,
    pub block_runtime_id: i32,
    pub nbt_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolScore {
    pub harvest: bool,
    pub progress: f32,
    pub usable: bool,
    pub silk_touch: u8,
    pub fortune: u8,
}

impl ToolScore {
    fn better_than(self, other: Self) -> bool {
        self.usable
            && self.progress.is_finite()
            && self.progress > 0.0
            && self.silk_touch >= other.silk_touch
            && self.fortune >= other.fortune
            && (self.harvest && !other.harvest
                || self.harvest == other.harvest && self.progress > other.progress)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub key: ItemKey,
    pub count: u16,
    pub tool: Option<ToolScore>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Slot {
    pub known: bool,
    pub item: Option<Item>,
}

pub struct Frame<'a> {
    pub scope: (u64, i32),
    pub selected: u8,
    pub slots: &'a [Slot; 9],
    pub target: Option<[i32; 3]>,
    pub hand: Option<ToolScore>,
    pub attack: bool,
    pub using: bool,
    pub manual: bool,
    pub auto_stack: bool,
    pub auto_tool: bool,
}

#[derive(Default)]
pub struct Selection {
    scope: Option<(u64, i32)>,
    selected: Option<u8>,
    previous: Option<ItemKey>,
    target: Option<[i32; 3]>,
    suppressed: bool,
}

impl Selection {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn select(&mut self, frame: Frame<'_>) -> Option<u8> {
        if self.scope != Some(frame.scope) {
            self.reset();
            self.scope = Some(frame.scope);
        }
        let current = frame.slots.get(usize::from(frame.selected))?;
        let changed = self
            .selected
            .is_some_and(|selected| selected != frame.selected);
        if !frame.attack && !frame.using {
            self.suppressed = false;
        }
        if frame.manual || changed {
            self.suppressed = frame.attack || frame.using;
            self.previous = None;
        }
        let target = frame.target.filter(|_| frame.attack && !frame.using);
        if !current.known {
            self.selected = Some(frame.selected);
            self.target = None;
            return None;
        }
        let choice = if self.suppressed || frame.manual || changed || !current.known {
            None
        } else if frame.auto_stack && (frame.using || frame.attack) && current.item.is_none() {
            self.previous.as_ref().and_then(|key| {
                frame.slots.iter().enumerate().find_map(|(index, slot)| {
                    let item = slot.item.as_ref()?;
                    (slot.known
                        && item.count > 0
                        && &item.key == key
                        && index != usize::from(frame.selected))
                    .then_some(index as u8)
                })
            })
        } else if frame.auto_tool && target.is_some() && target != self.target {
            best_tool(current, &frame)
        } else {
            None
        };
        self.target = target;
        let selected = choice.unwrap_or(frame.selected);
        self.selected = Some(selected);
        self.previous = frame.slots[usize::from(selected)]
            .item
            .as_ref()
            .filter(|item| item.count > 0)
            .map(|item| item.key.clone());
        choice
    }
}

fn best_tool(current: &Slot, frame: &Frame<'_>) -> Option<u8> {
    let mut best = current
        .item
        .as_ref()
        .and_then(|item| item.tool)
        .or(frame.hand)?;
    let mut choice = None;
    for (index, slot) in frame.slots.iter().enumerate() {
        if !slot.known || index == usize::from(frame.selected) {
            continue;
        }
        let Some(item) = slot.item.as_ref().filter(|item| item.count > 0) else {
            continue;
        };
        let Some(score) = item.tool else { continue };
        if score.better_than(best) {
            best = score;
            choice = Some(index as u8);
        }
    }
    choice
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i32) -> Slot {
        Slot {
            known: true,
            item: Some(Item {
                key: ItemKey {
                    network_id: id,
                    metadata: 0,
                    block_runtime_id: 0,
                    nbt_digest: [0; 32],
                },
                count: 1,
                tool: None,
            }),
        }
    }

    fn score(harvest: bool, progress: f32) -> ToolScore {
        ToolScore {
            harvest,
            progress,
            usable: true,
            silk_touch: 0,
            fortune: 0,
        }
    }

    fn frame(slots: &[Slot; 9]) -> Frame<'_> {
        Frame {
            scope: (1, 0),
            selected: 0,
            slots,
            target: None,
            hand: Some(score(false, 0.01)),
            attack: false,
            using: true,
            manual: false,
            auto_stack: true,
            auto_tool: true,
        }
    }

    fn mining(slots: &[Slot; 9]) -> Frame<'_> {
        Frame {
            using: false,
            attack: true,
            target: Some([1, 2, 3]),
            ..frame(slots)
        }
    }

    #[test]
    fn exhausted_stack_switches_only_to_an_exact_known_hotbar_match() {
        let mut state = Selection::default();
        let mut slots = std::array::from_fn(|_| Slot {
            known: true,
            item: None,
        });
        slots[0] = item(1);
        slots[1] = item(1);
        slots[1].item.as_mut().unwrap().key.nbt_digest = [1; 32];
        slots[2] = item(1);
        slots[2].known = false;
        slots[3] = item(2);
        slots[4] = item(1);
        assert_eq!(state.select(frame(&slots)), None);
        slots[0].item = None;
        assert_eq!(state.select(frame(&slots)), Some(4));
        let mut next = frame(&slots);
        next.selected = 4;
        assert_eq!(state.select(next), None);
    }

    #[test]
    fn manual_selection_and_session_changes_cancel_refill_intent() {
        for manual in [false, true] {
            let mut state = Selection::default();
            let mut slots = std::array::from_fn(|_| Slot {
                known: true,
                item: None,
            });
            slots[0] = item(1);
            slots[1] = item(1);
            state.select(frame(&slots));
            slots[0].item = None;
            let mut next = frame(&slots);
            if manual {
                next.manual = true;
            } else {
                next.scope = (2, 0);
            }
            assert_eq!(state.select(next), None);
            assert_eq!(state.select(frame(&slots)), None);
        }
    }

    #[test]
    fn auto_tool_prioritizes_harvesting_and_preserves_selected_enchantments() {
        let mut state = Selection::default();
        let mut slots = std::array::from_fn(|_| Slot {
            known: true,
            item: None,
        });
        slots[0] = item(1);
        slots[0].item.as_mut().unwrap().tool = Some(score(false, 0.4));
        slots[1] = item(2);
        slots[1].item.as_mut().unwrap().tool = Some(score(true, 0.1));
        slots[2] = item(3);
        slots[2].item.as_mut().unwrap().tool = Some(score(true, 0.2));
        assert_eq!(state.select(mining(&slots)), Some(2));
        state.reset();
        slots[0]
            .item
            .as_mut()
            .unwrap()
            .tool
            .as_mut()
            .unwrap()
            .silk_touch = 1;
        assert_eq!(state.select(mining(&slots)), None);
        slots[1]
            .item
            .as_mut()
            .unwrap()
            .tool
            .as_mut()
            .unwrap()
            .silk_touch = 1;
        state.reset();
        assert_eq!(state.select(mining(&slots)), Some(1));
    }

    #[test]
    fn mining_does_not_fight_manual_changes_or_use_actions() {
        let mut state = Selection::default();
        let mut slots = std::array::from_fn(|_| Slot {
            known: true,
            item: None,
        });
        slots[0] = item(1);
        slots[1] = item(2);
        slots[1].item.as_mut().unwrap().tool = Some(score(true, 0.2));
        let mut start = mining(&slots);
        start.manual = true;
        assert_eq!(state.select(start), None);
        assert_eq!(state.select(mining(&slots)), None);
        assert_eq!(state.select(frame(&slots)), None);
        let mut released = frame(&slots);
        released.using = false;
        assert_eq!(state.select(released), None);
        assert_eq!(state.select(mining(&slots)), Some(1));
        state.reset();
        assert_eq!(
            state.select(Frame {
                using: true,
                ..mining(&slots)
            }),
            None
        );
    }

    #[test]
    fn disabled_unknown_tied_or_worn_tools_never_force_a_switch() {
        for candidate in [
            score(true, 0.01),
            ToolScore {
                usable: false,
                ..score(true, 1.0)
            },
            ToolScore {
                progress: f32::NAN,
                ..score(true, 1.0)
            },
        ] {
            let mut state = Selection::default();
            let mut slots = std::array::from_fn(|_| Slot {
                known: true,
                item: None,
            });
            slots[0] = item(1);
            slots[0].item.as_mut().unwrap().tool = Some(score(true, 0.01));
            slots[1] = item(2);
            slots[1].item.as_mut().unwrap().tool = Some(candidate);
            let mining = Frame {
                using: false,
                attack: true,
                target: Some([1, 2, 3]),
                ..frame(&slots)
            };
            assert_eq!(state.select(mining), None);
        }
    }
    #[test]
    fn unresolved_inventory_does_not_lose_refill_intent_or_select_unknown_items() {
        let mut state = Selection::default();
        let mut slots = std::array::from_fn(|_| Slot {
            known: true,
            item: None,
        });
        slots[0] = item(1);
        slots[1] = item(1);
        state.select(frame(&slots));
        slots[0] = Slot::default();
        assert_eq!(state.select(frame(&slots)), None);
        slots[0].known = true;
        assert_eq!(state.select(frame(&slots)), Some(1));
        state.reset();
        slots[0] = item(1);
        state.select(frame(&slots));
        slots[0].item = None;
        let mut disabled = frame(&slots);
        disabled.auto_stack = false;
        assert_eq!(state.select(disabled), None);
    }
}
