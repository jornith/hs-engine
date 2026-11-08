"""Package-level checks: the missing-card-data error, a golden search result and a self-play smoke run.

The golden fixture (tests/fixtures/golden.json) records, for one fixed position, every result of playing one
card and the line the search picks under a node limit. Regenerate it with HSENGINE_UPDATE_GOLDEN=1 after a
deliberate engine change, and say why in the commit.
"""

import json
import os
from pathlib import Path

import pytest
from synth import DB, card, minion, power, state

from hsengine import cards, check, selfplay
from hsengine.planner import Planner

GOLDEN = Path(__file__).parent / "fixtures" / "golden.json"


def test_missing_card_data_raises_card_data_error(tmp_path):
    with pytest.raises(cards.CardDataError, match="docs/DATA.md"):
        cards.load(str(tmp_path / "missing" / "CardDefs.xml"))


def test_race_masks():
    fish = DB["T_FISH"]
    assert cards.race_mask(fish) == cards.race_bit("MURLOC") != 0
    assert cards.race_mask(DB["T_CHIMERA"]) == cards.race_bit("BEAST") | cards.race_bit("MURLOC")
    assert cards.race_mask(DB["T_STORM"]) == 0 and cards.race_mask(None) == 0
    assert cards.race_bit("ALL") & cards.race_bit("DRAGON") != 0
    assert cards.race_bit("SOMETHING_NEW") == cards.race_bit("ANOTHER_ONE") != 0


def _golden_position():
    return state(
        mana=4,
        my_hp=18,
        enemy_hp=14,
        hand=[card("T_HOLY", eid=20), card("T_ARCANE", eid=21), card("T_RAPTOR", eid=22)],
        board=[minion("T_YETI", 4, 5, eid=10), minion("T_BOAR", 2, 1, eid=11)],
        enemy_board=[
            minion("T_GUARD", 3, 5, kw=("taunt",), mine=False, eid=12),
            minion("T_RAPTOR", 3, 2, mine=False, eid=13),
        ],
        hero_power=power("T_BP_PING", eid=30),
    )


def _golden_now() -> dict:
    p = Planner(_golden_position(), DB)
    ctx = dict(p._rust_ctx())
    ctx.pop("w", None)
    case = check.Case(ctx, "PLAY", 20, None, sorted(p.cid))
    _, path, final = p.search(60.0, 40, 3000)
    return {
        "plays": json.loads(json.dumps(case.plays())),
        "line": [list(a) for a, _ in path],
        "outcome": json.loads(json.dumps(check.outcome(p._d(final), set(p.cid)))),
    }


def test_golden_position():
    now = _golden_now()
    if os.environ.get("HSENGINE_UPDATE_GOLDEN"):
        GOLDEN.write_text(json.dumps(now, indent=1) + "\n")
    assert now == json.loads(GOLDEN.read_text())


def test_selfplay_smoke():
    decks = [
        selfplay.Deck(["T_BOAR", "T_RAPTOR", "T_YETI", "T_ARCANE", "T_HOLY"] * 6, "T_BP_PING"),
        selfplay.Deck(["T_GUARD", "T_RIDER", "T_OGRE", "T_KILL", "T_LEADER"] * 6, "T_BP_ARMOR"),
    ]
    a = selfplay.run(decks, DB, n_games=4, seed=3, max_nodes=300)
    b = selfplay.run(decks, DB, n_games=4, seed=3, max_nodes=300)
    assert len(a["results"]) == 4
    assert all(r[2] in (-1, 0, 1) and r[3] > 0 for r in a["results"])
    assert a["results"] == b["results"]
