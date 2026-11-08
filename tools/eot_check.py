"""End-of-turn check: does the engine know what happens when the player ends the turn?

    python tools/eot_check.py --replays DIR [--limit N] [--workers W] [--out FILE]

For every turn of the replay's uploader: the position when they stop acting (the MAIN_END step), then the
engine's end of turn (doomed minions die, "At the end of your turn" effects, Frail Ghouls...), compared with
the position the game logged when the turn passed. Wrong turns are blamed on the cards on the board: how much
more often a card is on the board when the result is wrong ("lift"). Written to --out
(default .cache/results/eot_check.json).
"""

import argparse
import glob
import json
import multiprocessing as mp
import os
import sys
import time
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
DIMS = ("my_board", "enemy_board", "my_hp", "enemy_hp")
_g: dict = {}


def _init(xml) -> None:
    from hsengine import cards

    _g["db"] = cards.load(xml)


def _real(st) -> tuple:
    return (
        tuple(sorted((e.atk, e.health) for e in st.board)),
        tuple(sorted((e.atk, e.health) for e in st.enemy_board)),
        st.my_hp,
        st.enemy_hp,
    )


def _sim(s) -> tuple:
    return (
        tuple(sorted((u.atk - u.tmp, u.hp) for u in s.mine)),
        tuple(sorted((u.atk - u.tmp, u.hp) for u in s.theirs)),
        s.my_hp + s.my_armor,
        s.en_hp,
    )


def one(path: str) -> list:
    from hsengine import replayxml
    from hsengine.planner import Planner
    from hsengine.state import StateTracker

    db = _g["db"]
    try:
        lines, main = replayxml.to_lines(path)
    except Exception:
        return []
    tr = StateTracker()
    out, pending = [], None

    def snap():
        tr.local_id = main
        tr.dirty = True
        return tr.snapshot()

    for ln in lines:
        if "GameState.DebugPrintPower" in ln:
            if "tag=STEP value=MAIN_END" in ln:
                st = snap()
                if st and st.my_turn:
                    try:
                        p = Planner(st, db)
                        pending = (
                            _sim(p.end_of_turn(p.s0)),
                            sorted({e.card_id for e in st.board}),
                            sorted({e.card_id for e in st.enemy_board}),
                        )
                    except Exception:
                        pending = None
            elif pending is not None and "Entity=GameEntity tag=TURN value=" in ln:
                st = snap()
                if st:
                    real = _real(st)
                    simd, mine, theirs = pending
                    bad = [k for k, a, b in zip(DIMS, simd, real, strict=False) if a != b]
                    out.append({"bad": bad, "mine": mine, "theirs": theirs, "sim": simd, "real": real})
                pending = None
        tr.feed(ln)
        tr.local_id = main
    return out


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--replays", required=True, help="directory of replay archives (*.xml.zip)")
    ap.add_argument("--limit", type=int, default=0, help="first N replays only")
    ap.add_argument("--workers", type=int, default=max(1, (os.cpu_count() or 4) - 2))
    ap.add_argument("--cards-xml", default=None, help="CardDefs.xml (default: found or downloaded by hearthstone)")
    ap.add_argument("--out", default=str(ROOT / ".cache" / "results" / "eot_check.json"))
    a = ap.parse_args()
    from hsengine import cards, rules

    db = cards.load(a.cards_xml)
    files = sorted(glob.glob(os.path.join(a.replays, "*.xml.zip")))
    if a.limit:
        files = files[: a.limit]
    recs, t0 = [], time.time()
    with mp.Pool(a.workers, initializer=_init, initargs=(a.cards_xml,)) as pool:
        for k, r in enumerate(pool.imap_unordered(one, files, chunksize=4)):
            recs += r
            if k % 500 == 0:
                print(f"[{time.time() - t0:5.0f}s] {k}/{len(files)} replays, {len(recs)} turns", flush=True)
    bad = [r for r in recs if r["bad"]]
    print(f"\n{len(recs)} turn ends, engine right on {1 - len(bad) / max(1, len(recs)):.1%}")
    print("wrong dimensions:", Counter(d for r in bad for d in r["bad"]).most_common())
    on_bad, on_ok = Counter(), Counter()
    for r in recs:
        for c in r["mine"]:
            (on_bad if r["bad"] else on_ok)[c] += 1
        for c in r["theirs"]:
            (on_bad if r["bad"] else on_ok)["E:" + c] += 1
    nb, no = max(1, len(bad)), max(1, len(recs))
    blame = sorted(
        (((on_bad[c] / nb) / ((on_ok[c] + 1) / no)), on_bad[c], on_ok[c], c) for c in on_bad if on_bad[c] >= 15
    )[::-1]
    print("cards on the board when wrong (lift | wrong / right):")
    for lift, b, o, c in blame[:30]:
        cid = c[2:] if c.startswith("E:") else c
        card = db.get(cid)
        print(
            f"{lift:5.1f}x {b:4}/{o:<5} {'enemy' if c.startswith('E:') else 'mine '} {cid:14} "
            f"{(card.name if card else '?')[:24]:24} {rules.sim_text(card.text)[:70] if card else ''}"
        )
    out = Path(a.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({"turns": len(recs), "wrong": len(bad), "blame": blame[:200], "examples": bad[:300]}))


if __name__ == "__main__":
    main()
