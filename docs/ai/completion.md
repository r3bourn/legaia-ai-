# Completionist run contract

`data/ai/completion.toml` is the optional-content overlay for the one continuous live AI playthrough. When the orchestrator selects the `completionist` profile, this file is loaded alongside `data/ai/progression.toml` and the story spine.

The story spine still decides where the run is in the main plot. The completion overlay may insert detours and grinding between spine milestones, but it must never seed from an upstream save or skip ahead. Every action still happens in the same cold-New-Game session and every simulation tick remains gated by the active spectator lease.

## What “100%” means for this project

The completionist profile targets the practical in-game completion route used by the major 100% walkthroughs:

- the guide-backed 260/260 unique-item collection target;
- every reachable unique treasure and secret/key item;
- every normal, Hyper, and Miracle Art for Vahn, Noa, and Gala;
- every special/secret Ra-Seru summon: Palma, Mule, Horn, Jedo, Meta, Terra, Ozma, and Juggernaut;
- every major optional boss/reward, including Lapis and the Queen Bee reward;
- major optional minigame rewards from Muscle Dome, fishing, and Sol entertainment;
- at least one learned party copy of every unique normal Seru spell encountered by the completion route.

The profile does **not** require all three characters to absorb every normal Seru spell. That is a maxed-save challenge rather than the normal 100% collection definition. It can be added later as a stricter profile without changing the story route.

## Ra-Seru completion

The planner should treat the eight special summons as hard completion goals, not optional combat upgrades.

Palma requires the Earth Egg from the Muscle Dome coin exchange, then Zalan's appraisal into the Earth Talisman. Mule requires the Water Egg from the Buma fishing exchange, then the Water Talisman. Horn comes from Cara's Light Egg after the Buma/Cara/Grantes sidequest and Zalan's appraisal. Jedo comes from the Dark Stone chest at the dead West Voz Genesis Tree after the final Songi battle and becomes the Dark Talisman.

Meta, Terra, and Ozma require the late-game Genesis Tree revisits after the final Songi battle with the corresponding character's Arts requirements satisfied. Juggernaut is the intentionally extreme endpoint: all three main characters must reach level 99, Saryu supplies the Evil Seru Key, and the former Juggernaut room yields the Evil Talisman.

Because old FAQs disagree on a few item-id labels, runtime checks should resolve talisman/egg ids from the repository's decoded item/effect tables whenever available rather than trusting prose tables. The repository's accessory-passive documentation is authoritative for the engine's talisman summon behavior.

## Planner behavior

Completion goals are mandatory unless the live engine proves that a goal is permanently unavailable in the current genuine playthrough. If that happens, the orchestrator must stop before the final boss and report the exact unresolved goal instead of silently downgrading the run.

For grind-heavy goals such as 100,000 Muscle Dome coins, 20,000 fishing points, all Arts, or level 99 for Juggernaut, the AI may automate the grind while the viewer is present. The watcher lease still applies: when Jeremy stops watching, grinding stops at the exact authoritative simulation state.

Before Cort, the orchestrator runs a final audit. It must check all eight special summons, Arts completion, optional bosses, major minigame rewards, normal-Seru collection, and the unique-item completion ledger. If a reachable item is missing, the planner backtracks rather than starting the final battle.

## Research basis

The overlay normalizes facts from Meth962's 100% walkthrough, BSauls' Seru guide, Dan_GC's misc FAQ, PsychoPenguin's walkthrough, and repository-decoded tables. Guide prose is not copied; only compact requirements and route facts are stored.
