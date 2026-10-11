# AI progression data

`data/ai/progression.toml` is the strategy layer for a **single continuous playthrough**. It is keyed one-for-one by the milestone ids in `scripts/replays/full_game_spine.toml`; it does not describe seeded test segments and it must never require a retail save-state anchor.

The file intentionally separates game facts from policy. Scene order, shop stock, item prices, equipment effects, boss stats and item ids come from the repository's decoded/curated game data. Level targets, purchase priorities and boss tactics are recommendations assembled from the cited walkthroughs and boss guides. They are hints to the live planner, not hard gates: current HP/MP, inventory, money and observed battle performance win over a static recommendation.

## Schema

Top-level metadata:

```toml
[meta]
schema = "legaia-ai-progression-v1"
spine = "scripts/replays/full_game_spine.toml"
```

Each `[[milestone]]` has:

- `id`: exact `full_game_spine.toml` milestone id.
- `scene`: expected CDNAME scene label, duplicated from the spine so a loader can reject mismatched data early.
- `level_min` / `level_max`: recommended party range **before leaving this milestone**. `0/0` means there is no meaningful leveling target (ending/credits). These are planning targets, not completion conditions.
- `level_basis`: short provenance/derivation note. `repo-boss-table` means the target is tied to `data/gamedata/bosses.toml`; `guide` means a cited walkthrough gives the range; `derived` means the value is a conservative bridge to the next documented boss target.
- `buys`: idempotent shop goals. Each entry is `{ id, name, target_qty, priority, reason }`. `target_qty` is the desired **final bag count**, not "buy this many more." A shop policy should buy only `target_qty - currently_owned`, clamped by money and the 99-item cap.
- `equipment`: desired equipment goals. Each entry is `{ party, slot, id, name, priority, reason }`. `party` is `vahn`, `noa`, or `gala`; `slot` is `weapon`, `head`, `body`, `boots`, or an accessory slot (`goods1`..`goods3`). The live planner may keep a better already-equipped item.
- `must_get`: high-value treasure/story-item goals. Each entry is `{ kind, name, id?, reason }`. `id` is omitted for Seru magic because Seru spell identity is not the shared inventory id space.
- `seru`: absorb targets that are available in this stretch. These are names, not inventory ids.
- `bosses`: zero or more boss policy records for bosses expected before the next milestone. Fields are `{ name, element, weakness, guard_id?, guard, healing, tactics }`. `guard_id` is present only when the guard is an inventory accessory from the shared item-id space. `element` describes the documented boss/attack affinity relevant to defensive guards. `weakness` is a separately researched offensive vulnerability or guide recommendation; the runtime must **not infer it automatically as the opposite of `element`**. Old Legaia research explicitly shows that attack element and damage susceptibility are not the same measurement. If a weakness is not independently established, treat it as unknown and prefer observed damage/Arts rather than an inferred elemental counter.
- `notes`: compact planner notes for constraints that do not fit another field.

All inventory ids are hexadecimal `u8` values in Legaia's shared 256-entry item space. The ids are cross-checked against the item list embedded in Meth962's GameFAQs walkthrough and the repository's item/equipment tables. Examples: Healing Leaf `0x77`, Deep Sea Jewel `0xDE`, Defender Chain `0xD4`, Astral Sword `0xBA`, Point Card `0xFE`.

## Sources and precedence

The file's `[sources]` table records the research inputs. Precedence for contradictory facts is:

1. disc-derived repository tables / engine behavior;
2. repository curated gamedata (`data/gamedata/*.toml`);
3. GameFAQs factual tables and walkthrough observations;
4. guide recommendations.

For example, the repository corrects Forest Amulet's shop price to 2000 G from the SCUS item table even though some old FAQs list 4000 G. The progression file therefore never overrides a decoded price or item property with a guide value.

The main research references are:

- Meth962, *Legend of Legaia 100% Walkthrough*, GameFAQs FAQ 53721. Used for item ids, shop/treasure availability, boss notes, and the continuous route's optional/high-value pickups.
- BSauls, *Seru Guide*, GameFAQs FAQ 4737. Used for Seru availability by area.
- KBoon, *Guide and Walkthrough*, GameFAQs FAQ 4722. Used as a second source for early boss preparation, notably Zeto.
- PsychoPenguin, *Guide and Walkthrough*, GameFAQs FAQ 13719. Used as a second source for late-game level ranges and treasure lists.
- MStaMaria, *Boss Guide*, GameFAQs FAQ 4732. Used as a second source for late boss survivability guidance.
- `data/gamedata/bosses.toml`, `enemies.toml`, `shops.toml`, `items.toml`, `accessories.toml`, and the equipment tables in this repository.

No guide prose is copied into `progression.toml`. The data is normalized into short factual or policy records.

## Runtime rules

Progression data is advisory and must be evaluated against live state.

- Do not grind merely because `level_min` is unmet if the party is already winning normal encounters safely and the next boss policy has been proven by the live run.
- Do not spend gold below a configured healing/revive reserve to satisfy a low-priority equipment goal.
- Do not rebuy a `target_qty` already met.
- Do not replace an equipped item with a weaker item simply because the milestone lists it. Compare live equipment stats/effects first.
- `must_get` entries marked by story progression are mandatory; optional power/collection pickups can be skipped by a non-100%-completion profile.
- Seru absorb goals should use the live encounter table and stop after capture. Ivory Book (`0xEE`) can be prioritized once available for rare late Seru.
- Before a boss, boss-specific guard/status protection takes priority over generic damage accessories when the live party is near the lower end of the recommended level range.
- Never derive a boss weakness mechanically from its attack element. Use only a documented vulnerability or live observed damage; otherwise treat weakness as unknown.

The intended consumer is the live pad player/orchestrator. It should treat the file as a set of goals and constraints, never as a prerecorded button script.
