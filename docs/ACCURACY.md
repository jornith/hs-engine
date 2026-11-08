# Accuracy

An engine is useful only as far as it agrees with the real game. Two tools measure that agreement on logged
games, with the game log as ground truth. You supply the replays ([DATA.md](DATA.md#replays-and-logs)).

## Contents

- [Results](#results)
- [Action agreement](#action-agreement)
- [Exact and deterministic rates](#exact-and-deterministic-rates)
- [Hand size and held-back cards](#hand-size-and-held-back-cards)
- [Holdout split](#holdout-split)
- [Fitted rules](#fitted-rules)
- [End-of-turn agreement](#end-of-turn-agreement)
- [What these numbers do not cover](#what-these-numbers-do-not-cover)

## Results

| Measure | Tool | Result |
| --- | --- | --- |
| All actions, exact on all five dimensions | `sim_check.py` | 73,810 of 90,020 (82.0%) |
| Deterministic actions, boards and heroes match | `sim_check.py` | 49,776 of 55,830 (89.2%) |
| Turn ends, boards and heroes match | `eot_check.py` | 22,046 of 23,008 (95.8%) |

The action numbers come from a corpus of 3,070 games, the export that `tests/fixtures/cards/manifest.json`
records. The end-of-turn number was measured on an earlier, 3,001-game version of the corpus.

## Action agreement

```sh
python tools/sim_check.py --replays DIR [--logs GLOB] [--workers N] [--holdout]
```

For each action the replay's owner takes in their main phase (a card played, or a minion or hero attack), the
tool:

1. rebuilds the position just before the action from the log,
2. asks the engine for every move that is the same action (same card or attacker, same target), and
3. compares the closest engine result with the position the log shows right after the action.

An outcome has five dimensions: my board and the enemy board (the sorted attack and health of every minion),
my health plus armor, the enemy's health plus armor, and my hand size.

The tool writes the totals, the worst cards and every wrong action to `.cache/results/`. The per-card fixtures
([TESTING.md](TESTING.md#per-card-suite)) freeze a sample of these actions, so `pytest` catches a regression
without the replays.

## Exact and deterministic rates

An action is **exact** when all five dimensions match.

The **deterministic** rate counts only actions whose card text has no random effect: no random target,
Discover, draw, shuffle, copy or transform. It compares boards and heroes and ignores the hand size, which
generated cards make noisy. Hero attacks always count as deterministic.

The deterministic rate is the better measure of whether the rules are right. The exact rate also moves with
hidden information.

## Hand size and held-back cards

The hand-size dimension counts cards the engine cannot know. A draw, a Discover or a generated card adds a card
whose identity stays hidden, and whether the hand ends at the logged size can depend on information the log
shows only later (a full hand that burns a drawn card, for example). Hand-size agreement therefore includes
cases where hidden information decides the result.

Held-back cards ([ARCHITECTURE.md](ARCHITECTURE.md#held-back-cards)) change the predicted hand size: the
comparison adds `min(held_back, 10 - hand)` to the engine's count, and only when the hand starts full. That
condition is fitted, as described below. The deterministic rate does not use the hand size and is not affected.

## Holdout split

With `--holdout`, `sim_check.py` also reports both rates for two groups: a fixed 20% of the games (those whose
id has `crc32(id) % 5 == 0`) and the other 80%. The split depends only on the game id, so it is the same on
every run.

The current rules were tuned on all games, the 20% group included. That group is therefore not unseen data,
and close rates between the groups say nothing about new games. The split exists so that you can tune on the
80% only and watch the gap.

## Fitted rules

Some rules are not read from the card text. They are chosen because they lower the number of wrong actions on
this corpus, so they carry the same caveat as the holdout split: they are tuned on the data that measures
them. Rule files mark them `FITTED` in the `_why` field.

| Rule | What the engine assumes | Why |
| --- | --- | --- |
| `MEND_500` | The random enemy minion is the one with the highest health; the excess of a kill hits the enemy hero. | Of five target policies, this one had the fewest wrong actions (63, against 65 to 68). |
| `CATA_485` | The extra 1 damage to a random enemy minion hits the lowest-health minion, with no overflow. | The logged outcomes show it killing a 1-health minion. |
| `FIR_910` | The discard of a random Fire spell succeeds. | In the replays, nearly all wrong cases showed doubled damage and one card fewer in hand. |
| Held-back cards | The count applies only when the hand starts full. | Copies left over while the hand has room are stale, and counting them lowered agreement. |

## End-of-turn agreement

```sh
python tools/eot_check.py --replays DIR [--workers N]
```

For each of the owner's turns, the tool takes the position at the MAIN_END step and runs the engine's end of
turn: doomed minions die and "At the end of your turn" effects fire. It then compares both boards and both
heroes with the position the log shows when the turn passes.

The tool blames wrong turns on the cards that were on the board. A card's *lift* says how much more often it
is on the board when the result is wrong. The report goes to `.cache/results/eot_check.json`.

## What these numbers do not cover

- **The enemy's turn.** The tools check my actions and my end of turn. The enemy reply is a greedy heuristic
  for the search ([ARCHITECTURE.md](ARCHITECTURE.md#search-and-scoring)), and nothing here measures how close
  it comes to what opponents do.
- **Play quality.** Agreement says the engine knows what a move does, not whether the search picks good moves.
  Use self-play (`hsengine.selfplay`) to compare weights or search settings.
- **Hidden information.** The engine sees my hand and the public board. It scores unknown cards (draws,
  Discover, generated cards) by value instead of simulating them.
- **Cards outside the corpus.** 1,619 cards have real cases. The others have modeled or invariant cases,
  which pin the engine's behaviour without checking it against the game.
- **Game modes.** The engine models standard constructed one-on-one games only.
- **Other corpora.** All numbers come from one corpus. Other decks and seasons will give other numbers.
