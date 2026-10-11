use std::collections::VecDeque;

use anyhow::{Result, bail};
use legaia_ai_controller::{Button, ControllerPlan, PadStep};
use legaia_engine_core::world::World;
use serde::{Deserialize, Serialize};

const SUBCELL_SIZE: i32 = 64;
const SUBCELL_CENTER: i32 = SUBCELL_SIZE / 2;
const SUBCELL_STRIDE: i32 = 256;
const SUBCELL_COUNT: usize = (SUBCELL_STRIDE as usize) * (SUBCELL_STRIDE as usize);
const DIR_RING: [u16; 8] = [
    0x1000, 0x3000, 0x2000, 0x6000, 0x4000, 0xC000, 0x8000, 0x9000,
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct NavigationTarget {
    pub x: i32,
    pub z: i32,
    pub radius: u32,
}

impl NavigationTarget {
    pub fn validate(self) -> Result<()> {
        if self.radius == 0 {
            bail!("navigation target radius must be greater than zero");
        }
        Ok(())
    }

    pub fn reached(self, x: i32, z: i32) -> bool {
        let dx = i64::from(x) - i64::from(self.x);
        let dz = i64::from(z) - i64::from(self.z);
        let r = i64::from(self.radius);
        dx.saturating_mul(dx) + dz.saturating_mul(dz) <= r.saturating_mul(r)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NavigationStep {
    pub plan: ControllerPlan,
    pub next_x: i32,
    pub next_z: i32,
    pub remaining_cells: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationConfig {
    /// Number of simulation frames to hold a direction before the caller
    /// should observe telemetry and re-plan. Keeping this short makes camera
    /// motion, NPC collision and door transitions self-correcting.
    pub burst_frames: u16,
    /// Hard cap on BFS nodes. The whole field lattice is 65,536 subcells, so
    /// the default can still search every reachable point while preventing an
    /// accidental unbounded allocator/loop if the geometry changes later.
    pub max_nodes: usize,
}

impl Default for NavigationConfig {
    fn default() -> Self {
        Self {
            burst_frames: 8,
            max_nodes: SUBCELL_COUNT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationResult {
    Reached,
    Step(NavigationStep),
    NoPlayer,
    NoPath,
}

/// Plan one short field-navigation burst from the engine's semantic world
/// state. The planner never teleports and never mutates `World`; it only asks
/// the same public floor/wall samplers the field engine uses and returns a
/// bounded PSX controller plan.
pub fn plan_field_step(
    world: &World,
    target: NavigationTarget,
    config: NavigationConfig,
) -> Result<NavigationResult> {
    target.validate()?;
    if config.burst_frames == 0 {
        bail!("navigation burst_frames must be greater than zero");
    }

    let Some(slot) = world.player_actor_slot else {
        return Ok(NavigationResult::NoPlayer);
    };
    let Some(actor) = world.actors.get(slot as usize) else {
        return Ok(NavigationResult::NoPlayer);
    };
    let start_world = (
        i32::from(actor.move_state.world_x),
        i32::from(actor.move_state.world_z),
    );
    if target.reached(start_world.0, start_world.1) {
        return Ok(NavigationResult::Reached);
    }

    let start = to_subcell(start_world.0, start_world.1);
    if !in_bounds(start.0, start.1) {
        return Ok(NavigationResult::NoPath);
    }

    let Some((goal_idx, parents, visited)) = bfs_to_target(world, start, target, config.max_nodes)
    else {
        return Ok(NavigationResult::NoPath);
    };

    let start_idx = index(start.0, start.1);
    if goal_idx == start_idx {
        return Ok(NavigationResult::Reached);
    }

    // Walk parent links from the goal back until the next node is directly
    // after start. BFS is four-connected, so the resulting delta is always a
    // single cardinal world direction and cannot cut a diagonal wall corner.
    let mut cursor = goal_idx;
    let mut remaining = 0usize;
    loop {
        let parent = parents[cursor];
        if parent == usize::MAX {
            return Ok(NavigationResult::NoPath);
        }
        remaining += 1;
        if parent == start_idx {
            break;
        }
        cursor = parent;
    }

    let next = coords(cursor);
    let desired_world_dir = world_direction(start, next)
        .ok_or_else(|| anyhow::anyhow!("navigation path produced a non-cardinal first step"))?;
    let raw_screen_dir = raw_direction_for_world(desired_world_dir, world.locomotion.camera_azimuth);
    let buttons = buttons_for_direction(raw_screen_dir);
    if buttons.is_empty() {
        return Ok(NavigationResult::NoPath);
    }

    let (next_x, next_z) = subcell_center(next.0, next.1);
    let plan = ControllerPlan {
        steps: vec![
            PadStep::held(buttons, config.burst_frames),
            PadStep::release(1),
        ],
    };

    Ok(NavigationResult::Step(NavigationStep {
        plan,
        next_x,
        next_z,
        remaining_cells: remaining.min(visited),
    }))
}

fn bfs_to_target(
    world: &World,
    start: (i32, i32),
    target: NavigationTarget,
    max_nodes: usize,
) -> Option<(usize, Vec<usize>, usize)> {
    let start_idx = index(start.0, start.1);
    let mut parents = vec![usize::MAX; SUBCELL_COUNT];
    let mut seen = vec![false; SUBCELL_COUNT];
    let mut queue = VecDeque::new();

    seen[start_idx] = true;
    parents[start_idx] = start_idx;
    queue.push_back(start);
    let mut visited = 0usize;

    while let Some((sx, sz)) = queue.pop_front() {
        visited += 1;
        if visited > max_nodes {
            break;
        }
        let (wx, wz) = subcell_center(sx, sz);
        if target.reached(wx, wz) && walkable(world, sx, sz) {
            return Some((index(sx, sz), parents, visited));
        }

        for (dx, dz) in [(1, 0), (0, 1), (-1, 0), (0, -1)] {
            let (nx, nz) = (sx + dx, sz + dz);
            if !in_bounds(nx, nz) {
                continue;
            }
            let ni = index(nx, nz);
            if seen[ni] || !walkable(world, nx, nz) {
                continue;
            }
            seen[ni] = true;
            parents[ni] = index(sx, sz);
            queue.push_back((nx, nz));
        }
    }
    None
}

fn walkable(world: &World, sx: i32, sz: i32) -> bool {
    if !in_bounds(sx, sz) {
        return false;
    }
    let (x, z) = subcell_center(sx, sz);
    let Ok(x) = i16::try_from(x) else {
        return false;
    };
    let Ok(z) = i16::try_from(z) else {
        return false;
    };

    // Most field scenes carry the authored object/floor grid. For synthetic
    // or partially loaded scenes with no object grid, fall back to the retail
    // collision-wall rule instead of declaring the entire map unreachable.
    let on_floor = if world.terrain.object_cells.is_empty() {
        true
    } else {
        world.field_tile_is_walk_visible(x, z)
    };
    on_floor && !world.field_tile_is_wall(x, z)
}

fn raw_direction_for_world(desired_world: u16, camera_azimuth: u16) -> u16 {
    let rot = ((u32::from(camera_azimuth).wrapping_add(0x100) >> 9) & 7) as u32;
    DIR_RING
        .iter()
        .copied()
        .find(|raw| World::remap_pad_direction(*raw, rot) & 0xF000 == desired_world)
        .unwrap_or(0)
}

fn buttons_for_direction(raw: u16) -> Vec<Button> {
    let mut buttons = Vec::with_capacity(2);
    if raw & 0x1000 != 0 {
        buttons.push(Button::Up);
    }
    if raw & 0x2000 != 0 {
        buttons.push(Button::Right);
    }
    if raw & 0x4000 != 0 {
        buttons.push(Button::Down);
    }
    if raw & 0x8000 != 0 {
        buttons.push(Button::Left);
    }
    buttons
}

fn world_direction(from: (i32, i32), to: (i32, i32)) -> Option<u16> {
    match (to.0 - from.0, to.1 - from.1) {
        (1, 0) => Some(0x2000),
        (-1, 0) => Some(0x8000),
        (0, 1) => Some(0x1000),
        (0, -1) => Some(0x4000),
        _ => None,
    }
}

fn to_subcell(x: i32, z: i32) -> (i32, i32) {
    (x.div_euclid(SUBCELL_SIZE), z.div_euclid(SUBCELL_SIZE))
}

fn subcell_center(sx: i32, sz: i32) -> (i32, i32) {
    (
        sx * SUBCELL_SIZE + SUBCELL_CENTER,
        sz * SUBCELL_SIZE + SUBCELL_CENTER,
    )
}

fn in_bounds(sx: i32, sz: i32) -> bool {
    (0..SUBCELL_STRIDE).contains(&sx) && (0..SUBCELL_STRIDE).contains(&sz)
}

fn index(sx: i32, sz: i32) -> usize {
    sz as usize * SUBCELL_STRIDE as usize + sx as usize
}

fn coords(index: usize) -> (i32, i32) {
    (
        (index % SUBCELL_STRIDE as usize) as i32,
        (index / SUBCELL_STRIDE as usize) as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_camera_ring_finds_expected_screen_direction() {
        // Camera 0: screen up is world Z+.
        assert_eq!(raw_direction_for_world(0x1000, 0), 0x1000);
        // Quarter-turn camera: one of the screen cardinals must still map to
        // world Z+. Verify by round-tripping through the engine's own remap.
        let raw = raw_direction_for_world(0x1000, 1024);
        let rot = ((1024u32 + 0x100) >> 9) & 7;
        assert_eq!(World::remap_pad_direction(raw, rot) & 0xF000, 0x1000);
    }

    #[test]
    fn raw_ring_turns_into_real_pad_buttons() {
        assert_eq!(buttons_for_direction(0x1000), vec![Button::Up]);
        assert_eq!(
            buttons_for_direction(0x3000),
            vec![Button::Up, Button::Right]
        );
        assert_eq!(buttons_for_direction(0xC000), vec![Button::Down, Button::Left]);
    }

    #[test]
    fn target_radius_is_euclidean() {
        let target = NavigationTarget { x: 100, z: 100, radius: 10 };
        assert!(target.reached(106, 106));
        assert!(!target.reached(111, 100));
    }

    #[test]
    fn subcell_round_trip_uses_centers() {
        assert_eq!(to_subcell(96, 160), (1, 2));
        assert_eq!(subcell_center(1, 2), (96, 160));
    }
}
