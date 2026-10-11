use crate::runtime::LegaiaRuntime;
use legaia_ai_battle::{BattleDirective, RoutineBattlePolicy, routine_battle_step};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
impl LegaiaRuntime {
    /// Advance one semantic step of the conservative routine-battle policy.
    ///
    /// The browser calls this only when the shared AI controller is idle. A
    /// controller directive is serialized back through `ai_set_plan_json`, so
    /// the exact same bounded-plan validation and watcher authority boundary
    /// applies to battle input as field navigation.
    pub fn ai_routine_battle_step(&mut self, now_ms: f64) -> String {
        if !self.ai_authorized(now_ms) {
            return serde_json::json!({"status":"unauthorized"}).to_string();
        }

        let world = self
            .scene_host
            .host()
            .map(|h| &h.world)
            .unwrap_or(&self.world);
        let directive = routine_battle_step(world, RoutineBattlePolicy::default());

        match directive {
            BattleDirective::Controller { plan, reason } => {
                let plan_json = match serde_json::to_string(&plan) {
                    Ok(json) => json,
                    Err(err) => {
                        return serde_json::json!({
                            "status":"error",
                            "error":format!("serialize battle plan: {err}"),
                        })
                        .to_string();
                    }
                };
                match self.ai_set_plan_json(&plan_json) {
                    Ok(()) => serde_json::json!({
                        "status":"queued",
                        "reason":reason,
                    })
                    .to_string(),
                    Err(err) => serde_json::json!({
                        "status":"rejected",
                        "error":format!("{err:?}"),
                    })
                    .to_string(),
                }
            }
            BattleDirective::Wait { reason } => serde_json::json!({
                "status":"wait",
                "reason":reason,
            })
            .to_string(),
            BattleDirective::NeedDecision { reason } => serde_json::json!({
                "status":"need_decision",
                "reason":reason,
            })
            .to_string(),
            BattleDirective::NotInBattle => {
                serde_json::json!({"status":"not_in_battle"}).to_string()
            }
        }
    }
}
