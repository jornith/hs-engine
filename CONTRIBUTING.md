# Contributing

This page covers the development setup, the checks every pull request must pass, and the rules for changing
card behaviour or the engine.

## Contents

- [Setup](#setup)
- [Before you open a pull request](#before-you-open-a-pull-request)
- [Changing card rules](#changing-card-rules)
- [Changing the engine](#changing-the-engine)
- [Reporting a wrong card](#reporting-a-wrong-card)
- [Style](#style)
- [Data](#data)

## Setup

You need Rust 1.85 or newer and Python 3.10 or newer.

```sh
python -m venv .venv
.venv/bin/pip install maturin
.venv/bin/maturin develop --release --extras test
```

The Python tests load the built extension, so run `maturin develop` again after every Rust change.

## Before you open a pull request

Run the same checks as CI. CI treats every warning as an error.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
.venv/bin/ruff check . && .venv/bin/ruff format --check .
.venv/bin/pytest -q
```

[docs/TESTING.md](docs/TESTING.md) describes what each suite checks.

## Changing card rules

To fix one card, add or change its rule in `hsengine/card_fx/` ([docs/RULES_FORMAT.md](docs/RULES_FORMAT.md)).
When the fix applies to a whole family of card texts, prefer a parser pattern in `hsengine/rules.py`.

Change a rule only when the card's text and several logged actions agree, never to fit a single case. If you
choose a rule because it lowers the wrong count on the corpus rather than because the text says so, mark it
`FITTED` in its `_why` field and add it to [docs/ACCURACY.md](docs/ACCURACY.md#fitted-rules).

## Changing the engine

- A change that alters what the engine does changes snapshots in the per-card suite. Check each changed case.
  If the new result is right, regenerate the fixtures from your replays
  ([docs/TESTING.md](docs/TESTING.md#regenerating-the-fixtures)) and put the new `manifest.json` numbers in
  the pull request.
- Run `tools/sim_check.py` on your replays before and after the change, and report both rates
  ([docs/ACCURACY.md](docs/ACCURACY.md)). A pull request must not lower the deterministic rate without a stated
  reason.
- For every rule bug you fix, add a scenario to `tests/test_planner.py` or a Rust test to
  `engine/tests/rules.rs`.
- If the golden fixture changes, regenerate it as described in
  [docs/TESTING.md](docs/TESTING.md#package-checks) and explain why in the commit.

## Reporting a wrong card

Open an issue with the card id, what the engine does and what the game does. If you have a logged action
that shows the difference, describe the position (both boards, both heroes, the card's target). Do not attach
replays or logs: they hold player names and account ids.

## Style

- **Rust:** `rustfmt` defaults. Give every public item a `///` doc comment and every file a `//!` doc comment
  that says what the module owns. Keep functions under about 80 lines.
- **Python:** the ruff settings in `pyproject.toml`, with a line length of 120.
- **Commits:** an imperative subject under 60 characters. Add a body when the reason for the change is not
  obvious from the diff.

## Data

Do not commit card data, replays, logs, decks or anything that names a player ([docs/DATA.md](docs/DATA.md)).
