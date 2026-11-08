# Card rule format

`hsengine/rules.py` compiles most cards from their English text. When the parser gets a card wrong, or the
card does something its text patterns do not cover, you write a rule for it by hand in
`hsengine/card_fx/*.json`. A rule replaces the parsed fields it names and keeps the others.

This page is the reference for that format. [ARCHITECTURE.md](ARCHITECTURE.md#terminology) defines the terms
it uses.

## Contents

- [Rule files](#rule-files)
- [Rule keys](#rule-keys)
- [Op fields](#op-fields)
- [Ops](#ops)
- [Random effects](#random-effects)
- [Summon overrides](#summon-overrides)
- [Minion types](#minion-types)
- [Checking a rule](#checking-a-rule)

## Rule files

Each file maps card ids to rules. Most files cover one card set or theme (`CORE_1.json`,
`LOCATIONS_HEROES.json`); the `ZZ*` files hold later fixes. The loader reads the files in name order, and a
later file wins when two define the same id. The 29 files hold 1,115 entries for 1,051 card ids.

```json
{
  "BE_283": {"play": [{"op": "aoe", "n": 2, "scope": "other_minions"}]},
  "BE_749": {"dr": [{"op": "summon", "n": 2, "a": 1, "h": 2}, {"op": "value", "n": 20}]},
  "BE_781": {}
}
```

An empty rule (`{}`) keeps the parsed model as it is. Use it to mark a card as checked.

## Rule keys

| Key | Type | Meaning |
| --- | --- | --- |
| `play` | list of ops | What the card does when played: Battlecry, spell effect, weapon or hero card effect. |
| `modes` | list of op lists | Choose One cards and other cards with alternatives. The search tries each mode. |
| `activate` | list of ops | A location's effect. The loader adds the value of the charges left. |
| `dr` | list of ops | Deathrattle. `summon` ops become the minion's death summon; the other ops count as a value. |
| `dr_gen` | number | A Deathrattle that puts this many cards (a Coin) in my hand. |
| `dr_card` | string | Card id of the token a plain-summon Deathrattle makes. The token keeps its own rules, so an egg that hatches a minion that hatches again chains correctly. |
| `eot` | list of ops | "At the end of your turn" effects. |
| `wtrig` | list of ops | A weapon's "after your hero attacks" effects. Replaces the ones parsed from its text. |
| `trig` | list of `[event, ops, filter]` | Triggered effects. Events and filters are listed in [ARCHITECTURE.md](ARCHITECTURE.md#triggers). |
| `aura` | `[attack, health]` | Stats the card gives my other minions while it lives. |
| `pv` | number | Static value of ongoing text the engine cannot play out. |
| `_why` | string | A note for maintainers. The loader ignores it. Fitted rules say `FITTED` here. |

`modes` wins over `play` when a rule has both. `activate` applies to locations only. A silenced minion loses
`dr`, `dr_gen`, `dr_card`, `eot`, `trig` and `pv`.

## Op fields

An op is an object with an `op` name and up to seven more fields.

| Field | Default | Meaning |
| --- | --- | --- |
| `op` | required | One of the names in `OPS` in `engine/src/state.rs` |
| `n` | `0` | Main number: damage, healing, a count |
| `a`, `h` | `0` | Attack and health of a buff or summon; some ops use them as parameters |
| `kw` | `[]` | Keywords: `taunt`, `divine`, `windfury`, `charge`, `rush`, `stealth`, `lifesteal`, `poisonous`, `reborn`, `doomed` |
| `tgt` | `""` | Chosen target: `enemy_char`, `enemy_minion`, `friendly_char`, `friendly_minion`, `any_minion`, `any_char`, or `same` for the previous op's target. Empty means no target. |
| `cond` | `""` | Condition (see below) |
| `scope` | `""` | Area: `enemies`, `enemy_minions`, `minions`, `chars`, `other_chars`, `other_minions`, `other_enemies`, `others`, `mine` |

`cond` takes one of these values:

| Value | Effect |
| --- | --- |
| `atk>=N`, `atk<=N` | Only targets with that much attack |
| `damaged`, `undamaged` | Only damaged or undamaged targets |
| `temp` | The buff lasts this turn |
| `combo` | The effect works only when I played another card earlier this turn |
| `race:NAME` | The buff reaches only friendly minions of one type, such as `race:ELEMENTAL` |

## Ops

`OPS` in `engine/src/state.rs` lists all 80 op names. These are the ones most rules use.

| Op | Fields | Effect |
| --- | --- | --- |
| `dmg` | `n`, `tgt`, `cond` | Deal `n` damage to the chosen target. |
| `face` | `n` | Deal `n` damage to the enemy hero. |
| `aoe` | `n`, `scope` | Deal `n` damage to every character in `scope`. |
| `dmg_random` | `n` | Deal `n` damage to random enemies (one random target, or split). |
| `dmg_same_race` | `n`, `tgt` | Deal `n` damage to the chosen minion and to every other minion, on either side, that shares a type with it. |
| `halve_enemy` | | Set the enemy hero's health to half, rounded down. |
| `heal`, `heal_hero` | `n`, `tgt` | Restore `n` health to the target, or to my hero. |
| `buff`, `buff_all` | `a`, `h`, `kw`, `tgt` or `scope`, `cond` | Give stats and keywords. |
| `hand_buff` | `a`, `h` | Give the minions in my hand stats. |
| `summon` | `n`, `a`, `h`, `kw` | Summon `n` minions with these stats for me. |
| `mask_fill` | | Make the chosen minion (else my weakest other one) a 1/1, then fill my board with 1/1 minions. |
| `destroy` | `tgt`, `cond` | Destroy the chosen minion. On my own minion, its death runs as in combat, Deathrattle included. |
| `freeze`, `silence`, `bounce`, `transform` | `tgt` and fields | Single-target removal. |
| `armor`, `hero_atk`, `weapon` | `n`, or `a` and `h` | Hero armor, hero attack this turn, or equip a weapon. |
| `draw`, `discover`, `gen` | `n` | Card advantage. The engine scores cards it cannot know by value. |
| `opp_cards` | `n` | Give the opponent `n` cards: a small penalty in the score. |
| `draw_brk` | `n` | Weapon rule: draw `n` cards if the swing broke the weapon. |
| `draw_kill` | `n` | Weapon rule: draw `n` cards if the swing killed a minion. |
| `dmg_surv` | `n`, `a`, `h`, `tgt` | Deal `n` damage to a minion. If it lives, draw a card; if it dies, summon an `a`/`h` minion for me. |
| `aoe_draw` | `n` | Deal `n` damage to every minion and draw a card for each that dies. |
| `dmg_chain` | `n`, `tgt` | Deal `n` damage to an enemy minion. While the victim dies, repeat on another enemy minion. |
| `dmg_split` | `n`, `a` | Deal `n` damage one point at a time to enemy minions. With `a` = 1, heal my hero for every point that landed. |
| `dmg_excess` | `n`, `a`, `h` | Deal `n` damage to one enemy minion, `a` times (at least once). The excess of a kill hits the enemy hero, and so does the whole hit when no minion is left. `h` picks the target policy. |
| `shuffle_self` | | The Battlecry minion is shuffled into the deck and leaves the board. |
| `mana`, `ramp` | `n` | Mana this turn, or an empty mana crystal. |
| `value` | `n` | No effect on the board. Adds `n` to the score for what the rule leaves out. |

The core ignores an op name it does not know, so a typo leaves the card without that effect. The per-card
tests catch this, as described in [Checking a rule](#checking-a-rule).

## Random effects

The engine plays one outcome of a random effect, picked by a fixed policy:

- `dmg_chain` and `dmg_split` pick the lowest-health enemy minion.
- `dmg_excess` takes its policy from `h`: 0 lowest health, 1 highest health, 2 leftmost, 3 rightmost,
  4 highest attack. Add 10 to turn off the overflow to the enemy hero; with no minion left, the hit then has
  no effect.

Some of these choices are fitted to the corpus. [ACCURACY.md](ACCURACY.md#fitted-rules) lists them.

## Summon overrides

`hsengine/summon_overrides.json` records, per card id, the minions the card summoned in logged games. Each
entry has the minions (`summons`, as `[attack, health, keywords...]`), the number of logged plays behind the
entry (`n`) and how often that result came up (`share`). The file holds stable outcomes only.

When a card in hand has a non-empty entry and none of its modes already summons those minions, the planner
replaces the `summon` ops of its modes with the listed minions. Herald cards use the key
`card_id|h<herald count, at most 4>|<CLASS>`, for example `CATA_785|h1|ROGUE`.

## Minion types

`race:` conditions and `dmg_same_race` read the type bits on each unit. The Python side fills them from the
`races` of each card (`hsengine.cards.race_mask`). A minion whose card has no type has the mask 0, so a
`race:` buff skips it and `dmg_same_race` hits only the target. A card of type `ALL` matches every type.

## Checking a rule

The per-card suite records what each card model does, so a changed rule shows up as a changed snapshot or a
newly failing real case. After you add or change a rule:

1. Rebuild the extension if you changed Rust code (`.venv/bin/maturin develop --release`).
2. Run `.venv/bin/pytest -q tests/test_cards.py` and read every changed case.
3. If the new results are right, regenerate the fixtures as described in
   [TESTING.md](TESTING.md#regenerating-the-fixtures).

[CONTRIBUTING.md](../CONTRIBUTING.md#changing-card-rules) explains when a rule change is accepted.
