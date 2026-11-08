"""Engine vs real games: replay every action of logged games through the engine and compare the results.

    python tools/sim_check.py --replays DIR [--logs GLOB] [--limit N] [--workers W] [--cache DIR] [--out DIR]
        [--holdout]

The game log is the ground truth. Before each of the player's own actions (a card played, an attack) the
position is rebuilt, the engine applies the same action, and its result is compared with the position the
game reported right after it: both boards (attack and health of every minion), both heroes' health, the hand
size. Mismatches are counted per card, worst first.

Inputs: --replays is a directory of replay archives (*.xml.zip, one `replay.xml` each); --logs a glob of
Power.log files. Parsed positions are cached per input file in --cache (default .cache/sim_check); a cache
entry is rebuilt when the input or the parsing code changes. tools/export_fixtures.py reads the same cache.

Outputs in --out (default .cache/results): sim_check.json (totals and the worst cards), sim_check_bad.jsonl
(every wrong action), sim_check_cards.json (per card: actions, wrong). Random effects (Discover, random
targets, draws) make some mismatches unavoidable: those cards are flagged and left out of the deterministic
rate. The deterministic rate also ignores the hand size, which generated cards make noisy.

--holdout also reports the two rates separately for a fixed 20% of the games (those whose id has
crc32(id) % 5 == 0) and for the other 80%. The split is a fixed function of the game id, so it is the same on
every run. The rules were tuned on all games, so the 20% group is not unseen data: see docs/ACCURACY.md.
"""

import argparse
import glob
import json
import multiprocessing as mp
import os
import pickle
import re
import sys
import time
import zlib
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "tools")]

BLOCK = re.compile(
    r"DebugPrintPower\(\) -( +)BLOCK_START BlockType=(PLAY|ATTACK) "
    r"Entity=(?:\[.*? id=(\d+) .*?\]|(\d+)) .*?Target=(?:\[.*? id=(\d+) .*?\]|(\d+))"
)
TRADE = re.compile(
    r"DebugPrintPower\(\) -( +)BLOCK_START BlockType=DECK_ACTION"
)  # a Tradeable swap ends the action before it
RANDOM = re.compile(r"random|Discover|Draw|draw|Shuffle|copy of|Copy|Transform", re.I)
_g: dict = {}


def _init(xml, cache_dir) -> None:
    from hsengine import cards

    _g["db"] = cards.load(xml)
    _g["cache"] = Path(cache_dir)


def collect(lines: list[str], main: int | None) -> list[tuple]:
    """Parse a game log into (position before, kind, entity, target, outcome after): the slow part, cached."""
    from hsengine.check import real_outcome
    from hsengine.state import StateTracker

    # the state tracker follows PowerTaskList in a live log, which runs behind GameState: take the actions
    # from the same stream. Converted replays only have GameState lines.
    stream = "GameState.DebugPrintPower"
    blocks = [m for ln in lines if stream in ln and (m := BLOCK.search(ln))]
    if not blocks:
        return []
    top = min(len(m.group(1)) for m in blocks)
    tr = StateTracker()
    out, pending = [], None

    def snap():
        if main is not None:
            tr.local_id = main
        tr.dirty = True
        return tr.snapshot()

    def close(post):
        nonlocal pending
        if pending is None:
            return
        pre, kind, ent, tgt = pending
        pending = None
        if post is None or not post.my_turn:
            return
        out.append((pre, kind, ent, tgt, real_outcome(post)))

    for ln in lines:
        m = BLOCK.search(ln) if stream in ln else None
        if m and len(m.group(1)) == top:
            st = snap()
            close(st)
            ent = int(m.group(3) or m.group(4))
            tgt = int(m.group(5) or m.group(6) or 0)
            if st and st.my_turn and st.main_action:
                mine = (
                    {e.eid for e in st.hand + st.board}
                    | {st.hero_id}
                    | ({st.hero_power.eid} if st.hero_power else set())
                )
                if ent in mine:
                    pending = (st, m.group(2), ent, tgt)
        elif pending is not None and stream in ln and (t := TRADE.search(ln)) and len(t.group(1)) == top:
            close(snap())
        elif (
            pending is not None
            and stream in ln
            and ("tag=STEP value=MAIN_END" in ln or "tag=STEP value=MAIN_NEXT" in ln)
        ):
            close(snap())
        tr.feed(ln)
        if main is not None:
            tr.local_id = main
    return out


# the parsed positions depend on these files only: change one and the cache is rebuilt
_CODE = [ROOT / "hsengine" / "state.py", ROOT / "hsengine" / "replayxml.py", Path(__file__)]


def _code_stamp() -> str:
    return "-".join(str(int(f.stat().st_mtime)) for f in _CODE if f.exists())


def events(kind: str, path: str, cache_dir: Path) -> list[tuple]:
    import cache

    src = Path(path)
    cache_dir.mkdir(parents=True, exist_ok=True)
    cf = cache_dir / (kind + "_" + src.parent.name[-12:] + "_" + src.name[:40] + ".pkl")
    stamp = (_code_stamp(), int(src.stat().st_mtime))
    if cf.exists():
        try:
            if pickle.loads(cf.read_bytes())[0] == stamp:
                return cache.load(cf)
        except Exception:
            pass
    if kind == "log":
        lines = src.read_text(encoding="utf-8", errors="replace").splitlines()
        ev = []
        starts = [i for i, ln in enumerate(lines) if "GameState.DebugPrintPower() - CREATE_GAME" in ln] + [len(lines)]
        for a, b in zip(starts, starts[1:], strict=False):
            ev += collect(lines[a:b], None)
    else:
        from hsengine import replayxml

        lines, main = replayxml.to_lines(path)
        ev = collect(lines, main)
    cf.write_bytes(pickle.dumps((stamp, ev)))
    return ev


def judge(evs: list[tuple], label: str) -> list[dict]:
    from hsengine import check
    from hsengine.planner import HERO

    db = _g["db"]
    out = []
    for pre, kind, ent, tgt, real in evs:
        rec = {
            "game": label,
            "turn": pre.turn,
            "kind": kind,
            "card": "",
            "name": "",
            "pre_board": [(e.atk, e.health) for e in pre.board],
            "pre_theirs": [(e.atk, e.health) for e in pre.enemy_board],
            "pre_hp": (pre.my_hp, pre.enemy_hp),
            "tgt": tgt,
        }
        try:
            case, p = check.case_from_state(pre, db, kind, ent, tgt)
            if kind == "ATTACK" and case.src == HERO:
                rec["card"], rec["name"] = "HERO:" + (pre.my_weapon_id or "-"), "hero attack"
            elif kind == "ATTACK":
                rec["card"] = p.cid.get(ent) or next((e.card_id for e in pre.board if e.eid == ent), "")
                rec["name"] = p.info[ent].name if ent in p.info else "?"
            else:
                rec["card"] = p.cid.get(ent) or (
                    pre.hero_power.card_id if pre.hero_power and pre.hero_power.eid == ent else ""
                )
                rec["name"] = p.info[ent].name if ent in p.info else "?"
            cands = case.candidates()
        except Exception as e:  # the engine itself failed on this position: counted as wrong
            rec["bad"] = ["engine_error:" + type(e).__name__]
            out.append(rec)
            continue
        if not cands:
            rec["bad"] = ["no_move"]
        else:
            best = min(cands, key=lambda c: len(check.diff(c, real)))
            rec["bad"] = check.diff(best, real)
            if rec["bad"]:
                rec["sim"], rec["real"] = best, real
        out.append(rec)
    return out


def _job(args):
    kind, path = args
    try:
        label = Path(path).parent.name if kind == "log" else Path(path).name[:8]
        return judge(events(kind, path, _g["cache"]), label)
    except Exception:
        return []


def _wrong(r) -> bool:
    return bool([d for d in r["bad"] if d != "hand"])  # hand size alone is noise: generated cards


def _parse_args(ap: argparse.ArgumentParser) -> argparse.Namespace:
    """Declare the command line options and parse them."""
    ap.add_argument("--replays", help="directory of replay archives (*.xml.zip)")
    ap.add_argument("--logs", help="glob of Power.log files")
    ap.add_argument("--limit", type=int, default=0, help="first N replays only")
    ap.add_argument("--workers", type=int, default=max(1, (os.cpu_count() or 4) - 2))
    ap.add_argument("--cards-xml", default=None, help="CardDefs.xml (default: found or downloaded by hearthstone)")
    ap.add_argument("--cache", default=str(ROOT / ".cache" / "sim_check"))
    ap.add_argument("--out", default=str(ROOT / ".cache" / "results"))
    ap.add_argument(
        "--holdout", action="store_true", help="also report a fixed 20%% of games apart from the other 80%%"
    )
    a = ap.parse_args()
    if not a.replays and not a.logs:
        ap.error("give --replays DIR and/or --logs GLOB")
    return a


def _list_jobs(a: argparse.Namespace) -> list:
    """The (kind, path) inputs: Power.log files first, then replay archives."""
    jobs = []
    if a.logs:
        jobs += [("log", p) for p in sorted(glob.glob(a.logs))]
    if a.replays:
        reps = sorted(glob.glob(os.path.join(a.replays, "*.xml.zip")))
        jobs += [("rep", p) for p in (reps[: a.limit] if a.limit else reps)]
    return jobs


def _run_jobs(a: argparse.Namespace, jobs: list) -> list:
    """Judge every input in a worker pool and return all action records."""
    t0 = time.time()
    recs = []
    with mp.Pool(a.workers, initializer=_init, initargs=(a.cards_xml, a.cache)) as pool:
        for k, r in enumerate(pool.imap_unordered(_job, jobs, chunksize=4)):
            recs += r
            if k % 200 == 0:
                print(f"[{time.time() - t0:5.0f}s] {k}/{len(jobs)} files, {len(recs)} actions", flush=True)
    return recs


def _is_random(db, cid) -> bool:
    """Whether the card text has a random effect."""
    return bool(RANDOM.search((db[cid].text or "") if cid in db else ""))


def _worst_cards(db, by: dict) -> list:
    """One row per card with wrong actions, worst first."""
    rows = []
    for (cid, name, kind), rs in by.items():
        bad = [r for r in rs if _wrong(r)]
        if not bad:
            continue
        rows.append(
            {
                "card": cid,
                "name": name,
                "kind": kind,
                "n": len(rs),
                "bad": len(bad),
                "random": _is_random(db, cid),
                "why": Counter(d for r in bad for d in r["bad"]).most_common(3),
                "example": next(
                    (
                        {"game": r["game"], "turn": r["turn"], "sim": r.get("sim"), "real": r.get("real")}
                        for r in bad
                        if "sim" in r
                    ),
                    None,
                ),
            }
        )
    rows.sort(key=lambda r: -r["bad"])
    return rows


def in_holdout(game: str) -> bool:
    """Whether a game belongs to the fixed 20% holdout group (by a checksum of its id)."""
    return zlib.crc32(game.encode()) % 5 == 0


def _rates(db, recs: list) -> tuple[int, float, int, float]:
    """(actions, exact rate, deterministic actions, deterministic rate) of a group of action records."""
    core = [r for r in recs if not _is_random(db, r["card"])]
    return (
        len(recs),
        sum(not r["bad"] for r in recs) / max(1, len(recs)),
        len(core),
        sum(not _wrong(r) for r in core) / max(1, len(core)),
    )


def _report_holdout(db, recs: list) -> None:
    """Print the rates of the holdout group and of the rest."""
    for name, group in (
        ("20% holdout", [r for r in recs if in_holdout(r["game"])]),
        ("other 80%", [r for r in recs if not in_holdout(r["game"])]),
    ):
        n, exact, dn, det = _rates(db, group)
        print(f"{name:12} {n} actions, exact {exact:.1%}; deterministic {det:.1%} of {dn}")


def main() -> None:
    a = _parse_args(argparse.ArgumentParser(description=__doc__.split("\n")[0]))
    from hsengine import cards

    db = cards.load(a.cards_xml)
    recs = _run_jobs(a, _list_jobs(a))
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    by = defaultdict(list)
    for r in recs:
        by[(r["card"], r["name"], r["kind"])].append(r)
    ok = sum(not r["bad"] for r in recs)
    with open(out / "sim_check_bad.jsonl", "w") as fh:
        for r in recs:
            if r["bad"]:
                fh.write(json.dumps(r) + "\n")
    core = [r for r in recs if not _is_random(db, r["card"])]
    core_ok = sum(not _wrong(r) for r in core)
    print(f"\n{len(recs)} actions, exact outcome {ok / max(1, len(recs)):.1%}")
    print(f"deterministic cards, boards and health only: {core_ok / max(1, len(core)):.1%} of {len(core)}")
    if a.holdout:
        _report_holdout(db, recs)
    rows = _worst_cards(db, by)
    for r in rows[:40]:
        flag = " (random)" if r["random"] else ""
        print(f"{r['bad']:5}/{r['n']:<5} {r['kind']:6} {r['name'][:28]:28} {r['card']:14} {r['why']}{flag}")
    per = {
        f"{cid}|{kind}": [len(rs), sum(1 for r in rs if _wrong(r)), name, _is_random(db, cid)]
        for (cid, name, kind), rs in by.items()
    }
    (out / "sim_check_cards.json").write_text(json.dumps(per))
    (out / "sim_check.json").write_text(
        json.dumps(
            {"actions": len(recs), "ok": ok, "deterministic": len(core), "deterministic_ok": core_ok, "cards": rows},
            indent=1,
        )
    )


if __name__ == "__main__":
    main()
