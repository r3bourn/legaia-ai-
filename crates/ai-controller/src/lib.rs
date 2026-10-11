use std::collections::VecDeque;

use anyhow::{Result, bail};
use legaia_engine_vm::pad::PadButton;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TELEMETRY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GameMode {
    Boot,
    Title,
    Field,
    WorldMap,
    Battle,
    Menu,
    Cutscene,
    Minigame,
    Unknown,
}

impl Default for GameMode {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Vec3i {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PlayerTelemetry {
    pub actor_slot: Option<u8>,
    pub position: Option<Vec3i>,
    pub heading: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PartyMemberTelemetry {
    pub name: String,
    pub hp: u32,
    pub hp_max: u32,
    pub mp: u32,
    pub mp_max: u32,
    pub ap: u32,
    pub level: u32,
    #[serde(default)]
    pub status: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct EnemyTelemetry {
    pub slot: u8,
    pub name: Option<String>,
    pub hp: Option<u32>,
    pub hp_max: Option<u32>,
    pub alive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BattleTelemetry {
    pub phase: String,
    #[serde(default)]
    pub enemies: Vec<EnemyTelemetry>,
    pub active_party_slot: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DialogTelemetry {
    pub active: bool,
    pub can_advance: bool,
    pub choice_count: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TelemetrySnapshot {
    pub schema_version: u32,
    pub frame: u64,
    pub mode: GameMode,
    pub scene: String,
    pub player: PlayerTelemetry,
    #[serde(default)]
    pub party: Vec<PartyMemberTelemetry>,
    pub battle: Option<BattleTelemetry>,
    pub dialog: Option<DialogTelemetry>,
    pub menu_open: bool,
    #[serde(default)]
    pub story: Value,
}

impl TelemetrySnapshot {
    pub fn empty() -> Self {
        Self {
            schema_version: TELEMETRY_SCHEMA_VERSION,
            frame: 0,
            mode: GameMode::Unknown,
            scene: String::new(),
            player: PlayerTelemetry::default(),
            party: Vec::new(),
            battle: None,
            dialog: None,
            menu_open: false,
            story: Value::Null,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HighLevelAction {
    Navigate,
    Interact,
    OpenMenu,
    Battle,
    Wait,
    Observe,
    Pause,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Select,
    L3,
    R3,
    Start,
    Up,
    Right,
    Down,
    Left,
    L2,
    R2,
    L1,
    R1,
    Triangle,
    Circle,
    Cross,
    Square,
}

impl Button {
    pub fn pad(self) -> PadButton {
        match self {
            Self::Select => PadButton::Select,
            Self::L3 => PadButton::L3,
            Self::R3 => PadButton::R3,
            Self::Start => PadButton::Start,
            Self::Up => PadButton::Up,
            Self::Right => PadButton::Right,
            Self::Down => PadButton::Down,
            Self::Left => PadButton::Left,
            Self::L2 => PadButton::L2,
            Self::R2 => PadButton::R2,
            Self::L1 => PadButton::L1,
            Self::R1 => PadButton::R1,
            Self::Triangle => PadButton::Triangle,
            Self::Circle => PadButton::Circle,
            Self::Cross => PadButton::Cross,
            Self::Square => PadButton::Square,
        }
    }

    pub fn mask(self) -> u16 {
        self.pad().mask()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PadStep {
    #[serde(default)]
    pub buttons: Vec<Button>,
    pub frames: u16,
}

impl PadStep {
    pub fn held(buttons: impl IntoIterator<Item = Button>, frames: u16) -> Self {
        Self {
            buttons: buttons.into_iter().collect(),
            frames,
        }
    }

    pub fn release(frames: u16) -> Self {
        Self {
            buttons: Vec::new(),
            frames,
        }
    }

    pub fn mask(&self) -> u16 {
        self.buttons.iter().fold(0u16, |mask, b| mask | b.mask())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ControllerPlan {
    #[serde(default)]
    pub steps: Vec<PadStep>,
}

impl ControllerPlan {
    pub fn tap(button: Button) -> Self {
        Self {
            steps: vec![PadStep::held([button], 1), PadStep::release(1)],
        }
    }

    pub fn validate(&self, safety: ControllerSafety) -> Result<()> {
        if self.steps.len() > safety.max_plan_steps {
            bail!(
                "controller plan contains {} steps; maximum is {}",
                self.steps.len(),
                safety.max_plan_steps
            );
        }
        for (index, step) in self.steps.iter().enumerate() {
            if step.frames == 0 {
                bail!("controller step {index} has zero frames");
            }
            if step.frames > safety.max_step_frames {
                bail!(
                    "controller step {index} holds for {} frames; maximum is {}",
                    step.frames,
                    safety.max_step_frames
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControllerSafety {
    pub watcher_ttl_ms: u64,
    pub max_step_frames: u16,
    pub max_plan_steps: usize,
}

impl Default for ControllerSafety {
    fn default() -> Self {
        Self {
            watcher_ttl_ms: 5_000,
            max_step_frames: 180,
            max_plan_steps: 64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AuthorityState {
    pub armed: bool,
    pub manual_override: bool,
    pub watcher_deadline_ms: Option<u64>,
}

impl AuthorityState {
    pub fn arm(&mut self, now_ms: u64, safety: ControllerSafety) {
        self.armed = true;
        self.manual_override = false;
        self.renew_watcher(now_ms, safety);
    }

    pub fn disarm(&mut self) {
        self.armed = false;
        self.watcher_deadline_ms = None;
    }

    pub fn renew_watcher(&mut self, now_ms: u64, safety: ControllerSafety) {
        self.watcher_deadline_ms = Some(now_ms.saturating_add(safety.watcher_ttl_ms));
    }

    pub fn manual_takeover(&mut self) {
        self.manual_override = true;
        self.armed = false;
        self.watcher_deadline_ms = None;
    }

    pub fn authorized(&self, now_ms: u64) -> bool {
        self.armed
            && !self.manual_override
            && self
                .watcher_deadline_ms
                .is_some_and(|deadline| now_ms <= deadline)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveStep {
    mask: u16,
    frames_remaining: u16,
}

#[derive(Debug)]
pub struct AiController {
    safety: ControllerSafety,
    authority: AuthorityState,
    queue: VecDeque<PadStep>,
    active: Option<ActiveStep>,
}

impl AiController {
    pub fn new(safety: ControllerSafety) -> Self {
        Self {
            safety,
            authority: AuthorityState::default(),
            queue: VecDeque::new(),
            active: None,
        }
    }

    pub fn authority(&self) -> AuthorityState {
        self.authority
    }

    pub fn arm(&mut self, now_ms: u64) {
        self.clear_plan();
        self.authority.arm(now_ms, self.safety);
    }

    pub fn renew_watcher(&mut self, now_ms: u64) {
        if !self.authority.manual_override {
            self.authority.renew_watcher(now_ms, self.safety);
        }
    }

    pub fn disarm(&mut self) {
        self.authority.disarm();
        self.clear_plan();
    }

    pub fn manual_takeover(&mut self) {
        self.authority.manual_takeover();
        self.clear_plan();
    }

    pub fn replace_plan(&mut self, plan: ControllerPlan) -> Result<()> {
        plan.validate(self.safety)?;
        self.clear_plan();
        self.queue.extend(plan.steps);
        Ok(())
    }

    pub fn append_plan(&mut self, plan: ControllerPlan) -> Result<()> {
        plan.validate(self.safety)?;
        if self.queue.len().saturating_add(plan.steps.len()) > self.safety.max_plan_steps {
            bail!("controller queue would exceed maximum plan length");
        }
        self.queue.extend(plan.steps);
        Ok(())
    }

    pub fn clear_plan(&mut self) {
        self.queue.clear();
        self.active = None;
    }

    pub fn is_idle(&self) -> bool {
        self.active.is_none() && self.queue.is_empty()
    }

    /// Return the raw PSX pad word for this simulation tick.
    ///
    /// This is deliberately fail-closed. If the watcher lease expires,
    /// authority is absent, or manual takeover has happened, all queued input
    /// is discarded and the returned pad word is zero.
    pub fn tick(&mut self, now_ms: u64) -> u16 {
        if !self.authority.authorized(now_ms) {
            self.clear_plan();
            return 0;
        }

        if self.active.is_none() {
            if let Some(step) = self.queue.pop_front() {
                self.active = Some(ActiveStep {
                    mask: step.mask(),
                    frames_remaining: step.frames,
                });
            }
        }

        let Some(mut active) = self.active else {
            return 0;
        };

        let mask = active.mask;
        active.frames_remaining -= 1;
        self.active = (active.frames_remaining > 0).then_some(active);
        mask
    }
}

impl Default for AiController {
    fn default() -> Self {
        Self::new(ControllerSafety::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_masks_match_psx_pad_layout() {
        assert_eq!(Button::Up.mask(), 0x0010);
        assert_eq!(Button::Right.mask(), 0x0020);
        assert_eq!(Button::Cross.mask(), 0x4000);
        assert_eq!(Button::Square.mask(), 0x8000);
    }

    #[test]
    fn tap_includes_release_frame_for_edge_detection() {
        let mut c = AiController::default();
        c.arm(100);
        c.replace_plan(ControllerPlan::tap(Button::Cross)).unwrap();
        assert_eq!(c.tick(100), Button::Cross.mask());
        assert_eq!(c.tick(101), 0);
        assert_eq!(c.tick(102), 0);
        assert!(c.is_idle());
    }

    #[test]
    fn watcher_expiry_releases_and_discards_queue() {
        let safety = ControllerSafety {
            watcher_ttl_ms: 10,
            ..ControllerSafety::default()
        };
        let mut c = AiController::new(safety);
        c.arm(1_000);
        c.replace_plan(ControllerPlan {
            steps: vec![PadStep::held([Button::Up], 30)],
        })
        .unwrap();
        assert_eq!(c.tick(1_005), Button::Up.mask());
        assert_eq!(c.tick(1_011), 0);
        assert!(c.is_idle());
    }

    #[test]
    fn manual_takeover_is_immediate_and_requires_rearm() {
        let mut c = AiController::default();
        c.arm(0);
        c.replace_plan(ControllerPlan {
            steps: vec![PadStep::held([Button::Left], 30)],
        })
        .unwrap();
        assert_eq!(c.tick(1), Button::Left.mask());
        c.manual_takeover();
        assert_eq!(c.tick(2), 0);
        c.renew_watcher(3);
        assert_eq!(c.tick(3), 0);
        c.arm(4);
        assert_eq!(c.tick(4), 0);
    }

    #[test]
    fn rejects_unbounded_steps() {
        let safety = ControllerSafety::default();
        let plan = ControllerPlan {
            steps: vec![PadStep::held([Button::Up], safety.max_step_frames + 1)],
        };
        assert!(plan.validate(safety).is_err());
    }
}
