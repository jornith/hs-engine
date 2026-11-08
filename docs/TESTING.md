# Testing

The engine has four test suites. None of them needs card data or network access, and CI
(`.github/workflows/ci.yml`) runs all of them on every push and pull request.

## Contents

- [Running the checks](#running-the-checks)
- [Rust rule tests](#rust-rule-tests)
- [Turn scenarios](#turn-scenarios)
- [Package checks](#package-checks)
- [Per-card suite](#per-card-suite)
- [How the fixtures are built and anonymised](#how-the-fixtures-are-built-and-anonymised)
- [Regenerating the fixtures](#regenerating-the-fixtures)

## Running the checks

Build the extension first ([README.md](../README.md#install-from-source)), then:

```sh
cargo test --workspace                                       # Rust rule tests
.venv/bin/pytest -q                                          # all Python suites
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
.venv/bin/ruff check . && .venv/bin/ruff format --check .
```

CI runs the Rust checks on stable Rust and the Python checks on Python 3.10 and 3.12.

| Suite | File | Tests |
| --- | --- | --- |
| Rust rule tests | `engine/tests/rules.rs` | 41 |
| Turn scenarios | `tests/test_planner.py` | 49 |
| Package checks | `tests/test_api.py` | 4 |
| Per-card suite | `tests/test_cards.py` | 27,755 cases and 3 consistency checks |

## Rust rule tests

`engine/tests/rules.rs` builds positions by hand and calls the engine directly. The tests cover:

- **Combat and damage:** Divine Shield, Poisonous and immunity; Spell Damage on spells only; armor; area
  scopes; Taunt blocking face attacks; halving the enemy hero.
- **Board and deaths:** Deathrattle summons for the right owner, Reborn, the seven-minion limit, doomed
  minions, death and end-of-turn triggers, auras on play and on death, adjacency auras.
- **Rule details:** unknown ops doing nothing, the string-to-code tables, minion types, weapon durability,
  Elusive minions as Battlecry targets only, immune enemy minions as targets, the `combo` condition and the
  played-card counter, destroying my own minion.
- **Ops with their own logic:** `draw_brk`, `draw_kill`, `dmg_surv`, `aoe_draw`, `dmg_chain`, `dmg_split`,
  `dmg_excess`, `shuffle_self`, `mask_fill`, Deathrattle cards in hand.
- **Search:** finding lethal, and keeping hand cards with different effects as different moves.

## Turn scenarios

`tests/test_planner.py` holds 49 turn scenarios on synthetic cards from `tests/synth.py`. The synthetic cards
carry rules text in the card-database style, so the scenarios exercise the parser as well as the core.

They check that the search finds lethal through the board, a Taunt or a Divine Shield, and never claims lethal
behind a Taunt. Others check heals, hero powers, board clears, removal on the biggest threat, Freeze,
Poisonous trades, weapons, Windfury and attack buffs. Rush minions must not go face on the turn they enter.
Deathrattle and Reborn results, auras, draws burnt by a full hand and enemy Lifesteal each have a scenario.

The rest cover hand-written rules and special cases: weapons that draw, damage that draws or summons,
Deathrattles that add cards or fill the opponent's hand, script numbers read from the card, spell numbers
doubled by a weapon, Combo, Outcast, held-back cards, and a large board that must finish within the time
budget.

## Package checks

`tests/test_api.py` checks the package surface:

- `cards.load` on a missing file raises `CardDataError` with the fix in its message.
- Minion type masks come out right for one type, two types, no type, `ALL` and unknown type names.
- The golden fixture `tests/fixtures/golden.json` records one fixed position: every result of playing one
  card, and the line the search picks under a node limit. After a deliberate engine change, regenerate it with
  `HSENGINE_UPDATE_GOLDEN=1 .venv/bin/pytest tests/test_api.py` and explain the change in the commit.
- A self-play smoke run plays four games between two synthetic decks twice and checks that both runs agree.

## Per-card suite

`tests/test_cards.py` gives every playable card in the card database at least one case. The cases live in
`tests/fixtures/cards/*.jsonl.gz`: 223 files, one per card-id prefix, 2.8 MB in total. Each case holds a
compiled engine context, so the suite runs without card data.

| Kind | Cases | What the test checks |
| --- | --- | --- |
| real | 3,915 pass, 1,125 known misses | A logged action of the card. The engine must produce the outcome the game logged: both boards, both heroes, hand size. |
| modeled | 18,103 | A card with no logged action, played into one synthetic position (turn 15, 10 mana, two minions a side). Every engine result must match the recorded snapshot. |
| invariant | 4,612 | As modeled, for cards with random effects. The test checks the number of moves and bounds: at most 7 minions a side, no negative mana, health within the maximum. |

The suite covers 24,334 cards, 1,619 of them with real cases. `tests/fixtures/cards/manifest.json` records
these counts at export time, and three checks hold the suite to them:

- `test_real_agreement` fails if fewer real cases pass than at export.
- `test_coverage` fails if a card loses its last case.
- `test_every_miss_has_a_reason` fails if a known miss has no reason in `xfail_reasons.json`.

Known misses are strict `xfail`: a fix that makes one pass fails the suite until you regenerate the fixtures.
[KNOWN_GAPS.md](KNOWN_GAPS.md) sorts them by cause.

## How the fixtures are built and anonymised

`tools/export_fixtures.py` reads the position caches that `tools/sim_check.py` writes from your replays. For
each card it keeps up to three logged actions the engine gets right and one it gets wrong, smallest context
first. Cards without a logged action get a modeled or invariant case instead.

A stored case keeps the engine context and numbers only:

- Card models arrive compiled: ops, stats and keyword bits. No card text, art or flavour text goes in.
- Every card name becomes a per-case token (`n0`, `n1`, ...). The engine compares names for equality only,
  so the tokens behave the same.
- Entity ids stay as they are, since they are counters local to one game. Games are numbered by their
  position in the input list.
- Player names, account ids, timestamps, deck lists and the game log itself stay out.

## Regenerating the fixtures

You need your own replays and card data ([DATA.md](DATA.md)).

```sh
python tools/sim_check.py --replays DIR              # writes the position cache to .cache/sim_check
python tools/export_fixtures.py --cache .cache/sim_check
python tools/triage_xfail.py                         # writes xfail_reasons.json and docs/KNOWN_GAPS.md
```

The exporter overwrites `tests/fixtures/cards/` and its `manifest.json`, and prints the corpus agreement
([ACCURACY.md](ACCURACY.md)). `triage_xfail.py` then writes a reason for every known miss;
`test_every_miss_has_a_reason` fails until you run it.
