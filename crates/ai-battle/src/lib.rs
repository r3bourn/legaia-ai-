use legaia_ai_controller::{Button, ControllerPlan};
use legaia_engine_core::world::{SceneMode, World};
use legaia_engine_menus::battle_input::CommandPhase;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct RoutineBattlePolicy {
    /// Below this fraction the routine policy refuses to keep attacking and
    /// escalates to the higher-level planner. This is deliberately conservative:
    /// healing/item/magic selection is its own policy, not something a blind
    /// physical-attack loop should improvise.
    pub min_hp_fraction: f32,
}

impl Default for RoutineBattlePolicy {
    fn default() -> Self {
        Self {
            min_hp_fraction: 0.35,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BattleDirective {
    Controller {
        plan: ControllerPlan,
        reason: String,
    },
    Wait {
        reason: String,
    },
    NeedDecision {
        reason: String,
    },
    NotInBattle,
}

/// Produce at most one tiny input step for an ordinary player-driven battle.
///
/// The executor follows the engine's own semantic command phase rather than
/// replaying a timed macro:
///
/// RoundPrompt -> Left (Begin)
/// Menu        -> Left (Attack)
/// AttackMode  -> Left (Auto)
/// Targeting   -> Cross (current/first living target)
/// Commit      -> Left (Begin round)
///
/// Between those prompts the battle action state machine runs on its own and
/// this policy returns `Wait`. Any submenu or low-health condition escalates
/// instead of guessing.
pub fn routine_battle_step(world: &World, policy: RoutineBattlePolicy) -> BattleDirective {
    if world.mode != SceneMode::Battle {
        return BattleDirective::NotInBattle;
    }

    if world.battle.victory.is_some() || world.battle.end.is_some() {
        return BattleDirective::Wait {
            reason: "battle is in end/results presentation".into(),
        };
    }

    if !party_health_is_routine_safe(world, policy.min_hp_fraction) {
        return BattleDirective::NeedDecision {
            reason: "party HP is below the routine-attack safety threshold".into(),
        };
    }

    if world.battle.item_menu.is_some()
        || world.battle.spell_menu.is_some()
        || world.battle.arts_menu.is_some()
        || world.battle.arts_input.is_some()
    {
        return BattleDirective::NeedDecision {
            reason: "a non-routine battle submenu is already open".into(),
        };
    }

    let Some(command) = world.battle.command.as_ref() else {
        return BattleDirective::Wait {
            reason: "battle action/animation is running".into(),
        };
    };

    match &command.phase {
        CommandPhase::RoundPrompt { .. } => tap(Button::Left, "choose Begin at the round prompt"),
        CommandPhase::Menu { .. } => {
            tap(Button::Left, "choose Attack from the battle command ring")
        }
        CommandPhase::AttackMode { .. } => tap(Button::Left, "choose Auto attack mode"),
        CommandPhase::Targeting { .. } => {
            tap(Button::Cross, "confirm the current living enemy target")
        }
        CommandPhase::CommitConfirm { .. } => tap(
            Button::Left,
            "confirm Begin after party commands are staged",
        ),

        // These are transient resolved states consumed by the live battle loop
        // on its next tick. Adding input here can turn one press into a press on
        // the following surface, so the safe answer is to wait one observation.
        CommandPhase::Confirmed { .. }
        | CommandPhase::SpiritGuard
        | CommandPhase::RunAway
        | CommandPhase::Aborted
        | CommandPhase::StepBack
        | CommandPhase::BeginRound
        | CommandPhase::Reselect => BattleDirective::Wait {
            reason: "battle command is resolving; wait for the next semantic phase".into(),
        },

        CommandPhase::OpenArtsMenu | CommandPhase::OpenSpellMenu | CommandPhase::OpenItemMenu => {
            BattleDirective::NeedDecision {
                reason: "battle handed off to a non-routine submenu".into(),
            }
        }
    }
}

fn tap(button: Button, reason: &str) -> BattleDirective {
    BattleDirective::Controller {
        plan: ControllerPlan::tap(button),
        reason: reason.to_string(),
    }
}

fn party_health_is_routine_safe(world: &World, min_fraction: f32) -> bool {
    let threshold = min_fraction.clamp(0.0, 1.0);
    let count = usize::from(world.party.party_count.min(3));
    (0..count).all(|member| {
        let roster_slot = world.party_roster_slot(member);
        let Some(record) = world.party.roster.members.get(roster_slot) else {
            return false;
        };
        let hms = record.hp_mp_sp();
        if hms.hp_max == 0 {
            return true;
        }
        f32::from(hms.hp_cur) / f32::from(hms.hp_max) >= threshold
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use legaia_engine_menus::battle_input::BattleCommandSession;

    /// A battle world with one party member at `hp_cur`/`hp_max`. The routine
    /// health gate escalates when a counted member has no roster record, so
    /// phase tests need a real record, not just `party_count`.
    fn battle_world(hp_cur: u16, hp_max: u16) -> World {
        let mut world = World::new();
        let mut party = legaia_save::Party::zeroed(1);
        let mut hms = party.members[0].hp_mp_sp();
        hms.hp_max = hp_max;
        hms.hp_cur = hp_cur;
        party.members[0].set_hp_mp_sp(hms);
        world.load_party(party);
        world.party.party_count = 1;
        world.mode = SceneMode::Battle;
        world
    }

    #[test]
    fn outside_battle_does_nothing() {
        let world = World::new();
        assert_eq!(
            routine_battle_step(&world, RoutineBattlePolicy::default()),
            BattleDirective::NotInBattle
        );
    }

    #[test]
    fn round_prompt_starts_with_left_begin() {
        let mut world = battle_world(200, 200);
        world.battle.command = Some(BattleCommandSession::new_round_open(0, 0, false));
        let out = routine_battle_step(
            &world,
            RoutineBattlePolicy {
                min_hp_fraction: 0.0,
            },
        );
        match out {
            BattleDirective::Controller { plan, .. } => {
                assert_eq!(plan.steps[0].buttons, vec![Button::Left]);
            }
            other => panic!("unexpected directive: {other:?}"),
        }
    }

    #[test]
    fn menu_chooses_attack_with_left() {
        let mut world = battle_world(200, 200);
        world.battle.command = Some(BattleCommandSession::new(0, 0));
        let out = routine_battle_step(
            &world,
            RoutineBattlePolicy {
                min_hp_fraction: 0.0,
            },
        );
        match out {
            BattleDirective::Controller { plan, .. } => {
                assert_eq!(plan.steps[0].buttons, vec![Button::Left]);
            }
            other => panic!("unexpected directive: {other:?}"),
        }
    }

    #[test]
    fn low_hp_escalates_instead_of_attacking() {
        let mut world = battle_world(50, 200);
        world.battle.command = Some(BattleCommandSession::new(0, 0));
        assert!(matches!(
            routine_battle_step(&world, RoutineBattlePolicy::default()),
            BattleDirective::NeedDecision { .. }
        ));
    }
}
