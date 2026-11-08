# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.1.0 - Unreleased

First public release.

### Added

- Rust crate `hs-engine`: legal moves, 80 effect ops, combat and deaths, Deathrattles, Reborn, auras,
  10 trigger events, end-of-turn effects, a greedy enemy reply, position scoring and a depth-first turn
  search with re-ranking ([docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)).
- Self-play of whole games on the hand-tuned weights, with optional Monte Carlo rollouts or MCTS.
- Python package `hsengine`: game log and replay parsing, the card-text parser, the bindings to the Rust core
  and the card database loader.
- 1,115 hand-written card rules for 1,051 card ids, and 511 summon override entries
  ([docs/RULES_FORMAT.md](docs/RULES_FORMAT.md)).
- Measurement tools: `tools/sim_check.py` (action agreement, with a fixed holdout split), `tools/eot_check.py`
  (end-of-turn agreement), `tools/export_fixtures.py` (per-card fixtures) and `tools/triage_xfail.py` (causes
  of known misses).
- Tests: 41 Rust rule tests, 49 turn scenarios, 4 package checks (including a golden fixture and a self-play
  smoke run) and a per-card suite of 27,755 cases covering 24,334 cards ([docs/TESTING.md](docs/TESTING.md)).
- Agreement with 3,070 logged games: 82.0% of 90,020 actions exact, 89.2% of 55,830 deterministic actions.
  End-of-turn agreement on an earlier 3,001-game version of the corpus: 95.8% of 23,008 turn ends
  ([docs/ACCURACY.md](docs/ACCURACY.md)).

