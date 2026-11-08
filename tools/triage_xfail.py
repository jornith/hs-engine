"""Sort the known misses of the per-card suite by cause.

    python tools/triage_xfail.py            # writes tests/fixtures/cards/xfail_reasons.json and docs/KNOWN_GAPS.md

Every real case the engine gets wrong (`status == "miss"` in tests/fixtures/cards) gets a cause and a reason
string. The cause comes from the stored engine context only: the ops of the played card, the position, and the
dimensions where the closest engine result differs from the log. The rules are heuristics, and the
reasons say "likely" where the fixture cannot prove the cause.
"""

import gzip
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
from hsengine import check  # noqa: E402

FIX = ROOT / "tests" / "fixtures" / "cards"
HAND_OPS = {"draw", "gen", "discover", "value", "hand_buff", "mana", "opp_draw", "shuffle_self", "summon_copies"}
RANDOM_OPS = {"dmg_random", "dmg_random_minions", "dmg_chain", "dmg_split", "discover", "random_summon"}

# cause -> (kind, what it means)
CAUSES = {
    "free_play": (
        "hidden information",
        "Played with less mana than its cost: a cost change or a free cast the position does not show.",
    ),
    "no_legal_play": (
        "fixture artifact",
        "The engine offers no play of the card in the rebuilt position (target or condition not met).",
    ),
    "attacker_blocked": (
        "fixture artifact",
        "The logged attacker cannot attack in the rebuilt position, or the attack is not offered.",
    ),
    "hand_hidden": (
        "hidden information",
        "Hand size differs for a card that draws, discovers or generates; the engine cannot see the cards.",
    ),
    "hand_full": (
        "held-back hand size",
        "Hand size differs when the hand is nearly full: burnt or held-back cards depend on the hidden deck.",
    ),
    "hand_unmodeled": (
        "rule gap",
        "Hand size differs for a card with no hand effect in its rules (a generated or returned card is missing).",
    ),
    "random_effect": (
        "random effect",
        "The card has a random target or random result; the log shows one outcome of many.",
    ),
    "no_rule": ("rule gap", "The card has no effect modeled (rules exist only for the body or not at all)."),
    "unsimulated_effect": (
        "hidden information",
        "The card draws, generates or is only scored by a value op; the result depends on unseen cards.",
    ),
    "face_prevented": (
        "hidden information",
        "The hero took less damage in the log: a secret, immunity or prevention the position does not show.",
    ),
    "trigger_mismatch": (
        "rule gap",
        "The card has a trigger, Deathrattle, aura or end-of-turn effect and the result differs from the log.",
    ),
    "wrong_rule": (
        "wrong rule",
        "The modeled ops run but give a different board or health than the log (target choice or rule detail).",
    ),
}


def _load() -> list[dict]:
    out = []
    for f in sorted(FIX.glob("*.jsonl.gz")):
        with gzip.open(f, "rt") as fh:
            out += [json.loads(line) for line in fh]
    return out


def _ops(info) -> list[str]:
    if not info:
        return []
    names = []
    for fx, _unk in info["modes"]:
        names += [f[0] for f in fx]
    return list(dict.fromkeys(names))


def _delta(dim: str, best: tuple, exp: tuple) -> str:
    i = check.DIMS.index(dim)
    if dim.endswith("board"):
        a, b = len(best[i]), len(exp[i])
        return f"{dim} {a} vs {b} minions" if a != b else f"{dim} stats differ"
    return f"{dim} {best[i]} vs {exp[i]}"


def classify(r: dict) -> tuple[str, str]:
    c = check.case_from_json(r)
    s0 = c.ctx["s0"]
    info = {i["uid"]: i for i in c.ctx["infos"]}.get(r["src"])
    ops = _ops(info)
    unk = bool(info) and any(u for _fx, u in info["modes"])
    tag = ops and "ops " + ",".join(ops[:4]) or "no ops"
    cands = c.candidates()
    if not cands:
        if r["kind"] == "ATTACK":
            return "attacker_blocked", f"no engine attack by this unit on the logged target; {tag}"
        if info and r["src"] in s0["hand"] and info["cost"] > s0["mana"]:
            return "free_play", f"cost {info['cost']} with {s0['mana']} mana in the position; {tag}"
        return (
            "no_legal_play",
            f"no engine play of the card (kind {info['kind'] if info else '?'}, mana {s0['mana']}); {tag}",
        )
    e = r["expect"]
    exp = (tuple(map(tuple, e[0])), tuple(map(tuple, e[1])), e[2], e[3], e[4])
    best = min(cands, key=lambda x: len(check.diff(x, exp)))
    dims = check.diff(best, exp)
    det = "; ".join(_delta(d, best, exp) for d in dims)
    if dims == ["hand"]:
        if set(ops) & HAND_OPS or unk:
            return "hand_hidden", f"{det}; {tag}"
        if len(r["hand"]) >= 9:
            return "hand_full", f"{det}; hand of {len(r['hand'])} cards, {tag}"
        return "hand_unmodeled", f"{det}; {tag}"
    if unk or set(ops) & RANDOM_OPS:
        return "random_effect", f"{det}; {tag}"
    if set(ops) & (HAND_OPS | {"value"}):
        return "unsimulated_effect", f"{det}; {tag}"
    if r["kind"] == "ATTACK" and "enemy_hp" in dims and best[3] < exp[3] and not ops:
        return "face_prevented", det
    if info and (info["trig"] or info["drs"] or info["aura"] or info["eot"]):
        return "trigger_mismatch", f"{det}; {tag}"
    if not ops:
        return "no_rule", f"{det}; {tag}"
    return "wrong_rule", f"{det}; {tag}"


def main() -> None:
    misses = [r for r in _load() if r.get("status") == "miss"]
    reasons, by_cause, cards = {}, Counter(), defaultdict(Counter)
    for r in misses:
        cause, detail = classify(r)
        reasons[r["id"]] = f"{cause}: {detail}"
        by_cause[cause] += 1
        cards[cause][r["card"]] += 1
    (FIX / "xfail_reasons.json").write_text(json.dumps(reasons, indent=0, sort_keys=True) + "\n")
    rows = ["| Cause | Kind | Cases | What it means |", "| --- | --- | --- | --- |"]
    for cause, n in by_cause.most_common():
        kind, what = CAUSES[cause]
        rows.append(f"| `{cause}` | {kind} | {n} | {what} |")
    total = sum(by_cause.values())
    kinds = Counter()
    for cause, n in by_cause.items():
        kinds[CAUSES[cause][0]] += n
    krows = ["| Kind | Cases |", "| --- | --- |"] + [f"| {k} | {n} |" for k, n in kinds.most_common()]
    (ROOT / "docs" / "KNOWN_GAPS.md").write_text(
        TEMPLATE.format(total=total, causes="\n".join(rows), kinds="\n".join(krows))
    )
    print(total, dict(by_cause))


TEMPLATE = """# Known gaps

1125 real cases in `tests/fixtures/cards` are known misses: the engine does not reproduce the logged outcome.
`tests/test_cards.py` marks them as strict `xfail`, so a fix that makes one pass fails the suite until you
regenerate its fixture. Each carries a reason of the form `cause: details`, written by
`tools/triage_xfail.py` into `tests/fixtures/cards/xfail_reasons.json`. The details name the dimensions that
differ (engine value against logged value) and the ops of the played card.

The triage sees only the stored engine context, so each cause is a best guess. Where the fixture cannot show
the real reason (a secret, a cost change), the cause names the likely one.

`tools/triage_xfail.py` writes this page. Change the template in that script, not this file.

## By cause

{causes}

## By kind

{kinds}

## What can be fixed here

Most misses need information the log shows only later (hand contents, secrets, random picks) or card text
that is not in the repository. The `no_rule`, `trigger_mismatch` and `wrong_rule` cases allow rule-level
fixes: add the missing ops to `hsengine/card_fx/*.json` ([RULES_FORMAT.md](RULES_FORMAT.md)), regenerate the
fixtures ([TESTING.md](TESTING.md#regenerating-the-fixtures)) and rerun `tools/triage_xfail.py`. Change a
rule only when the card's text and several logged actions agree, never to fit a single case.
"""

if __name__ == "__main__":
    main()
