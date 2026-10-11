//! Spectator mode: the play page renders a game an AI is playing elsewhere.
//!
//! The AI player (the full-game ladder's pad player, run on a homelab) drives
//! a `BootSession` imperatively and records every input-side step it applies:
//! each `World::set_pad` write, each `BootSession::tick`, and the two FMV
//! steps a headless driver takes instead of playing a movie. Replaying that
//! stream into a session seeded the same way reproduces the run exactly - the
//! ladder's `LEGAIA_FGL_REPLAY_CHECK` pins this natively.
//!
//! The page's normal frame path is a different host: it drains the per-tick
//! queues itself, owns the pause menu and stages the field CD-XA lane, steps
//! the naming prompt through its own overlay and holds the session under a
//! movie. Feeding the stream through `tick_frame` would therefore diverge.
//! Spectator mode instead switches the page's session to the headless host's
//! semantics and applies the stream's events directly, so the page only
//! renders a session that evolves exactly as the AI's did.
//!
//! Stream text (one event per line, run-length ticks):
//! `p <hex>` pad write, `t <n>` n ticks, `c` cutscene finished, `h` FMV
//! hand-off applied.

use std::cell::RefCell;
use std::collections::VecDeque;

use legaia_engine_core::scene::{FmvHandoffOutcome, SceneTickEvent};
use legaia_engine_session::boot::FieldLiveOpts;
use wasm_bindgen::prelude::*;

use crate::runtime::LegaiaRuntime;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Ev {
    Pad(u16),
    Tick,
    FinishCutscene,
    FmvHandoff,
}

#[derive(Default)]
struct Spectate {
    active: bool,
    queue: VecDeque<Ev>,
    ticks: u64,
}

thread_local! {
    static SPECTATE: RefCell<Spectate> = RefCell::new(Spectate::default());
}

/// Parse stream text into events. Unknown or malformed lines are errors: a
/// silently skipped event desynchronises everything after it.
fn parse_stream(text: &str) -> Result<Vec<Ev>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (op, arg) = line.split_once(' ').unwrap_or((line, ""));
        match op {
            "p" => out.push(Ev::Pad(
                u16::from_str_radix(arg, 16).map_err(|e| format!("line {}: {e}", n + 1))?,
            )),
            "t" => {
                let k: u32 = arg.parse().map_err(|e| format!("line {}: {e}", n + 1))?;
                out.extend(std::iter::repeat_n(Ev::Tick, k as usize));
            }
            "c" => out.push(Ev::FinishCutscene),
            "h" => out.push(Ev::FmvHandoff),
            _ => return Err(format!("line {}: unknown op {op:?}", n + 1)),
        }
    }
    Ok(out)
}

/// The live-loop options the AI's pad tier runs with (the ladder's
/// `live_opts(true)`): player-driven battles, random encounters armed.
fn spectator_opts() -> FieldLiveOpts {
    FieldLiveOpts {
        live_loop: true,
        player_battle: true,
        battle_bgm: None,
    }
}

#[wasm_bindgen]
impl LegaiaRuntime {
    /// Begin spectating: switch the session to headless host semantics and
    /// start a New Game the way the AI's session did. Returns the scene
    /// entered; the page rebuilds its render state for it.
    pub fn spectate_begin(&mut self) -> Result<String, JsValue> {
        let session = self
            .scene_host
            .session_mut()
            .ok_or_else(|| JsValue::from_str("spectate_begin: load a disc first"))?;
        session.set_host_drains_queues(false);
        session.set_host_owns_pause_menu(false);
        session.set_host_stages_field_xa(false);
        session.host.world.toggles.use_vm_dialogue = true;
        let scene = session
            .start_new_game(&spectator_opts())
            .ok_or_else(|| JsValue::from_str("spectate_begin: no opening scene would enter"))?;
        SPECTATE.with(|s| {
            *s.borrow_mut() = Spectate {
                active: true,
                ..Spectate::default()
            }
        });
        Ok(scene.to_string())
    }

    /// Append stream text to the pending events. Returns the queue length.
    pub fn spectate_feed(&mut self, text: &str) -> Result<u32, JsValue> {
        let evs =
            parse_stream(text).map_err(|e| JsValue::from_str(&format!("spectate_feed: {e}")))?;
        Ok(SPECTATE.with(|s| {
            let mut s = s.borrow_mut();
            s.queue.extend(evs);
            s.queue.len() as u32
        }))
    }

    /// Apply queued events until `max_ticks` ticks have run, the queue runs
    /// dry, or a scene is entered (the page must rebuild before the next
    /// frame renders). Returns `{"entered": <scene or "">, "ticks": n,
    /// "pending": queued events left, "total_ticks": ticks since begin}`.
    pub fn spectate_step(&mut self, max_ticks: u32) -> String {
        let mut entered = String::new();
        let mut ran = 0u32;
        let active = SPECTATE.with(|s| s.borrow().active);
        if active && let Some(session) = self.scene_host.session_mut() {
            while ran < max_ticks && entered.is_empty() {
                let Some(ev) = SPECTATE.with(|s| s.borrow_mut().queue.pop_front()) else {
                    break;
                };
                match ev {
                    Ev::Pad(m) => session.host.world.set_pad(m),
                    Ev::Tick => {
                        ran += 1;
                        if let Ok(SceneTickEvent::SceneEntered { name }) = session.tick() {
                            entered = name;
                        }
                    }
                    Ev::FinishCutscene => session.host.world.finish_cutscene(),
                    Ev::FmvHandoff => {
                        if let Some(FmvHandoffOutcome::Entered { scene, .. }) =
                            session.apply_pending_fmv_handoff()
                        {
                            entered = scene.to_string();
                        }
                    }
                }
            }
        }
        let (pending, total) = SPECTATE.with(|s| {
            let mut s = s.borrow_mut();
            s.ticks += u64::from(ran);
            (s.queue.len(), s.ticks)
        });
        serde_json::json!({
            "entered": entered,
            "ticks": ran,
            "pending": pending,
            "total_ticks": total,
        })
        .to_string()
    }

    /// The state a replay must reproduce, in the ladder's
    /// `replay_fingerprint` format, so a host can be checked against the
    /// AI's own session.
    pub fn spectate_fingerprint(&self) -> String {
        let Some(session) = self.scene_host.session() else {
            return String::new();
        };
        let w = &session.host.world;
        let scene = session
            .host
            .scene
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let slot = w.player_actor_slot.unwrap_or(0) as usize;
        let ms = &w.actors[slot].move_state;
        let mut flags = Vec::new();
        for (i, b) in w.flags.system_flags.iter().enumerate() {
            for k in 0..8 {
                if b & (0x80 >> k) != 0 {
                    flags.push((i * 8 + k) as u16);
                }
            }
        }
        format!(
            "scene={} xz={:?} frames={} rng={:#010x} mode={:?} flags={}:{:x}",
            scene,
            (ms.world_x, ms.world_z),
            session.frames,
            w.rng_state,
            w.mode,
            flags.len(),
            flags.iter().fold(0u64, |h, f| h
                .wrapping_mul(1_000_003)
                .wrapping_add(u64::from(*f))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_run_length_stream() {
        let evs = parse_stream("p 4000\nt 3\nc\nh\np 0\n").unwrap();
        assert_eq!(
            evs,
            vec![
                Ev::Pad(0x4000),
                Ev::Tick,
                Ev::Tick,
                Ev::Tick,
                Ev::FinishCutscene,
                Ev::FmvHandoff,
                Ev::Pad(0)
            ]
        );
    }

    #[test]
    fn rejects_unknown_ops() {
        assert!(parse_stream("x 1\n").is_err());
        assert!(parse_stream("p zz\n").is_err());
    }
}
