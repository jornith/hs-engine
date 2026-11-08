# hs-engine

[![CI](https://github.com/jornith/hs-engine/actions/workflows/ci.yml/badge.svg)](https://github.com/jornith/hs-engine/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A one-turn Hearthstone simulator and turn search, with a Rust core and Python bindings.

You give hs-engine a position and the cards in play. It lists every legal move and plays each one out:
damage, deaths, Deathrattles, auras, triggers and the end of the turn. It then searches the turn for the best
line, scores each candidate after a simulated enemy reply, and returns the line with its final position. The
same core plays whole games against itself for self-play. Card behaviour comes from a parser of English card
text plus hand-written rules, and every playable card has at least one regression case.

## Contents

- [At a glance](#at-a-glance)
- [Install from source](#install-from-source)
- [Quick start](#quick-start)
- [Accuracy](#accuracy)
- [Status and known limits](#status-and-known-limits)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [License](#license)

## At a glance

| Area | What the repository has |
| --- | --- |
| Rust core | Crate `hs-engine` in `engine/`, no I/O: legal moves, 80 effect ops, combat and deaths, Deathrattles, Reborn, auras, 10 trigger events, end-of-turn effects |
| Search | Depth-first search over every line of the turn, re-ranked after a greedy enemy reply and a look at my next turn |
| Evaluation | Hand-tuned weights in `hsengine/weights.py` |
| Self-play | Whole games between decks, with optional Monte Carlo rollouts or MCTS |
| Python package | `hsengine`: game log and replay parsing, the card-text parser, the bindings (`hsengine._core`, built from `python/`) |
| Card rules | 1,115 hand-written rules for 1,051 card ids in `hsengine/card_fx/` |
| Tests | 41 Rust rule tests, 49 turn scenarios, 4 package checks, 27,755 per-card cases |
| Card data | Not shipped; read from CardDefs.xml through the `hearthstone` package ([docs/DATA.md](docs/DATA.md)) |

## Install from source

You need Rust 1.85 or newer and Python 3.10 or newer. From the repository root:

```sh
python -m venv .venv
.venv/bin/pip install maturin
.venv/bin/maturin develop --release --extras test
```

`maturin develop` builds the Rust core and installs `hsengine` into the virtual environment. Install
`hearthstone_data` as well if you want to load card data offline:

```sh
.venv/bin/pip install hearthstone_data
```

To build and test the Rust crate alone:

```sh
cargo build --release
cargo test --workspace
```

## Quick start

This position gives me a 4/5 minion that can attack and a Fireball in hand, against an enemy hero on 10
health:

```python
from hsengine import Ent, GameState, Planner, cards

db = cards.load()  # CardDefs.xml, see docs/DATA.md

st = GameState(turn=8, my_turn=True, mana=4, max_mana=4, my_hp=30, enemy_hp=10, main_action=True)
st.hand = [Ent(20, "CS2_029", 4, 0, 0)]  # Fireball: entity id, card id, cost, attack, health
st.board = [Ent(10, "CS2_182", 4, 4, 5, can_attack=True, attacks=1, maxhp=5)]  # a 4/5 minion

score, path, final = Planner(st, db).search()
for action, _position in path:
    print(action)
print("enemy health:", final.en_hp)
```

The search finds lethal: attack with the minion, then cast Fireball at the enemy hero.

```text
('attack', 10, 'face')
('play', 20, 'face', 0)
enemy health: 0
```

From here:

- **Live games.** `hsengine.StateTracker` reads `Power.log` lines (`feed`) into a `GameState` (`snapshot`).
- **Self-play.** `hsengine.selfplay.run` plays whole games between decks:

  ```python
  from hsengine import cards, selfplay

  db = cards.load()
  deck = ["CS2_120", "CS2_182", "CS2_029"] * 10
  decks = [selfplay.Deck(deck, selfplay.POWERS["MAGE"]), selfplay.Deck(deck, selfplay.POWERS["WARRIOR"])]
  out = selfplay.run(decks, db, n_games=4)
  print(out["results"])  # (deck a, deck b, winning seat or -1, turns, B's seat or -1) per game
  ```

- **How it works.** [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) follows a position from the log to the
  searched line.

## Accuracy

`tools/sim_check.py` replays every action of logged games through the engine and compares the result with the
position the game logged next. `tools/eot_check.py` does the same for the end of each turn.

| Measure | Result |
| --- | --- |
| All actions, exact match | 82.0% of 90,020 |
| Deterministic actions (no random effect), boards and heroes match | 89.2% of 55,830 |
| Turn ends, end-of-turn effects match | 95.8% of 23,008 |

The action numbers come from 3,070 logged games; the turn-end number from an earlier 3,001-game version of the
same corpus. [docs/ACCURACY.md](docs/ACCURACY.md) explains the measures and what they leave out.

## Status and known limits

Version 0.1.0 is the first public release. It installs from source only, and the Python and Rust APIs may
still change.

- **One turn, my side.** The engine searches my turn. The enemy reply is a single greedy turn, and no tool
  measures how close it comes to real opponents.
- **Hidden information.** Draws, Discover and generated cards count by value; the engine does not simulate
  the unknown cards.
- **Random effects.** The engine plays one outcome per random effect, picked by a fixed policy. Some of these
  policies are fitted to the corpus ([docs/ACCURACY.md](docs/ACCURACY.md#fitted-rules)).
- **Game modes.** Standard constructed one-on-one games only, with English card text.
- **Known misses.** 1,125 logged cases in the per-card suite still disagree with the game, sorted by cause in
  [docs/KNOWN_GAPS.md](docs/KNOWN_GAPS.md).
- **Evaluation.** The weights are tuned by hand. Self-play is the tool for comparing them.

## Documentation

| Page | Contents |
| --- | --- |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Terminology, data flow, module map, ops, triggers, search, self-play |
| [docs/RULES_FORMAT.md](docs/RULES_FORMAT.md) | The JSON format of hand-written card rules and summon overrides |
| [docs/DATA.md](docs/DATA.md) | What the repository ships and what you supply |
| [docs/TESTING.md](docs/TESTING.md) | The test suites and how the per-card fixtures are built |
| [docs/ACCURACY.md](docs/ACCURACY.md) | How agreement with real games is measured, and its caveats |
| [docs/KNOWN_GAPS.md](docs/KNOWN_GAPS.md) | The known misses of the per-card suite, by cause |
| [CHANGELOG.md](CHANGELOG.md) | Release notes |

## Contributing

Bug reports and pull requests are welcome. [CONTRIBUTING.md](CONTRIBUTING.md) covers the setup, the checks a
pull request must pass and how to change card rules.

## License

See [LICENSE](LICENSE).

This is an unofficial project, not affiliated with or endorsed by Blizzard Entertainment. Hearthstone and its
card names and texts belong to Blizzard Entertainment.
