# Data

The repository ships code, rules and anonymised test fixtures. You supply the card database, and replays if
you want to measure the engine.

## What the repository ships

| Path | Contents |
| --- | --- |
| `hsengine/weights.py` | The hand-tuned evaluation weights. The Rust `Wt` struct has one field per key. |
| `hsengine/card_fx/*.json` | Hand-written card rules: card ids with ops and numbers, no card text ([RULES_FORMAT.md](RULES_FORMAT.md)). |
| `hsengine/summon_overrides.json` | Summon results per card id, as stats and keywords ([RULES_FORMAT.md](RULES_FORMAT.md#summon-overrides)). |
| `tests/fixtures/` | Compiled engine contexts with card names replaced by tokens ([TESTING.md](TESTING.md#how-the-fixtures-are-built-and-anonymised)). |

The repository ships no card database, card text, card art, replays, game logs or decks.

## What you supply

### Card data

`hsengine.cards.load()` reads CardDefs.xml through the `hearthstone` package, which `pip` installs with
`hsengine`. You have three options:

| Setup | Behaviour |
| --- | --- |
| `pip install hearthstone_data` | `load()` reads the CardDefs.xml that package provides, offline. |
| Nothing extra | `hearthstone` downloads the current CardDefs.xml when you call `load()`. |
| `cards.load(path="CardDefs.xml")` | `load()` reads a file you already have. |

When none of these works, `load()` raises `CardDataError` with a message that says how to fix it. The tools
take the same choice through `--cards-xml`. The test suites need no card data.

Each `Card` carries its minion types in `races`, read from the same file (names such as `ELEMENTAL`, or `ALL`
for every type). The planner turns them into the type bits the engine uses for `race:` conditions and
same-type damage. A `Card` built without `races` has no type.

### Replays and logs

Only the measurement tools and the fixture exporter read games:

| Option | Input |
| --- | --- |
| `--replays DIR` | Zipped XML replays (`*.xml.zip`, each holding one `replay.xml`) |
| `--logs GLOB` | The game client's `Power.log` files (`sim_check.py` only) |

`tools/sim_check.py` caches parsed positions in `.cache/sim_check/`, and the tools write their results to
`.cache/results/`. Git ignores `.cache/`.

Your replays hold player names and account ids. The fixture exporter keeps them out of the fixtures, and no
other file in the repository stores them. Do not commit replays, logs or decks
([CONTRIBUTING.md](../CONTRIBUTING.md#data)).
