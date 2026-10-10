use anyhow::{Context, Result, bail};
use legaia_ai_controller::{GameMode, HighLevelAction, TelemetrySnapshot};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GuidePlan {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
}

impl GuidePlan {
    pub fn from_yaml(text: &str) -> Result<Self> {
        let plan: Self = serde_yaml::from_str(text).context("invalid guide YAML")?;
        plan.validate()?;
        Ok(plan)
    }

    pub fn from_json(text: &str) -> Result<Self> {
        let plan: Self = serde_json::from_str(text).context("invalid guide JSON")?;
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<()> {
        if self.title.trim().is_empty() {
            bail!("guide title cannot be empty");
        }
        if self.checkpoints.is_empty() {
            bail!("guide must contain at least one checkpoint");
        }

        let mut ids = HashSet::new();
        for (index, checkpoint) in self.checkpoints.iter().enumerate() {
            checkpoint
                .validate()
                .with_context(|| format!("checkpoint {index}"))?;
            if !ids.insert(checkpoint.id.as_str()) {
                bail!("duplicate checkpoint id `{}`", checkpoint.id);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Checkpoint {
    pub id: String,
    pub objective: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<GuideTarget>,
    #[serde(default)]
    pub action: CheckpointAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<CompletionRule>,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub optional: bool,
}

impl Checkpoint {
    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            bail!("checkpoint id cannot be empty");
        }
        if self.objective.trim().is_empty() {
            bail!("checkpoint objective cannot be empty");
        }
        if let Some(target) = self.target {
            target.validate()?;
        }
        Ok(())
    }

    pub fn high_level_action(&self) -> HighLevelAction {
        match self.action {
            CheckpointAction::Navigate => HighLevelAction::Navigate,
            CheckpointAction::Interact => HighLevelAction::Interact,
            CheckpointAction::Battle => HighLevelAction::Battle,
            CheckpointAction::OpenMenu => HighLevelAction::OpenMenu,
            CheckpointAction::Wait => HighLevelAction::Wait,
            CheckpointAction::Observe => HighLevelAction::Observe,
        }
    }

    pub fn is_complete(&self, state: &TelemetrySnapshot) -> bool {
        if let Some(rule) = &self.completion {
            return rule.matches(state, self.target.as_ref());
        }

        // A coordinate checkpoint has a useful safe default. Everything else
        // requires an explicit rule or manual advancement, avoiding accidental
        // story skips caused by guessing what a prose objective meant.
        self.target
            .as_ref()
            .is_some_and(|target| target.reached(state))
    }

    pub fn concise_context(&self) -> String {
        let mut out = format!("checkpoint={}\nobjective={}", self.id, self.objective);
        if let Some(scene) = &self.scene {
            out.push_str(&format!("\nexpected_scene={scene}"));
        }
        if let Some(target) = self.target {
            out.push_str(&format!(
                "\ntarget=x:{} z:{} radius:{}",
                target.x, target.z, target.radius
            ));
        }
        out.push_str(&format!("\naction={:?}", self.action));
        for note in &self.notes {
            out.push_str("\nnote=");
            out.push_str(note);
        }
        out
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointAction {
    Navigate,
    Interact,
    Battle,
    OpenMenu,
    Wait,
    Observe,
}

impl Default for CheckpointAction {
    fn default() -> Self {
        Self::Observe
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuideTarget {
    pub x: i32,
    pub z: i32,
    #[serde(default = "default_radius")]
    pub radius: u32,
}

const fn default_radius() -> u32 {
    24
}

impl GuideTarget {
    pub fn validate(self) -> Result<()> {
        if self.radius == 0 {
            bail!("target radius must be greater than zero");
        }
        Ok(())
    }

    pub fn reached(self, state: &TelemetrySnapshot) -> bool {
        let Some(pos) = state.player.position else {
            return false;
        };
        let dx = i64::from(pos.x) - i64::from(self.x);
        let dz = i64::from(pos.z) - i64::from(self.z);
        let radius = i64::from(self.radius);
        dx.saturating_mul(dx) + dz.saturating_mul(dz) <= radius.saturating_mul(radius)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompletionRule {
    Manual,
    NearTarget,
    SceneIs { scene: String },
    SceneChangedFrom { scene: String },
    ModeIs { mode: GameMode },
    DialogOpen,
    DialogClosed,
    FrameAtLeast { frame: u64 },
    Any { rules: Vec<CompletionRule> },
    All { rules: Vec<CompletionRule> },
}

impl CompletionRule {
    pub fn matches(&self, state: &TelemetrySnapshot, target: Option<&GuideTarget>) -> bool {
        match self {
            Self::Manual => false,
            Self::NearTarget => target.is_some_and(|target| target.reached(state)),
            Self::SceneIs { scene } => state.scene == *scene,
            Self::SceneChangedFrom { scene } => !state.scene.is_empty() && state.scene != *scene,
            Self::ModeIs { mode } => state.mode == *mode,
            Self::DialogOpen => state.dialog.as_ref().is_some_and(|d| d.active),
            Self::DialogClosed => state.dialog.as_ref().is_none_or(|d| !d.active),
            Self::FrameAtLeast { frame } => state.frame >= *frame,
            Self::Any { rules } => rules.iter().any(|r| r.matches(state, target)),
            Self::All { rules } => !rules.is_empty() && rules.iter().all(|r| r.matches(state, target)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GuideRunner {
    plan: GuidePlan,
    index: usize,
}

impl GuideRunner {
    pub fn new(plan: GuidePlan) -> Result<Self> {
        plan.validate()?;
        Ok(Self { plan, index: 0 })
    }

    pub fn plan(&self) -> &GuidePlan {
        &self.plan
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn is_finished(&self) -> bool {
        self.index >= self.plan.checkpoints.len()
    }

    pub fn current(&self) -> Option<&Checkpoint> {
        self.plan.checkpoints.get(self.index)
    }

    pub fn next(&self) -> Option<&Checkpoint> {
        self.plan.checkpoints.get(self.index.saturating_add(1))
    }

    pub fn advance_manual(&mut self) -> bool {
        if self.is_finished() {
            return false;
        }
        self.index += 1;
        true
    }

    /// Advance through every checkpoint whose explicit completion condition is
    /// already true. A cap equal to plan length makes malformed/self-referential
    /// input harmless rather than an accidental infinite loop.
    pub fn advance_completed(&mut self, state: &TelemetrySnapshot) -> usize {
        let start = self.index;
        let max = self.plan.checkpoints.len();
        for _ in 0..max {
            let Some(current) = self.current() else {
                break;
            };
            if !current.is_complete(state) {
                break;
            }
            self.index += 1;
        }
        self.index.saturating_sub(start)
    }

    /// Compact context intended for the model, not the raw copyrighted guide
    /// text. The user-supplied source can remain local while checkpoints carry
    /// only the facts needed for the current decision.
    pub fn current_context(&self) -> Option<String> {
        let current = self.current()?;
        let mut out = current.concise_context();
        if let Some(next) = self.next() {
            out.push_str("\nnext_checkpoint=");
            out.push_str(&next.id);
            out.push_str("\nnext_objective=");
            out.push_str(&next.objective);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use legaia_ai_controller::{GameMode, PlayerTelemetry, Vec3i};

    fn state(x: i32, z: i32) -> TelemetrySnapshot {
        let mut s = TelemetrySnapshot::empty();
        s.mode = GameMode::Field;
        s.scene = "town01".into();
        s.player = PlayerTelemetry {
            actor_slot: Some(0),
            position: Some(Vec3i { x, y: 0, z }),
            heading: Some(0),
        };
        s
    }

    #[test]
    fn parses_yaml_and_defaults_target_radius() {
        let plan = GuidePlan::from_yaml(
            r#"
title: Opening
checkpoints:
  - id: rim-elm-tree
    objective: Reach the Genesis Tree
    scene: town01
    action: navigate
    target: { x: 100, z: 200 }
    completion: { kind: near_target }
"#,
        )
        .unwrap();
        assert_eq!(plan.checkpoints[0].target.unwrap().radius, 24);
    }

    #[test]
    fn target_completion_uses_radius() {
        let cp = Checkpoint {
            id: "a".into(),
            objective: "walk".into(),
            scene: None,
            target: Some(GuideTarget { x: 100, z: 100, radius: 10 }),
            action: CheckpointAction::Navigate,
            completion: None,
            notes: vec![],
            optional: false,
        };
        assert!(cp.is_complete(&state(106, 106)));
        assert!(!cp.is_complete(&state(120, 100)));
    }

    #[test]
    fn runner_advances_completed_checkpoints_but_stops_on_manual() {
        let plan = GuidePlan {
            title: "x".into(),
            source: None,
            checkpoints: vec![
                Checkpoint {
                    id: "scene".into(),
                    objective: "be here".into(),
                    scene: None,
                    target: None,
                    action: CheckpointAction::Observe,
                    completion: Some(CompletionRule::SceneIs { scene: "town01".into() }),
                    notes: vec![],
                    optional: false,
                },
                Checkpoint {
                    id: "manual".into(),
                    objective: "wait".into(),
                    scene: None,
                    target: None,
                    action: CheckpointAction::Observe,
                    completion: Some(CompletionRule::Manual),
                    notes: vec![],
                    optional: false,
                },
            ],
        };
        let mut runner = GuideRunner::new(plan).unwrap();
        assert_eq!(runner.advance_completed(&state(0, 0)), 1);
        assert_eq!(runner.current().unwrap().id, "manual");
    }

    #[test]
    fn duplicate_checkpoint_ids_are_rejected() {
        let yaml = r#"
title: Bad
checkpoints:
  - { id: same, objective: one }
  - { id: same, objective: two }
"#;
        assert!(GuidePlan::from_yaml(yaml).is_err());
    }
}
