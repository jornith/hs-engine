"""Build the per-card test fixtures in tests/fixtures/cards/ (one gzipped JSON Lines file per card-id prefix).

    python tools/export_fixtures.py --cache CACHE_DIR [--cards-xml CardDefs.xml] [--workers N] [--out DIR]

CACHE_DIR holds the parsed-position caches that tools/sim_check.py writes (rep_*.pkl / log_*.pkl). Every
playable card of the card database gets cases:

- real: logged actions of that card (up to 3 the engine gets exactly right, plus 1 it gets wrong, which the
  test marks xfail with the wrong dimensions as the reason). The expected outcome is what the game logged.
- modeled: no logged action; the card is played into a fixed synthetic position and every engine result of
  that play is recorded (a snapshot of the compiled rule).
- invariant: as modeled, for cards with random effects; only bounds are asserted.

A case stores the engine context (card models already compiled: no card text), the action and numbers only.
Names are replaced by per-case tokens, entity ids are kept (they are game-local numbers), games are numbered.
"""

import argparse
import dataclasses
import glob
import gzip
import json
import multiprocessing as mp
import os
import re
import sys
import time
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "tools")]

from hearthstone.enums import GameTag  # noqa: E402

from hsengine import cards, check  # noqa: E402
from hsengine.planner import HERO, Planner  # noqa: E402
from hsengine.state import KEYWORDS, Ent, GameState  # noqa: E402

RANDOM = re.compile(r"random|Discover|Draw|draw|Shuffle|copy of|Copy|Transform", re.I)
PLAYABLE = ("MINION", "SPELL", "WEAPON", "HERO", "HERO_POWER", "LOCATION")
KEEP_OK, KEEP_MISS = 3, 1
_g: dict = {}


def _init(xml) -> None:
    _g["db"] = cards.load(xml)


def anonymize(ctx: dict) -> dict:
    """Card names are only compared for equality by the engine: replace them by tokens."""
    names: dict = {}
    infos = []
    for i in ctx["infos"]:
        i = dict(i)
        i["name"] = names.setdefault(i["name"], f"n{len(names)}")
        infos.append(i)
    return {**ctx, "infos": infos}


def _actor(st: GameState, p: Planner, kind: str, ent: int) -> str:
    if kind == "ATTACK":
        if ent == st.hero_id:
            return st.my_weapon_id or ""
        return next((e.card_id for e in st.board if e.eid == ent), "")
    if ent in p.cid:
        return p.cid[ent]
    if st.hero_power and st.hero_power.eid == ent:
        return st.hero_power.card_id
    return next((e.card_id for e in st.locations if e.eid == ent), "")


def _job(args):
    """All logged actions of one cache file: (index, [(card, ok, wrong dims, size, case)])."""
    import cache

    k, path = args
    db = _g["db"]
    out = []
    try:
        events = cache.load(path)
    except Exception:
        return k, []
    for pre, kind, ent, tgt, real in events:
        try:
            case, p = check.case_from_state(pre, db, kind, ent, tgt)
            bad = case.judge(real)
            card = _actor(pre, p, kind, ent)
        except Exception as e:  # counted as wrong, as sim_check.py does; no case can be stored
            out.append(("", False, ["engine_error:" + type(e).__name__], 0, {"src": None}))
            continue
        if bad is None:
            bad = ["no_move"]  # the engine does not offer this action
        # a fixture does not store the held-back cards: its status is what it reproduces without them
        stored = bad
        if case.held_back:
            stored = dataclasses.replace(case, held_back=0).judge(real)
            stored = ["no_move"] if stored is None else stored
        rec = {
            "card": card,
            "origin": "real",
            "game": k,
            "turn": pre.turn,
            "kind": kind,
            "src": case.src,
            "tgt": case.tgt,
            "hand": case.hand_ids,
            "ctx": anonymize(case.ctx),
            "expect": real,
            "status": "ok" if not stored else "miss",
        }
        if stored:
            rec["why"] = stored
        out.append((card, not bad, bad, len(json.dumps(rec["ctx"])), rec))
    return k, out


# ------------------------------------------------ the synthetic position for cards without a logged action


def _ent(eid: int, cid: str, db: dict, xml: dict, can_attack: bool = False) -> Ent:
    c = db[cid]
    tags = xml[cid].tags if cid in xml else {}
    kw = frozenset(name for tag, name in KEYWORDS.items() if tags.get(tag, 0))
    return Ent(
        eid,
        cid,
        c.cost,
        c.atk,
        c.health,
        "taunt" in kw,
        can_attack,
        "divine" in kw,
        "stealth" in kw,
        1 if can_attack else 0,
        True,
        c.health,
        kw,
        int(tags.get(GameTag.SPELLPOWER, 0)),
        False,
        (),
    )


def synthetic(cid: str, db: dict, xml: dict) -> tuple[GameState, int]:
    """Turn 15, 10 mana, 30/30 heroes; my 2/3 and 4/5, their 6/7 and 3/2; the card in hand (or as my hero
    power). Returns the state and the entity id of the card."""
    st = GameState(
        turn=15,
        my_turn=True,
        mana=10,
        max_mana=10,
        my_hp=30,
        my_max_hp=30,
        enemy_hp=30,
        enemy_hand_size=5,
        main_action=True,
        hero_id=64,
        my_hero="HERO_08",
        enemy_max_mana=10,
    )
    st.board = [_ent(10, "CS2_120", db, xml, True), _ent(11, "CS2_182", db, xml, True)]
    st.enemy_board = [_ent(12, "CS2_200", db, xml), _ent(13, "CS2_172", db, xml)]
    e = _ent(20, cid, db, xml)
    if db[cid].type == "HERO_POWER":
        st.hero_power = e
    else:
        st.hand = [e]
    return st, 20


def modeled(cid: str, db: dict, xml: dict) -> dict:
    st, uid = synthetic(cid, db, xml)
    p = Planner(st, db)
    ctx = dict(p._rust_ctx())
    ctx.pop("w", None)
    case = check.Case(ctx, "PLAY", uid, None, sorted(p.cid))
    moves = [list(m) for m in case.plays()]
    rnd = bool(RANDOM.search(db[cid].text or ""))
    rec = {
        "card": cid,
        "origin": "invariant" if rnd else "modeled",
        "kind": "PLAY",
        "src": uid,
        "tgt": None,
        "hand": case.hand_ids,
        "ctx": anonymize(ctx),
        "max_hp": 30,
    }
    if rnd:
        rec["n_moves"] = len(moves)
    else:
        rec["moves"] = moves
    return rec


def _prefix(cid: str) -> str:
    head = cid.split("_")[0] if "_" in cid else re.sub(r"\d+.*$", "", cid)
    return re.sub(r"[^A-Za-z0-9]", "", head).upper() or "MISC"


def _parse_args(ap: argparse.ArgumentParser) -> argparse.Namespace:
    """Declare the command line options and parse them."""
    ap.add_argument("--cache", required=True, help="directory with sim_check.py position caches (*.pkl)")
    ap.add_argument("--cards-xml", default=None, help="CardDefs.xml (default: found or downloaded by hearthstone)")
    ap.add_argument("--out", default=str(ROOT / "tests" / "fixtures" / "cards"))
    ap.add_argument("--workers", type=int, default=max(1, (os.cpu_count() or 4) - 2))
    ap.add_argument("--limit", type=int, default=0, help="read only the first N cache files")
    return ap.parse_args()


def _scan_cache(a: argparse.Namespace, db: dict, files: list, t0: float) -> tuple:
    """Replay every cached action. Returns the best kept records per card (matching, mismatching) and
    the corpus counters (actions, exact, deterministic, deterministic exact)."""
    ok_keep: dict = defaultdict(list)  # card -> [(size, rec)]
    miss_keep: dict = defaultdict(list)
    total = exact = core = core_ok = 0
    with mp.Pool(a.workers, initializer=_init, initargs=(a.cards_xml,)) as pool:
        for n, (k, recs) in enumerate(pool.imap_unordered(_job, list(enumerate(files)), chunksize=8)):
            for card, ok, bad, size, rec in recs:
                total += 1
                exact += ok
                text = db[card].text if card in db and rec["src"] != HERO else ""  # hero attacks: not random
                if not RANDOM.search(text or ""):
                    core += 1
                    core_ok += not [d for d in bad if d != "hand"]
                if not card or card not in db:
                    continue
                kept = rec["status"] == "ok"
                keep = ok_keep if kept else miss_keep
                lim = KEEP_OK if kept else KEEP_MISS
                keep[card].append((size, k, rec))
                keep[card].sort(key=lambda x: (x[0], x[1]))
                del keep[card][lim:]
            if n % 300 == 0:
                print(f"[{time.time() - t0:5.0f}s] {n}/{len(files)} files, {total} actions", flush=True)
    return ok_keep, miss_keep, total, exact, core, core_ok


def _build_cases(db: dict, xml, ok_keep: dict, miss_keep: dict) -> tuple:
    """Fixture records grouped by card prefix: real ones where the corpus has them, modeled ones otherwise.
    Returns (by_prefix, counts, playable)."""
    by_prefix: dict = defaultdict(list)
    counts = defaultdict(int)
    playable = sorted(c for c, v in db.items() if v.type in PLAYABLE)
    for cid in playable:
        real = [r for _, _, r in ok_keep.get(cid, [])] + [r for _, _, r in miss_keep.get(cid, [])]
        if real:
            for j, r in enumerate(real):
                r["id"] = f"{cid}-real-{'miss' if r['status'] == 'miss' else j}"
                counts["real_" + r["status"]] += 1
                by_prefix[_prefix(cid)].append(r)
        else:
            try:
                r = modeled(cid, db, xml)
            except Exception as e:  # a card model that cannot be built: recorded, so it is visible
                r = {"card": cid, "origin": "error", "error": type(e).__name__}
            r["id"] = f"{cid}-{r['origin']}"
            counts[r["origin"]] += 1
            by_prefix[_prefix(cid)].append(r)
    return by_prefix, counts, playable


def _write_cases(out: Path, by_prefix: dict) -> None:
    """Replace the fixture files in `out` with one gzipped JSON-lines file per prefix."""
    out.mkdir(parents=True, exist_ok=True)
    for f in out.glob("*.jsonl.gz"):
        f.unlink()
    for pre, recs in sorted(by_prefix.items()):
        with gzip.open(out / f"{pre}.jsonl.gz", "wt", compresslevel=9) as fh:
            for r in recs:
                fh.write(json.dumps(r, separators=(",", ":")) + "\n")


def _manifest(counts: dict, playable: list, ok_keep: dict, miss_keep: dict, files: list, stats: tuple) -> dict:
    """The manifest.json content: fixture counts and the corpus agreement."""
    total, exact, core, core_ok = stats
    real_cases = counts["real_ok"] + counts["real_miss"]
    return {
        "cards": len(playable),
        "cards_real": sum(1 for c in playable if c in ok_keep or c in miss_keep),
        "cases": dict(counts),
        "fixture_agreement": round(counts["real_ok"] / max(1, real_cases), 4),
        "corpus": {
            "files": len(files),
            "actions": total,
            "exact": exact,
            "exact_rate": round(exact / max(1, total), 4),
            "deterministic": core,
            "deterministic_ok": core_ok,
            "deterministic_rate": round(core_ok / max(1, core), 4),
        },
    }


def main() -> None:
    a = _parse_args(argparse.ArgumentParser(description=__doc__.split("\n")[0]))
    from hearthstone import cardxml

    db = cards.load(a.cards_xml)
    xml, _ = cardxml.load(a.cards_xml) if a.cards_xml else cardxml.load()
    files = sorted(glob.glob(os.path.join(a.cache, "*.pkl")))
    if a.limit:
        files = files[: a.limit]
    t0 = time.time()
    ok_keep, miss_keep, *stats = _scan_cache(a, db, files, t0)
    by_prefix, counts, playable = _build_cases(db, xml, ok_keep, miss_keep)
    out = Path(a.out)
    _write_cases(out, by_prefix)
    manifest = _manifest(counts, playable, ok_keep, miss_keep, files, tuple(stats))
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1) + "\n")
    print(json.dumps(manifest, indent=1))
    print(f"{time.time() - t0:.0f}s")


if __name__ == "__main__":
    main()
