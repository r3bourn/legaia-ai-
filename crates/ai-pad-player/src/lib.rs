//! Deterministic per-frame pad policies for the live Legaia menu state machines.
//!
//! This crate does not mutate the game. It inspects the same `World`,
//! `MenuRuntime`, and `FieldMenuSubsession` state the real hosts render and
//! returns at most one edge-triggered PSX pad word for the next tick.
//!
//! Callers must still feed a neutral pad tick between repeated non-zero edges.
//! The authoritative live player owns that cadence; these functions only
//! choose the next edge.

use legaia_engine_core::equip_session::{
    ENGINE_ONLY_HAND_GUARD_SLOT, EquipState, browse_row_for_slot, record_byte_for_engine_slot,
};
use legaia_engine_core::equipment::EquipSlot;
use legaia_engine_core::field_menu_dispatch::FieldMenuSubsession;
use legaia_engine_core::input::PadButton;
use legaia_engine_core::menu_runtime::{MenuRuntime, MenuState, shop_list_page_rows};
use legaia_engine_core::shop::SHOP_HELD_CAP;
use legaia_engine_core::world::World;

/// Rich policy answer. The `*_pad` convenience functions below collapse this
/// to `Option<u16>` for the imperative player, while orchestration code can
/// keep the reason when a goal cannot be acted on in the current screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PadDecision {
    /// Apply this edge-triggered pad word on the next simulation tick.
    Press(u16),
    /// No more work from this policy in the current shop/equip session.
    Done,
    /// Let the current state machine advance one neutral tick before asking
    /// again (for example the one-tick ShopQuantity handoff).
    Wait,
    /// The requested goal cannot be completed from the current live state.
    Blocked(BlockReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockReason {
    NotInShop,
    ItemNotStocked { item_id: u8 },
    InsufficientGold { item_id: u8, price: u32, gold: i32 },
    StackFull { item_id: u8 },
    UnexpectedShopState { state: u8 },
    NotInEquipMenu,
    PartyMemberAbsent { party_slot: u8 },
    EquipSlotUnavailable { slot: u8 },
    EquipItemUnavailable { item_id: u8, slot: u8 },
}

#[inline]
fn press(button: PadButton) -> PadDecision {
    PadDecision::Press(button.mask())
}

/// Idempotent shop goal: `(item_id, target_final_bag_count)`.
///
/// Unlike "buy N", this is safe across autosave/resume and policy replay. If
/// the bag already holds the target count, the policy does nothing for it.
pub type ShopBuyGoal = (u8, u8);

/// Return the next shop pad edge, or `None` when the rich policy says Done,
/// Wait, or Blocked. Prefer [`shop_buy_decision`] when diagnostics matter.
pub fn shop_buy_pad(
    world: &World,
    menu: &MenuRuntime,
    desired: &[ShopBuyGoal],
) -> Option<u16> {
    match shop_buy_decision(world, menu, desired) {
        PadDecision::Press(mask) => Some(mask),
        PadDecision::Done | PadDecision::Wait | PadDecision::Blocked(_) => None,
    }
}

/// Drive the live shop toward a set of final bag counts.
///
/// Equipment rows are deliberately bought to **bag row 0** in the retail
/// recipient picker. Equipping is a separate, idempotent policy, so a reconnect
/// cannot accidentally buy a second weapon just because the direct-equip
/// recipient flow was interrupted.
pub fn shop_buy_decision(
    world: &World,
    menu: &MenuRuntime,
    desired: &[ShopBuyGoal],
) -> PadDecision {
    let Some(shop) = menu.shop_session.as_ref() else {
        return if desired.iter().all(|&(id, qty)| owned(world, id) >= qty) {
            PadDecision::Done
        } else {
            PadDecision::Blocked(BlockReason::NotInShop)
        };
    };

    // Sub-screens own the pad even though the menu ctx remains parked on the
    // buy list. Finish/cancel them before reasoning about the list cursor.
    if let Some(recipient) = menu.recipient_session.as_ref() {
        let target = target_qty(desired, recipient.item_id);
        let have = owned(world, recipient.item_id);
        if target.is_none() || have >= target.unwrap_or(0) {
            // Safe in every recipient phase: cancel in Interactive, dismiss in
            // ToastWait, ignored-then-exit in Exit.
            return press(PadButton::Triangle);
        }
        return if recipient.cursor == 0 {
            // Row 0 is BAG. Party rows buy-and-equip directly, which is not
            // idempotent enough for reconnect recovery.
            press(PadButton::Cross)
        } else {
            press(PadButton::Up)
        };
    }

    if let Some(view) = menu.quantity_view() {
        if !view.buying {
            return press(PadButton::Triangle);
        }
        let Some(target) = target_qty(desired, view.item_id) else {
            return press(PadButton::Triangle);
        };
        let have = owned(world, view.item_id);
        if have >= target {
            // Cancels an unneeded interactive picker, or dismisses its
            // post-purchase toast/exit beat without another purchase.
            return press(PadButton::Triangle);
        }
        let need = target.saturating_sub(have).max(1);
        let wanted = need.min(view.max.max(1));
        return quantity_pad(view.quantity, wanted);
    }

    let state = MenuState::from_byte(menu.ctx.state);
    match state {
        Some(MenuState::ShopMenu) => {
            // Buy is row 0 regardless of whether the optional Trade row exists.
            if menu.ctx.cursor == 0 {
                press(PadButton::Cross)
            } else {
                press(PadButton::Up)
            }
        }
        Some(MenuState::ShopBuy) => {
            let Some((row, item_id, target)) = first_stocked_unmet_goal(world, shop, desired) else {
                return PadDecision::Done;
            };
            let have = owned(world, item_id);
            if have >= SHOP_HELD_CAP && target > have {
                return PadDecision::Blocked(BlockReason::StackFull { item_id });
            }
            let item = &shop.inventory.items[row];
            if item.price > 0 && i64::from(world.party.money) < i64::from(item.price) {
                return PadDecision::Blocked(BlockReason::InsufficientGold {
                    item_id,
                    price: item.price,
                    gold: world.party.money,
                });
            }
            list_cursor_pad(menu.ctx.cursor as usize, row, shop_list_page_rows(MenuState::ShopBuy))
        }
        // The current runtime creates the live quantity picker immediately
        // after this state is entered. A neutral tick is the correct input.
        Some(MenuState::ShopQuantity) => PadDecision::Wait,
        // Legacy/host-built flow: Yes is slot 0.
        Some(MenuState::ShopConfirm) => {
            if menu.ctx.cursor == 0 {
                press(PadButton::Cross)
            } else {
                press(PadButton::Left)
            }
        }
        // Back out of unrelated shop branches to the root picker.
        Some(MenuState::ShopSell | MenuState::ShopTrade | MenuState::ShopTradeConfirm) => {
            press(PadButton::Triangle)
        }
        Some(MenuState::ShopExit | MenuState::Closing | MenuState::Deactivate) => PadDecision::Wait,
        Some(other) => PadDecision::Blocked(BlockReason::UnexpectedShopState {
            state: other.as_byte(),
        }),
        None => PadDecision::Blocked(BlockReason::UnexpectedShopState {
            state: menu.ctx.state,
        }),
    }
}

fn first_stocked_unmet_goal(
    world: &World,
    shop: &legaia_engine_core::shop::ShopSession,
    desired: &[ShopBuyGoal],
) -> Option<(usize, u8, u8)> {
    desired.iter().find_map(|&(item_id, target)| {
        if target == 0 || owned(world, item_id) >= target {
            return None;
        }
        shop.inventory
            .items
            .iter()
            .position(|i| i.item_id == item_id)
            .map(|row| (row, item_id, target))
    })
}

fn target_qty(desired: &[ShopBuyGoal], item_id: u8) -> Option<u8> {
    desired
        .iter()
        .filter(|&&(id, _)| id == item_id)
        .map(|&(_, qty)| qty)
        .max()
}

fn owned(world: &World, item_id: u8) -> u8 {
    world.party.inventory.get(&item_id).copied().unwrap_or(0)
}

fn quantity_pad(current: u8, wanted: u8) -> PadDecision {
    if current == wanted {
        return press(PadButton::Cross);
    }
    if current < wanted {
        if wanted - current >= 10 {
            press(PadButton::Down) // retail quantity step: +10
        } else {
            press(PadButton::Right) // +1
        }
    } else if current - wanted >= 10 {
        press(PadButton::Up) // -10
    } else {
        press(PadButton::Left) // -1
    }
}

fn list_cursor_pad(current: usize, target: usize, page_rows: Option<usize>) -> PadDecision {
    if current == target {
        return press(PadButton::Cross);
    }
    if let Some(rows) = page_rows.filter(|&r| r > 0) {
        let current_page = current / rows;
        let target_page = target / rows;
        if current_page < target_page {
            return press(PadButton::Right);
        }
        if current_page > target_page {
            return press(PadButton::Left);
        }
    }
    if current < target {
        press(PadButton::Down)
    } else {
        press(PadButton::Up)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EquipTarget {
    /// Roster/present-party slot used by the engine's Equip character picker.
    pub party_slot: u8,
    pub slot: EquipSlot,
    pub item_id: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EquipPlan {
    pub targets: Vec<EquipTarget>,
}

/// Return one Equip-menu pad edge, or `None` when the rich policy is not
/// requesting a press. The live equip sub-session is a separate state owner
/// from `World`, so it is intentionally an explicit argument.
pub fn equip_pad(
    world: &World,
    active: &FieldMenuSubsession,
    plan: &EquipPlan,
) -> Option<u16> {
    match equip_decision(world, active, plan) {
        PadDecision::Press(mask) => Some(mask),
        PadDecision::Done | PadDecision::Wait | PadDecision::Blocked(_) => None,
    }
}

pub fn equip_decision(
    world: &World,
    active: &FieldMenuSubsession,
    plan: &EquipPlan,
) -> PadDecision {
    let Some(target) = plan.targets.iter().copied().find(|t| !is_equipped(world, *t)) else {
        return PadDecision::Done;
    };

    if target.slot.as_index() == ENGINE_ONLY_HAND_GUARD_SLOT {
        return PadDecision::Blocked(BlockReason::EquipSlotUnavailable {
            slot: target.slot.as_index(),
        });
    }

    let FieldMenuSubsession::Equip {
        session,
        char_slot,
        picking,
    } = active
    else {
        return PadDecision::Blocked(BlockReason::NotInEquipMenu);
    };

    let party = world.present_party_list();
    let Some(target_party_index) = party.iter().position(|&s| s == target.party_slot) else {
        return PadDecision::Blocked(BlockReason::PartyMemberAbsent {
            party_slot: target.party_slot,
        });
    };
    let current_party_index = party.iter().position(|&s| s == *char_slot).unwrap_or(0);

    if *picking {
        if *char_slot == target.party_slot {
            return press(PadButton::Cross);
        }
        return if current_party_index < target_party_index {
            press(PadButton::Down)
        } else {
            press(PadButton::Up)
        };
    }

    // A different character is active inside the slot/item flow. Cancel back
    // to the character picker first; the shared dispatcher performs the
    // session rebuild on the subsequent Up/Down edge.
    if *char_slot != target.party_slot {
        return press(PadButton::Circle);
    }

    match session.state() {
        EquipState::SlotPicker { cursor } => {
            let wanted = browse_row_for_slot(target.slot.as_index());
            if cursor == wanted {
                press(PadButton::Cross)
            } else if cursor < wanted {
                press(PadButton::Down)
            } else {
                press(PadButton::Up)
            }
        }
        EquipState::ItemPicker { slot, cursor } => {
            if slot != target.slot.as_index() {
                return press(PadButton::Circle);
            }
            let items = session.items_for_slot(slot);
            let Some(row) = items.iter().position(|item| item.id == target.item_id && item.owned) else {
                return PadDecision::Blocked(BlockReason::EquipItemUnavailable {
                    item_id: target.item_id,
                    slot,
                });
            };
            list_cursor_pad(cursor as usize, row, None)
        }
        EquipState::Confirm {
            slot,
            item_id,
            cursor,
        } => {
            if slot != target.slot.as_index() || item_id != target.item_id {
                return press(PadButton::Circle);
            }
            if cursor == 0 {
                press(PadButton::Cross)
            } else {
                press(PadButton::Left)
            }
        }
        // The host must fold the EquipOutcome back into World before a second
        // target can be evaluated. Sending another edge here would be lost.
        EquipState::Done(_) => PadDecision::Wait,
    }
}

pub fn is_equipped(world: &World, target: EquipTarget) -> bool {
    let Some(record) = world.party.roster.members.get(target.party_slot as usize) else {
        return false;
    };
    let equipment = record.equipment();
    let byte = record_byte_for_engine_slot(target.slot.as_index(), target.party_slot);
    equipment.slots.get(byte).copied() == Some(target.item_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use legaia_engine_core::battle_stats::{EquipmentTable, StatRecord, StatusModifiers};
    use legaia_engine_core::equip_session::{EquipInput, EquipSession};
    use legaia_engine_core::item_bag::ItemBag;
    use legaia_engine_core::shop::{
        BuyQuantitySession, QuantityPicker, ShopInventory, ShopItem, ShopSession,
    };

    fn stock(ids: &[u8]) -> ShopSession {
        ShopSession::new(ShopInventory::new(
            0,
            ids.iter()
                .copied()
                .map(|item_id| ShopItem {
                    item_id,
                    price: 100,
                })
                .collect(),
        ))
    }

    #[test]
    fn shop_root_enters_buy() {
        let mut world = World::new();
        world.party.money = 1_000;
        let mut menu = MenuRuntime::new(".");
        menu.open_shop_menu(stock(&[0x77]));
        assert_eq!(
            shop_buy_decision(&world, &menu, &[(0x77, 3)]),
            press(PadButton::Cross)
        );
    }

    #[test]
    fn shop_buy_uses_page_flip_for_eighth_row() {
        let mut world = World::new();
        world.party.money = 10_000;
        let ids: Vec<u8> = (1..=10).collect();
        let mut menu = MenuRuntime::new(".");
        menu.open_shop(stock(&ids));
        menu.ctx.state = MenuState::ShopBuy.as_byte();
        menu.ctx.cursor = 0;
        assert_eq!(
            shop_buy_decision(&world, &menu, &[(8, 1)]),
            press(PadButton::Right)
        );
    }

    #[test]
    fn quantity_policy_uses_retail_ten_step() {
        let mut world = World::new();
        world.party.money = 10_000;
        let mut menu = MenuRuntime::new(".");
        menu.open_shop(stock(&[0x77]));
        menu.ctx.state = MenuState::ShopQuantity.as_byte();
        menu.quantity_session = Some(QuantityPicker::Buy(BuyQuantitySession::new(
            0x77,
            100,
            10_000,
            Some(0),
            false,
        )));
        assert_eq!(
            shop_buy_decision(&world, &menu, &[(0x77, 12)]),
            press(PadButton::Down)
        );
    }

    fn equip_subsession_with_weapon(item_id: u8) -> FieldMenuSubsession {
        let mut bag = ItemBag::new();
        assert!(bag.add(item_id, 1));
        let session = EquipSession::new(
            StatRecord::default(),
            bag,
            EquipmentTable::default(),
            StatusModifiers::default(),
            Vec::new(),
        );
        FieldMenuSubsession::Equip {
            session,
            char_slot: 0,
            picking: false,
        }
    }

    #[test]
    fn equip_policy_moves_from_best_row_to_weapon_row() {
        let world = World::new();
        let active = equip_subsession_with_weapon(0x01);
        let plan = EquipPlan {
            targets: vec![EquipTarget {
                party_slot: 0,
                slot: EquipSlot::Weapon,
                item_id: 0x01,
            }],
        };
        assert_eq!(
            equip_decision(&world, &active, &plan),
            press(PadButton::Down)
        );
    }

    #[test]
    fn equip_policy_selects_live_candidate_row() {
        let world = World::new();
        let mut bag = ItemBag::new();
        assert!(bag.add(0x01, 1));
        let mut session = EquipSession::new(
            StatRecord::default(),
            bag,
            EquipmentTable::default(),
            StatusModifiers::default(),
            Vec::new(),
        );
        session.input(EquipInput {
            down: true,
            ..Default::default()
        });
        session.input(EquipInput {
            cross: true,
            ..Default::default()
        });
        let active = FieldMenuSubsession::Equip {
            session,
            char_slot: 0,
            picking: false,
        };
        let plan = EquipPlan {
            targets: vec![EquipTarget {
                party_slot: 0,
                slot: EquipSlot::Weapon,
                item_id: 0x01,
            }],
        };
        assert_eq!(
            equip_decision(&world, &active, &plan),
            press(PadButton::Cross)
        );
    }
}
