"""Per-card regression suite: every playable card of the card database has cases in tests/fixtures/cards/.

real       a logged action of the card; the engine must reproduce the logged outcome exactly. Known misses
           are strict xfail; the reason is the cause and the wrong dimensions (xfail_reasons.json, docs/KNOWN_GAPS.md).
modeled    the card played into a fixed synthetic position; every engine result must match the snapshot.
invariant  as modeled, for cards with random effects: only the number of moves and bounds are checked.

The fixtures hold compiled card models and numbers only; no card database is needed. Regenerate them with
tools/export_fixtures.py (docs/ACCURACY.md).
"""

import gzip
import json
from pathlib import Path

import pytest

from hsengine import check

FIX = Path(__file__).parent / "fixtures" / "cards"
MANIFEST = json.loads((FIX / "manifest.json").read_text())
REASONS = json.loads((FIX / "xfail_reasons.json").read_text())


def _load() -> list[dict]:
    out = []
    for f in sorted(FIX.glob("*.jsonl.gz")):
        with gzip.open(f, "rt") as fh:
            out += [json.loads(line) for line in fh]
    return out


CASES = _load()
REAL = [r for r in CASES if r["origin"] == "real"]


def _norm(x):
    return json.loads(json.dumps(x))


def _param(r: dict):
    marks = ()
    if r.get("status") == "miss":
        marks = pytest.mark.xfail(reason=REASONS.get(r["id"], "known miss: " + ", ".join(r["why"])), strict=True)
    elif r["origin"] == "error":
        marks = pytest.mark.xfail(reason="card model failed: " + r["error"], strict=True)
    return pytest.param(r, id=r["id"], marks=marks)


def _real_ok(r: dict) -> bool:
    cands = check.case_from_json(r).candidates()
    return _norm(r["expect"]) in [_norm(c) for c in cands]


@pytest.mark.parametrize("rec", [_param(r) for r in CASES])
def test_card(rec):
    origin = rec["origin"]
    if origin == "real":
        assert _real_ok(rec), f"logged outcome {rec['expect']} not among the engine results"
        return
    assert origin in ("modeled", "invariant"), rec.get("error")
    plays = check.case_from_json(rec).plays()
    if origin == "modeled":
        assert _norm([list(p) for p in plays]) == rec["moves"]
        return
    assert len(plays) == rec["n_moves"]
    for _, _, d in plays:
        assert len(d["mine"]) <= 7 and len(d["theirs"]) <= 7
        assert d["mana"] >= 0
        assert d["my_hp"] <= rec["max_hp"] and d["en_hp"] <= rec["max_hp"]
        # the played card itself may have 0 printed Health (some non-collectible cards do)
        assert all(u[2] > 0 for u in d["mine"] + d["theirs"] if u[0] != rec["src"])
        assert all(u[1] >= 0 for u in d["mine"] + d["theirs"])


def test_real_agreement():
    """The share of real cases the engine reproduces must not drop below the value recorded at export."""
    ok = sum(_real_ok(r) for r in REAL)
    assert len(REAL) == MANIFEST["cases"]["real_ok"] + MANIFEST["cases"]["real_miss"]
    assert ok >= MANIFEST["cases"]["real_ok"], f"{ok}/{len(REAL)}, recorded {MANIFEST['fixture_agreement']:.2%}"


def test_coverage():
    """Every playable card has at least one case."""
    assert len({r["card"] for r in CASES}) == MANIFEST["cards"]


def test_every_miss_has_a_reason():
    """tools/triage_xfail.py writes one reason per known miss; rerun it after regenerating the fixtures."""
    miss = {r["id"] for r in CASES if r.get("status") == "miss"}
    assert miss == set(REASONS)
    assert len(miss) == MANIFEST["cases"]["real_miss"]
