# Known gaps

1125 real cases in `tests/fixtures/cards` are known misses: the engine does not reproduce the logged outcome.
`tests/test_cards.py` marks them as strict `xfail`, so a fix that makes one pass fails the suite until you
regenerate its fixture. Each carries a reason of the form `cause: details`, written by
`tools/triage_xfail.py` into `tests/fixtures/cards/xfail_reasons.json`. The details name the dimensions that
differ (engine value against logged value) and the ops of the played card.

The triage sees only the stored engine context, so each cause is a best guess. Where the fixture cannot show
the real reason (a secret, a cost change), the cause names the likely one.

`tools/triage_xfail.py` writes this page. Change the template in that script, not this file.

## By cause

| Cause | Kind | Cases | What it means |
| --- | --- | --- | --- |
| `wrong_rule` | wrong rule | 199 | The modeled ops run but give a different board or health than the log (target choice or rule detail). |
| `unsimulated_effect` | hidden information | 188 | The card draws, generates or is only scored by a value op; the result depends on unseen cards. |
| `hand_hidden` | hidden information | 178 | Hand size differs for a card that draws, discovers or generates; the engine cannot see the cards. |
| `no_rule` | rule gap | 157 | The card has no effect modeled (rules exist only for the body or not at all). |
| `random_effect` | random effect | 107 | The card has a random target or random result; the log shows one outcome of many. |
| `free_play` | hidden information | 88 | Played with less mana than its cost: a cost change or a free cast the position does not show. |
| `hand_unmodeled` | rule gap | 86 | Hand size differs for a card with no hand effect in its rules (a generated or returned card is missing). |
| `face_prevented` | hidden information | 62 | The hero took less damage in the log: a secret, immunity or prevention the position does not show. |
| `trigger_mismatch` | rule gap | 28 | The card has a trigger, Deathrattle, aura or end-of-turn effect and the result differs from the log. |
| `hand_full` | held-back hand size | 16 | Hand size differs when the hand is nearly full: burnt or held-back cards depend on the hidden deck. |
| `no_legal_play` | fixture artifact | 11 | The engine offers no play of the card in the rebuilt position (target or condition not met). |
| `attacker_blocked` | fixture artifact | 5 | The logged attacker cannot attack in the rebuilt position, or the attack is not offered. |

## By kind

| Kind | Cases |
| --- | --- |
| hidden information | 516 |
| rule gap | 271 |
| wrong rule | 199 |
| random effect | 107 |
| fixture artifact | 16 |
| held-back hand size | 16 |

## What can be fixed here

Most misses need information the log shows only later (hand contents, secrets, random picks) or card text
that is not in the repository. The `no_rule`, `trigger_mismatch` and `wrong_rule` cases allow rule-level
fixes: add the missing ops to `hsengine/card_fx/*.json` ([RULES_FORMAT.md](RULES_FORMAT.md)), regenerate the
fixtures ([TESTING.md](TESTING.md#regenerating-the-fixtures)) and rerun `tools/triage_xfail.py`. Change a
rule only when the card's text and several logged actions agree, never to fit a single case.
