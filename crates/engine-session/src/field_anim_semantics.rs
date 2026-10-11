//! Renderer-independent field-animation semantics for a session-owned frame tail.
//!
//! Retail field animation is not presentation-only: NPC clip cursors write the
//! end/restart bits field scripts wait on, and the player's locomotion clip
//! selection is part of the live world state. The native/browser render hosts
//! historically installed those cursors while building actor meshes. A
//! headless [`BootSession`](crate::boot::BootSession) therefore ran a subtly
//! different game.
//!
//! This helper owns no meshes and never renders. It retains only the two ANM
//! banks required to resolve semantic clip timing, the placement -> spawn-bank
//! fallback map, and the last scene/leader identities. A session that owns the
//! world frame tail calls [`FieldAnimSemantics::refresh`] before simulation and
//! [`FieldAnimSemantics::step_world_tail`] after it. Render-owned sessions keep
//! using their existing actor layers instead.

use std::collections::HashMap;

use legaia_asset::player_anm::PlayerAnmBundle;
use legaia_engine_core::field_anim::{FieldClipPlayer, FieldPlayerAnim};
use legaia_engine_core::scene::SceneHost;
use legaia_engine_core::world::{SceneMode, WorldFrameTail};

/// The minimal animation state a renderer-less session needs in order to run
/// the same field-script timing as a render host.
#[derive(Default)]
pub(crate) struct FieldAnimSemantics {
    scene: Option<String>,
    scene_anm: Option<PlayerAnmBundle>,
    locomotion_anm: Option<PlayerAnmBundle>,
    spawn_party: HashMap<u8, bool>,
    player_leader: Option<u8>,
}

impl FieldAnimSemantics {
    /// Forget scene-local bindings. The locomotion bank is disc-global and is
    /// intentionally kept so a door does not re-decode PROT 0874 every time.
    pub(crate) fn invalidate_scene(&mut self) {
        self.scene = None;
        self.scene_anm = None;
        self.spawn_party.clear();
        self.player_leader = None;
    }

    /// Ensure the current scene has the same semantic clip setup that the
    /// native/browser actor rebuild installs.
    ///
    /// This is safe to call every frame. Work occurs only on a scene edge, a
    /// missing player animation, or a party-leader change.
    pub(crate) fn refresh(&mut self, host: &mut SceneHost) {
        let scene_name = host.scene.as_ref().map(|s| s.name.clone());
        if scene_name != self.scene {
            self.rebuild_scene(host, scene_name);
        }
        self.ensure_player(host);
    }

    fn ensure_locomotion_bank(&mut self, host: &SceneHost) {
        if self.locomotion_anm.is_some() {
            return;
        }
        self.locomotion_anm = host
            .index
            .entry_bytes(legaia_asset::character_pack::PROT_ENTRY_INDEX)
            .ok()
            .and_then(|bytes| legaia_asset::character_pack::field_locomotion_anm(&bytes).ok());
    }

    fn rebuild_scene(&mut self, host: &mut SceneHost, scene_name: Option<String>) {
        self.scene = scene_name.clone();
        self.scene_anm = None;
        self.spawn_party.clear();
        self.player_leader = None;
        self.ensure_locomotion_bank(host);

        let Some(name) = scene_name else {
            return;
        };
        self.scene_anm = host
            .scene
            .as_ref()
            .and_then(legaia_engine_core::npc_catalog::scene_anm_bundle);

        // Build the same play-shape placement catalog as the browser actor
        // layer. Collect immutable facts first, then mutate World to install
        // cursors so the SceneHost's fields never overlap mutable borrows.
        let entries = match host.resources.as_ref() {
            Some(res) => legaia_engine_core::npc_catalog::catalog_scene_npcs(
                &host.index,
                &name,
                res,
                Some(&host.world.field_head_pool),
            )
            .map(|c| c.entries)
            .unwrap_or_default(),
            None => Vec::new(),
        };

        for entry in entries {
            let slot = entry.placement.index as u8;
            self.spawn_party.insert(slot, entry.special);

            // The actor's live +0x5C value after its spawn prologue wins over
            // the MAN header's anim byte. This matters for save crystals and
            // story actors whose first clip is installed by script.
            let anim_id = host
                .world
                .field_npc_live_anim(entry.placement.index)
                .unwrap_or(entry.placement.anim_id);
            let Some(record) = (anim_id as usize).checked_sub(1) else {
                continue;
            };
            let party_bank = host.world.npc_clip_party_bank(slot, entry.special);
            let bundle = if party_bank {
                self.locomotion_anm.as_ref()
            } else {
                self.scene_anm.as_ref()
            };
            if let Some(player) = bundle.and_then(|b| FieldClipPlayer::from_record(b, record)) {
                host.world.bind_npc_clip_cursor(slot, anim_id, &player);
            }
        }
    }

    fn ensure_player(&mut self, host: &mut SceneHost) {
        if !matches!(host.world.mode, SceneMode::Field | SceneMode::WorldMap)
            || host.world.player_actor_slot.is_none()
        {
            return;
        }
        self.ensure_locomotion_bank(host);
        let leader = host.world.party.active_party.first().copied().unwrap_or(0);
        if self.player_leader == Some(leader) && host.world.locomotion.player_anim.is_some() {
            return;
        }

        let anim = self
            .locomotion_anm
            .as_ref()
            .filter(|_| leader <= 2)
            .and_then(|bundle| FieldPlayerAnim::from_locomotion_bank(bundle, leader as usize));
        host.world.set_field_player_anim(anim);
        self.player_leader = Some(leader);
    }

    /// Run the world-side frame tail with the actual animation banks. The
    /// returned visual retarget objects may be dropped by a headless caller:
    /// `World::drain_field_anim_cues` has already rebound the authoritative
    /// world cursors before returning them.
    pub(crate) fn step_world_tail(&mut self, host: &mut SceneHost) -> WorldFrameTail {
        self.refresh(host);
        let scene = self.scene_anm.as_ref();
        let locomotion = self.locomotion_anm.as_ref();
        let spawn_party = &self.spawn_party;
        host.world
            .step_world_frame_tail(scene, locomotion, |slot| spawn_party.get(&slot).copied())
    }
}
