//! The keyboard-to-pad binding table, read from the engine instead of typed
//! into the page.
//!
//! Every browser page that takes input needs a `KeyboardEvent.code -> PSX pad
//! bit` table. There used to be two of them, hand-written in JS, and neither
//! agreed with the engine's own default layout
//! (`legaia_engine_core::input::Mapping::default`) or with each other:
//!
//! | key | native window | play page | boot / title |
//! |---|---|---|---|
//! | `X` | Square | Circle | Circle |
//! | `S` | Circle | Down | Down |
//! | `A` | Triangle | Left | - |
//! | `W` | R1 | Up | Up |
//! | `RShift` | Select | - | - |
//! | `Q` / `1` / `2` | L1 / L2 / R2 | - | - |
//!
//! Nothing about that is visible in a diff, because no file holds two of the
//! columns. The fix is not to correct the tables - it is to delete them: the
//! pages call [`pad_bindings_json`] and use whatever the engine binds, so a
//! rebind lands on every host at once and a disagreement can no longer be
//! written down.
//!
//! The layout served here *starts* at `Mapping::web_default`, not
//! `Mapping::default`. The browser page binds `WASD` to the d-pad and the
//! desktop layout spends those keys on Triangle / Circle / R1, so the two
//! cannot be one table - a `HashMap<key, button>` holds either `S -> Down` or
//! `S -> Circle`, never both. They are two *named layouts in the engine*
//! rather than one layout plus a page-side override, which keeps the
//! single-source-of-truth property that made the tables above deletable.
//!
//! "Starts at", because the table is now **editable**: the options screen's
//! Key Config screen ([`legaia_engine_core::key_rebind::KeyRebindSession`],
//! reached through [`legaia_engine_core::options::OptionsSession`]) commits
//! binds into it, and [`store_mapping`] round-trips the result through
//! `localStorage` under [`BINDINGS_STORAGE_KEY`] - the browser twin of the
//! native window's `legaia-input.toml`. A page that folded
//! [`pad_bindings_json`] into a JS object at load notices a rebind through
//! [`pad_bindings_revision`].
//!
//! A free function rather than a [`crate::runtime::LegaiaRuntime`] method
//! because the title screen's key loop runs before a runtime exists, and it
//! needs the same table.

use crate::runtime::LegaiaRuntime;
use legaia_ai_controller::{
    AiController, BattleTelemetry, ControllerPlan, DialogTelemetry, EnemyTelemetry,
    GameMode as AiGameMode, PartyMemberTelemetry, PlayerTelemetry, TelemetrySnapshot, Vec3i,
};
use legaia_ai_navigation::{
    NavigationConfig, NavigationResult, NavigationTarget, plan_field_step,
};
use legaia_engine_core::input::Mapping;
use legaia_engine_core::world::SceneMode;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

/// `localStorage` key the page's edited binding table round-trips through -
/// the browser twin of the native window's `legaia-input.toml`.
///
/// Same store, same shape and the same "absent or unparseable falls back to
/// the default layout" rule as `legaia.options` beside it, so the two
/// persisted settings behave identically.
pub const BINDINGS_STORAGE_KEY: &str = "legaia.bindings";

thread_local! {
    /// The page's **live** binding table, plus a revision counter.
    ///
    /// A module-level cell rather than a [`LegaiaRuntime`] field because the
    /// title screen's key loop runs before a runtime exists and reads the same
    /// table (which is why [`pad_bindings_json`] is a free function), and
    /// because the minigames page has no `LegaiaRuntime` at all. wasm32 is
    /// single-threaded, so there is exactly one of these per page.
    static LIVE_BINDINGS: RefCell<(Option<Mapping>, u32)> = const { RefCell::new((None, 0)) };

    /// Session-only AI input authority for the play page.
    ///
    /// Kept outside `LegaiaRuntime` deliberately: it lets the AI control layer
    /// remain an additive browser feature instead of changing the upstream
    /// runtime's enormous construction surface. wasm32 is single-threaded and
    /// the play page owns one runtime, so there is one controller per page.
    /// Every page load starts disarmed and with an empty queue.
    static AI_CONTROLLER: RefCell<AiController> = RefCell::new(AiController::default());
}

/// The live table, loading the persisted one (or the default layout) on first
/// use.
pub fn live_mapping() -> Mapping {
    LIVE_BINDINGS.with(|c| {
        let mut c = c.borrow_mut();
        if c.0.is_none() {
            c.0 = Some(load_persisted_mapping());
        }
        c.0.clone().expect("just seeded")
    })
}

/// Adopt `mapping` as the live table and persist it - the browser twin of the
/// native window writing `legaia-input.toml` when the options screen's Key
/// Config row commits a bind.
pub fn store_mapping(mapping: &Mapping) {
    LIVE_BINDINGS.with(|c| {
        let mut c = c.borrow_mut();
        c.0 = Some(mapping.clone());
        c.1 = c.1.wrapping_add(1);
    });
    #[cfg(target_arch = "wasm32")]
    if let Some(store) = bindings_storage()
        && let Ok(json) = serde_json::to_string(mapping)
        && store.set_item(BINDINGS_STORAGE_KEY, &json).is_err()
    {
        crate::console_log("bindings: localStorage write failed");
    }
}

/// Read the persisted table, falling back to [`Mapping::web_default`] when
/// nothing is stored, the store is unreachable (private mode, non-browser
/// target) or the stored JSON no longer parses.
fn load_persisted_mapping() -> Mapping {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(store) = bindings_storage()
            && let Ok(Some(raw)) = store.get_item(BINDINGS_STORAGE_KEY)
            && let Ok(m) = serde_json::from_str::<Mapping>(&raw)
            && !m.bindings.is_empty()
        {
            return m;
        }
    }
    Mapping::web_default()
}

#[cfg(target_arch = "wasm32")]
fn bindings_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// How many times the live table has changed since the page loaded. A page
/// polls this and re-reads [`pad_bindings_json`] when it moves.
#[wasm_bindgen]
pub fn pad_bindings_revision() -> u32 {
    LIVE_BINDINGS.with(|c| c.borrow().1)
}

/// Forget the persisted table and go back to [`Mapping::web_default`] - the
/// escape hatch for a player who bound themselves out of the menu.
#[wasm_bindgen]
pub fn pad_bindings_reset() {
    let d = Mapping::web_default();
    store_mapping(&d);
    #[cfg(target_arch = "wasm32")]
    if let Some(store) = bindings_storage() {
        let _ = store.remove_item(BINDINGS_STORAGE_KEY);
    }
}

/// The engine's default keyboard layout as `{ "<KeyboardEvent.code>": <bit> }`.
///
/// Bits are the PSX digital-pad masks
/// (`legaia_engine_core::input::PadButton`) the runtime's `set_pad` /
/// `*_input` entry points already take, so a page folds held or pressed codes
/// straight into a pad word with no second table.
///
/// Keys the browser has no `code` for are absent rather than guessed at - see
/// `Mapping::dom_code_bindings`. Ordering is stable across calls.
#[wasm_bindgen]
pub fn pad_bindings_json() -> String {
    let mapping = live_mapping();
    let mut out = String::from("{");
    for (i, (code, bit)) in mapping.dom_code_bindings().into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("\"{code}\":{bit}"));
    }
    out.push('}');
    out
}

/// Pad bits by button name, for the handful of places a page needs to name a
/// button rather than a key ("is confirm pressed?").
///
/// The title screen used to test `pulse.has('KeyZ') || pulse.has('Space')`,
/// which hardcodes both the layout and the assumption that confirm is a key
/// rather than a button. With this a page asks for `Cross` and stays correct
/// through a rebind.
#[wasm_bindgen]
pub fn pad_buttons_json() -> String {
    use legaia_engine_core::input::PadButton::*;
    let all = [
        Select, L3, R3, Start, Up, Right, Down, Left, L2, R2, L1, R1, Triangle, Circle, Cross,
        Square,
    ];
    let body = all
        .iter()
        .map(|b| format!("\"{}\":{}", b.name(), b.mask()))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{body}}}")
}

fn ai_now_ms(value: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        0
    } else if value >= u64::MAX as f64 {
        u64::MAX
    } else {
        value as u64
    }
}

fn ai_game_mode(mode: SceneMode) -> AiGameMode {
    match mode {
        SceneMode::Title => AiGameMode::Title,
        SceneMode::Field => AiGameMode::Field,
        SceneMode::Battle => AiGameMode::Battle,
        SceneMode::Cutscene => AiGameMode::Cutscene,
        SceneMode::WorldMap => AiGameMode::WorldMap,
        SceneMode::Menu => AiGameMode::Menu,
        SceneMode::Dance
        | SceneMode::Fishing
        | SceneMode::SlotMachine
        | SceneMode::BakaFighter
        | SceneMode::MuscleDome => AiGameMode::Minigame,
    }
}

/// Runtime-method forwarders and the opt-in AI control surface.
#[wasm_bindgen]
impl LegaiaRuntime {
    /// See [`pad_bindings_json`].
    pub fn pad_bindings_json(&self) -> String {
        pad_bindings_json()
    }

    /// See [`pad_buttons_json`].
    pub fn pad_buttons_json(&self) -> String {
        pad_buttons_json()
    }

    /// See [`pad_bindings_revision`].
    pub fn pad_bindings_revision(&self) -> u32 {
        pad_bindings_revision()
    }

    /// See [`pad_bindings_reset`].
    pub fn pad_bindings_reset(&self) {
        pad_bindings_reset();
    }

    /// Explicitly grant the AI a short-lived watcher lease.
    ///
    /// The lease lasts five seconds by default. It must be renewed by a
    /// visible/focused browser heartbeat; without renewal `ai_set_pad` drops
    /// the pad word to zero and discards all queued input.
    pub fn ai_arm(&mut self, now_ms: f64) {
        AI_CONTROLLER.with(|c| c.borrow_mut().arm(ai_now_ms(now_ms)));
    }

    /// Renew the watcher lease without changing the current plan.
    pub fn ai_heartbeat(&mut self, now_ms: f64) {
        AI_CONTROLLER.with(|c| c.borrow_mut().renew_watcher(ai_now_ms(now_ms)));
    }

    /// Revoke AI authority and discard every queued/held AI input.
    pub fn ai_disarm(&mut self) {
        AI_CONTROLLER.with(|c| c.borrow_mut().disarm());
        self.set_pad(0);
    }

    /// Human takeover is stronger than a temporary disarm: heartbeats alone
    /// cannot resume AI play. A fresh explicit [`Self::ai_arm`] is required.
    pub fn ai_manual_takeover(&mut self) {
        AI_CONTROLLER.with(|c| c.borrow_mut().manual_takeover());
        self.set_pad(0);
    }

    /// Replace the bounded controller queue with a validated JSON plan.
    ///
    /// Example: `{"steps":[{"buttons":["up"],"frames":30},{"buttons":[],"frames":1}]}`.
    pub fn ai_set_plan_json(&mut self, plan_json: &str) -> Result<(), JsValue> {
        let plan: ControllerPlan = serde_json::from_str(plan_json)
            .map_err(|e| JsValue::from_str(&format!("ai plan JSON: {e}")))?;
        AI_CONTROLLER
            .with(|c| c.borrow_mut().replace_plan(plan))
            .map_err(|e| JsValue::from_str(&format!("ai plan rejected: {e}")))
    }

    /// Queue one short collision-aware field-navigation burst toward a
    /// world-space target. The call never teleports and never holds input
    /// indefinitely: when the current queue is busy it simply reports busy;
    /// when idle it BFS-plans from the engine's live collision/floor grids and
    /// queues a short burst that will be re-planned after new telemetry.
    pub fn ai_navigate_to(&mut self, x: i32, z: i32, radius: u32, now_ms: f64) -> String {
        let now = ai_now_ms(now_ms);
        let authorized = AI_CONTROLLER.with(|c| c.borrow().authority().authorized(now));
        if !authorized {
            return serde_json::json!({"status":"unauthorized"}).to_string();
        }
        let idle = AI_CONTROLLER.with(|c| c.borrow().is_idle());
        if !idle {
            return serde_json::json!({"status":"busy"}).to_string();
        }

        let world = self
            .scene_host
            .host()
            .map(|h| &h.world)
            .unwrap_or(&self.world);
        let target = NavigationTarget { x, z, radius };
        match plan_field_step(world, target, NavigationConfig::default()) {
            Ok(NavigationResult::Reached) => {
                serde_json::json!({"status":"reached"}).to_string()
            }
            Ok(NavigationResult::NoPlayer) => {
                serde_json::json!({"status":"no_player"}).to_string()
            }
            Ok(NavigationResult::NoPath) => {
                serde_json::json!({"status":"no_path"}).to_string()
            }
            Ok(NavigationResult::Step(step)) => {
                let queued = AI_CONTROLLER.with(|c| c.borrow_mut().replace_plan(step.plan));
                match queued {
                    Ok(()) => serde_json::json!({
                        "status":"queued",
                        "next_x":step.next_x,
                        "next_z":step.next_z,
                        "remaining_cells":step.remaining_cells,
                    })
                    .to_string(),
                    Err(err) => serde_json::json!({
                        "status":"rejected",
                        "error":err.to_string(),
                    })
                    .to_string(),
                }
            }
            Err(err) => serde_json::json!({
                "status":"error",
                "error":err.to_string(),
            })
            .to_string(),
        }
    }

    /// Whether the AI currently holds a valid watcher lease and has not been
    /// displaced by manual input.
    pub fn ai_authorized(&self, now_ms: f64) -> bool {
        AI_CONTROLLER.with(|c| c.borrow().authority().authorized(ai_now_ms(now_ms)))
    }

    /// One browser-frame arbitration point.
    ///
    /// Any non-zero human pad word wins immediately and permanently pauses AI
    /// input until the user explicitly arms it again. With no human input the
    /// controller supplies the next bounded AI pad word, but only while the
    /// watcher lease is valid. The selected word is forwarded through the
    /// unchanged upstream [`LegaiaRuntime::set_pad`] entry point.
    pub fn ai_set_pad(&mut self, human_mask: u16, now_ms: f64) -> u16 {
        let mask = if human_mask != 0 {
            AI_CONTROLLER.with(|c| c.borrow_mut().manual_takeover());
            human_mask
        } else {
            AI_CONTROLLER.with(|c| c.borrow_mut().tick(ai_now_ms(now_ms)))
        };
        self.set_pad(mask);
        mask
    }

    /// Small status object for the browser overlay.
    pub fn ai_status_json(&self, now_ms: f64) -> String {
        let now = ai_now_ms(now_ms);
        AI_CONTROLLER.with(|c| {
            let c = c.borrow();
            let a = c.authority();
            serde_json::json!({
                "armed": a.armed,
                "manual_override": a.manual_override,
                "authorized": a.authorized(now),
                "watcher_deadline_ms": a.watcher_deadline_ms,
                "idle": c.is_idle(),
            })
            .to_string()
        })
    }

    /// Semantic game state for the external AI broker.
    pub fn ai_telemetry_json(&self) -> String {
        let world = self
            .scene_host
            .host()
            .map(|h| &h.world)
            .unwrap_or(&self.world);

        let player = world
            .player_actor_slot
            .and_then(|slot| world.actors.get(slot as usize).map(|actor| (slot, actor)))
            .map(|(slot, actor)| PlayerTelemetry {
                actor_slot: Some(slot),
                position: Some(Vec3i {
                    x: i32::from(actor.move_state.world_x),
                    y: i32::from(actor.move_state.world_y),
                    z: i32::from(actor.move_state.world_z),
                }),
                heading: Some(actor.move_state.render_26 & 0x0fff),
            })
            .unwrap_or_default();

        let party = (0..usize::from(world.party.party_count.min(3)))
            .filter_map(|member| {
                let roster_slot = world.party_roster_slot(member);
                let record = world.party.roster.members.get(roster_slot)?;
                let hms = record.hp_mp_sp();
                Some(PartyMemberTelemetry {
                    name: world.party_name(roster_slot).to_string(),
                    hp: u32::from(hms.hp_cur),
                    hp_max: u32::from(hms.hp_max),
                    mp: u32::from(hms.mp_cur),
                    mp_max: u32::from(hms.mp_max),
                    ap: u32::from(hms.sp_cur),
                    level: u32::from(record.level().max(1)),
                    status: Vec::new(),
                })
            })
            .collect::<Vec<_>>();

        let dialog_active = world.dialog.current.is_some()
            || world.dialog.inline.is_some()
            || world.dialog.three_actor_talk.is_some();
        let dialog = dialog_active.then_some(DialogTelemetry {
            active: true,
            can_advance: true,
            choice_count: 0,
        });

        let battle = (world.mode == SceneMode::Battle).then(|| {
            let party_count = usize::from(world.party.party_count.min(3));
            let enemies = world
                .actors
                .iter()
                .enumerate()
                .skip(party_count)
                .filter(|(_, actor)| actor.battle_monster_id.is_some() || actor.battle.max_hp > 0)
                .filter_map(|(slot, actor)| {
                    let slot = u8::try_from(slot).ok()?;
                    Some(EnemyTelemetry {
                        slot,
                        name: None,
                        hp: Some(u32::from(actor.battle.hp)),
                        hp_max: Some(u32::from(actor.battle.max_hp)),
                        alive: actor.battle.hp > 0,
                    })
                })
                .collect();
            let phase = if world.battle.victory.is_some() {
                "results"
            } else if world.battle.end.is_some() {
                "ending"
            } else {
                "active"
            };
            BattleTelemetry {
                phase: phase.to_string(),
                enemies,
                active_party_slot: None,
            }
        });

        let snapshot = TelemetrySnapshot {
            schema_version: legaia_ai_controller::TELEMETRY_SCHEMA_VERSION,
            frame: world.frame,
            mode: ai_game_mode(world.mode),
            scene: world.active_scene_label.clone(),
            player,
            party,
            battle,
            dialog,
            menu_open: world.mode == SceneMode::Menu,
            story: serde_json::json!({
                "money": world.party.money,
                "party_count": world.party.party_count,
            }),
        };

        serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exported table must be parseable JSON carrying the whole default
    /// layout - a page that gets `{}` falls back to no input at all, silently.
    #[test]
    fn the_exported_table_is_json_and_carries_every_default_binding() {
        let json = pad_bindings_json();
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON object");
        let obj = v.as_object().expect("object");
        let want = legaia_engine_core::input::Mapping::web_default().dom_code_bindings();
        assert_eq!(obj.len(), want.len());
        for (code, bit) in want {
            assert_eq!(obj.get(code).and_then(|b| b.as_u64()), Some(u64::from(bit)));
        }
    }

    #[test]
    fn the_button_table_names_all_sixteen_pad_bits() {
        let v: serde_json::Value = serde_json::from_str(&pad_buttons_json()).unwrap();
        let obj = v.as_object().unwrap();
        assert_eq!(obj.len(), 16);
        assert_eq!(obj.get("Cross").and_then(|b| b.as_u64()), Some(0x4000));
        assert_eq!(obj.get("Square").and_then(|b| b.as_u64()), Some(0x8000));
        assert_eq!(obj.get("Select").and_then(|b| b.as_u64()), Some(0x0001));
    }

    #[test]
    fn ai_runtime_starts_disarmed_and_requires_a_lease() {
        let mut rt = LegaiaRuntime::new();
        rt.ai_disarm();
        assert!(!rt.ai_authorized(100.0));
        rt.ai_arm(100.0);
        assert!(rt.ai_authorized(101.0));
        assert!(!rt.ai_authorized(5_101.0));
    }

    #[test]
    fn human_pad_immediately_takes_authority() {
        let mut rt = LegaiaRuntime::new();
        rt.ai_disarm();
        rt.ai_arm(10.0);
        rt.ai_set_plan_json(
            r#"{"steps":[{"buttons":["up"],"frames":30}]}"#,
        )
        .unwrap();
        assert_eq!(rt.ai_set_pad(0, 11.0), 0x0010);
        assert_eq!(rt.ai_set_pad(0x4000, 12.0), 0x4000);
        assert!(!rt.ai_authorized(13.0));
        rt.ai_heartbeat(14.0);
        assert!(!rt.ai_authorized(15.0));
    }

    #[test]
    fn telemetry_is_versioned_json_even_without_a_loaded_disc() {
        let rt = LegaiaRuntime::new();
        let v: serde_json::Value = serde_json::from_str(&rt.ai_telemetry_json()).unwrap();
        assert_eq!(v["schema_version"].as_u64(), Some(1));
        assert_eq!(v["mode"].as_str(), Some("title"));
        assert!(v["party"].is_array());
    }
}