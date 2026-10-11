use anyhow::{Context, Result};
use legaia_ai_controller::{Button, ControllerPlan, GameMode, TelemetrySnapshot};
use legaia_ai_guide::{CheckpointAction, GuidePlan, GuideRunner, GuideTarget};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Directive {
    Navigate {
        x: i32,
        z: i32,
        radius: u32,
    },
    Controller {
        plan: ControllerPlan,
    },
    NeedDecision {
        objective: String,
        walkthrough_context: String,
        reason: String,
    },
    Wait {
        reason: String,
    },
    Pause {
        reason: String,
    },
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrchestrationStep {
    pub checkpoint_index: usize,
    pub checkpoint_id: Option<String>,
    pub objective: Option<String>,
    pub advanced: usize,
    pub directive: Directive,
}

#[derive(Debug, Clone)]
pub struct Orchestrator {
    runner: GuideRunner,
}

impl Orchestrator {
    pub fn new(plan: GuidePlan) -> Result<Self> {
        Ok(Self {
            runner: GuideRunner::new(plan)?,
        })
    }

    pub fn from_yaml(text: &str) -> Result<Self> {
        Self::new(GuidePlan::from_yaml(text)?)
    }

    pub fn from_json(text: &str) -> Result<Self> {
        Self::new(GuidePlan::from_json(text)?)
    }

    pub fn checkpoint_index(&self) -> usize {
        self.runner.index()
    }

    pub fn is_finished(&self) -> bool {
        self.runner.is_finished()
    }

    pub fn manual_advance(&mut self) -> bool {
        self.runner.advance_manual()
    }

    pub fn step(&mut self, state: &TelemetrySnapshot) -> Result<OrchestrationStep> {
        let advanced = self.runner.advance_completed(state);
        let index = self.runner.index();

        let Some(checkpoint) = self.runner.current().cloned() else {
            return Ok(OrchestrationStep {
                checkpoint_index: index,
                checkpoint_id: None,
                objective: None,
                advanced,
                directive: Directive::Complete,
            });
        };

        let context = self
            .runner
            .current_context()
            .context("current checkpoint unexpectedly lacked guide context")?;
        let checkpoint_id = Some(checkpoint.id.clone());
        let objective = Some(checkpoint.objective.clone());

        let directive = if let Some(expected_scene) = checkpoint.scene.as_deref() {
            if !state.scene.is_empty() && state.scene != expected_scene {
                Directive::NeedDecision {
                    objective: checkpoint.objective.clone(),
                    walkthrough_context: context,
                    reason: format!(
                        "expected scene `{expected_scene}` but telemetry reports `{}`",
                        state.scene
                    ),
                }
            } else {
                self.directive_for_checkpoint(&checkpoint, state, context)
            }
        } else {
            self.directive_for_checkpoint(&checkpoint, state, context)
        };

        Ok(OrchestrationStep {
            checkpoint_index: index,
            checkpoint_id,
            objective,
            advanced,
            directive,
        })
    }

    fn directive_for_checkpoint(
        &self,
        checkpoint: &legaia_ai_guide::Checkpoint,
        state: &TelemetrySnapshot,
        context: String,
    ) -> Directive {
        match checkpoint.action {
            CheckpointAction::Navigate => {
                checkpoint
                    .target
                    .map(navigate)
                    .unwrap_or_else(|| Directive::NeedDecision {
                        objective: checkpoint.objective.clone(),
                        walkthrough_context: context,
                        reason: "navigate checkpoint has no coordinate target".into(),
                    })
            }

            CheckpointAction::Interact => {
                if let Some(target) = checkpoint.target {
                    if !target.reached(state) {
                        return navigate(target);
                    }
                }
                Directive::Controller {
                    plan: ControllerPlan::tap(Button::Cross),
                }
            }

            CheckpointAction::OpenMenu => Directive::Controller {
                plan: ControllerPlan::tap(Button::Start),
            },

            CheckpointAction::Wait => Directive::Wait {
                reason: "checkpoint explicitly waits for state to change".into(),
            },

            CheckpointAction::Observe => Directive::NeedDecision {
                objective: checkpoint.objective.clone(),
                walkthrough_context: context,
                reason: "checkpoint delegates the next safe high-level choice to the model".into(),
            },

            CheckpointAction::Battle => {
                if state.mode != GameMode::Battle {
                    if let Some(target) = checkpoint.target {
                        if !target.reached(state) {
                            return navigate(target);
                        }
                    }
                    return Directive::Wait {
                        reason: "battle checkpoint is waiting for battle mode".into(),
                    };
                }
                Directive::NeedDecision {
                    objective: checkpoint.objective.clone(),
                    walkthrough_context: context,
                    reason: "battle policy requires a high-level choice".into(),
                }
            }
        }
    }
}

fn navigate(target: GuideTarget) -> Directive {
    Directive::Navigate {
        x: target.x,
        z: target.z,
        radius: target.radius,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use legaia_ai_controller::{PlayerTelemetry, Vec3i};

    fn state(scene: &str, mode: GameMode, x: i32, z: i32) -> TelemetrySnapshot {
        let mut s = TelemetrySnapshot::empty();
        s.scene = scene.into();
        s.mode = mode;
        s.player = PlayerTelemetry {
            actor_slot: Some(0),
            position: Some(Vec3i { x, y: 0, z }),
            heading: Some(0),
        };
        s
    }

    #[test]
    fn navigate_checkpoint_becomes_coordinate_directive() {
        let mut o = Orchestrator::from_yaml(
            r#"
title: t
checkpoints:
  - id: walk
    objective: walk there
    scene: town01
    action: navigate
    target: { x: 1000, z: 2000, radius: 32 }
    completion: { kind: near_target }
"#,
        )
        .unwrap();
        let step = o.step(&state("town01", GameMode::Field, 0, 0)).unwrap();
        assert_eq!(
            step.directive,
            Directive::Navigate {
                x: 1000,
                z: 2000,
                radius: 32
            }
        );
    }

    #[test]
    fn interact_checkpoint_walks_first_then_taps_cross() {
        let mut o = Orchestrator::from_yaml(
            r#"
title: t
checkpoints:
  - id: talk
    objective: talk
    action: interact
    target: { x: 100, z: 100, radius: 20 }
    completion: { kind: dialog_open }
"#,
        )
        .unwrap();
        let far = o.step(&state("town01", GameMode::Field, 0, 0)).unwrap();
        assert!(matches!(far.directive, Directive::Navigate { .. }));
        let near = o.step(&state("town01", GameMode::Field, 100, 100)).unwrap();
        assert!(matches!(near.directive, Directive::Controller { .. }));
    }

    #[test]
    fn unexpected_scene_escalates_instead_of_guessing() {
        let mut o = Orchestrator::from_yaml(
            r#"
title: t
checkpoints:
  - id: walk
    objective: walk
    scene: town01
    action: navigate
    target: { x: 10, z: 10 }
"#,
        )
        .unwrap();
        let step = o.step(&state("town02", GameMode::Field, 0, 0)).unwrap();
        assert!(matches!(step.directive, Directive::NeedDecision { .. }));
    }

    #[test]
    fn completed_final_checkpoint_returns_complete() {
        let mut o = Orchestrator::from_yaml(
            r#"
title: t
checkpoints:
  - id: here
    objective: arrive
    action: navigate
    target: { x: 100, z: 100, radius: 20 }
    completion: { kind: near_target }
"#,
        )
        .unwrap();
        let step = o.step(&state("town01", GameMode::Field, 100, 100)).unwrap();
        assert_eq!(step.advanced, 1);
        assert_eq!(step.directive, Directive::Complete);
    }
}
