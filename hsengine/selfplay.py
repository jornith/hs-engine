"""Whole games between decks, played by the Rust core's turn search with the hand-tuned weights (`W`).

    from hsengine import cards, selfplay
    db = cards.load()
    decks = [selfplay.Deck(["CS2_120"] * 30, "HERO_08bp"), ...]
    out = selfplay.run(decks, db, n_games=100)

The core plays both sides: draws, mana, fatigue, a simple mulligan (cards costing 4 or more go back), hidden
hands. `table` compiles every card of the decks once into the core's card table.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

from . import _core, rules
from .planner import Planner, _kwbits
from .state import Ent
from .weights import W

POWERS = {
    "WARRIOR": "HERO_01bp",
    "SHAMAN": "HERO_02bp",
    "ROGUE": "HERO_03bp",
    "PALADIN": "HERO_04bp",
    "HUNTER": "HERO_05bp",
    "DRUID": "HERO_06bp",
    "WARLOCK": "HERO_07bp",
    "MAGE": "HERO_08bp",
    "PRIEST": "HERO_09bp",
    "DEMONHUNTER": "HERO_10bp",
    "DEATHKNIGHT": "HERO_11bp",
}
COIN = "GAME_005"
_KW = (
    ("taunt", "Taunt"),
    ("divine", "Divine Shield"),
    ("charge", "Charge"),
    ("rush", "Rush"),
    ("windfury", "Windfury"),
    ("stealth", "Stealth"),
    ("poisonous", "Poisonous"),
    ("lifesteal", "Lifesteal"),
    ("deathrattle", "Deathrattle"),
    ("reborn", "Reborn"),
)
_EV = {"hero_attack": 1, "spell": 2, "minion_play": 3, "power": 4, "draw": 5, "self_attack": 6}


@dataclass
class Deck:
    cards: list[str]  # 30 card ids
    hero_power: str  # e.g. POWERS["MAGE"]


def _card(cid: str, c) -> Ent:
    """A card as it enters a hand: printed stats, keywords read from the text before Battlecry/Deathrattle."""
    txt = rules.norm(c.text)
    head = txt.split("Battlecry")[0].split("Deathrattle")[0]
    kw = frozenset(k for k, w in _KW if w in (txt if k == "deathrattle" else head))
    sp = int(m[1]) if (m := re.search(r"Spell Damage \+(\d+)", txt)) else 0
    return Ent(
        0,
        cid,
        c.cost,
        c.atk,
        c.health,
        "taunt" in kw,
        False,
        "divine" in kw,
        "stealth" in kw,
        0,
        True,
        c.health,
        kw,
        sp,
    )


def _fx(f) -> tuple:
    return (f.op, int(f.n), int(f.a), int(f.h), _kwbits(f.kw), f.tgt or "", f.cond or "", f.scope or "")


def table(decks: list[Deck], db: dict, extra_ids=()) -> dict:
    """The self-play context: card models of every card in the decks (plus the Coin and the hero powers)."""
    ids = sorted(
        ({c for d in decks for c in d.cards} | {COIN} | {d.hero_power for d in decks} | set(extra_ids)) & set(db)
    )
    missing = sorted({c for d in decks for c in d.cards + [d.hero_power]} - set(db))
    if missing:
        raise KeyError(f"cards not in the card database: {missing[:10]}")
    index = {c: k for k, c in enumerate(ids)}
    infos, cards = [], []
    for k, cid in enumerate(ids):
        c = db[cid]
        power = c.type == "HERO_POWER"
        e = _card(cid, c) if not power else Ent(0, cid, c.cost, 0, 0, ready=True)
        i = rules.card_model(e, c, power=power)
        txt = rules.sim_text(c.text)
        infos.append(_info(k, cid, i))
        cards.append(_card_entry(k, cid, c, e, i, power, txt))
    ctx = _base_ctx()
    ctx.update(
        infos=infos,
        cards=cards,
        coin=index[COIN],
        decks=[([index[c] for c in d.cards], index[d.hero_power], d.hero_power.startswith("HERO_11")) for d in decks],
        index=index,
    )
    return ctx


def _info(k: int, cid: str, i) -> dict:
    """The core's card model of card number `k`."""
    return {
        "uid": k,
        "name": i.name,
        "kind": i.kind,
        "cost": int(i.cost),
        "atk": int(i.atk),
        "hp": int(i.hp),
        "kw": _kwbits(i.kw),
        "sp": int(i.sp or 0),
        "modes": [([_fx(f) for f in m], bool(u)) for m, u in (i.modes or ())],
        "dr": float(i.dr or 0),
        "aura": (int(i.aura[0]), int(i.aura[1])) if i.aura else None,
        "overload": int(i.overload or 0),
        "drs": (int(i.drs[0]), int(i.drs[1]), int(i.drs[2]), _kwbits(i.drs[3])) if i.drs else None,
        "eot": [_fx(f) for f in (i.eot or ())],
        "pv": float(i.pv or 0),
        "cannon": cid == "JAIL_450",
        "trig": [
            (_EV[a], [_fx(f) for f in b], 1 if fl == "battlecry" else 0) for a, b, fl in (i.trig or ()) if a in _EV
        ],
        "wid": k + 1 if i.kind == "WEAPON" else 0,
    }


def _card_entry(k: int, cid: str, c, e: Ent, i, power: bool, txt: str) -> dict:
    """The per-card flags of card number `k`: hold value, special cards, weapon trigger, hero power reach."""
    wt = None
    if c.type == "WEAPON" and cid not in rules._TRIG_SPECIAL:
        t = [b for a, b, _ in rules._triggers(rules.norm(c.text or "")) if a == "hero_attack"]
        wt = [_fx(f) for f in t[0]] if t else None
    direct = swing = 0
    if power:
        for f in i.modes[0][0] if i.modes else ():
            if f.op in ("face", "dmg"):
                direct += f.n
            elif f.op == "hero_atk":
                swing = f.n
            elif f.op == "weapon":
                swing = f.a
    return {
        "uid": k,
        "ekw": _kwbits(e.kw),
        "hold": float(Planner._hold(i)),
        "corpse_cost": "Corpses</b> instead" in (c.text or ""),
        "death_discount": "less for each minion that died this turn" in txt,
        "dormant": c.type == "MINION" and "Dormant" in (c.text or ""),
        "tower": cid == "JAIL_440",
        "maiev": cid == "JAIL_850",
        "nozdormu": cid == "CATA_473",
        "thoret": cid == "EDR_258",
        "wlife": c.type == "WEAPON" and bool(re.match(r"\s*Lifesteal", txt)),
        "wtrig": wt,
        "p_direct": direct,
        "p_swing": swing,
        "cheap": c.type == "MINION" and c.cost == 1,
    }


def _base_ctx() -> dict:
    """The context fields that are the same for every table (a fresh dict each call)."""
    empty_s = {
        "mana": 0,
        "hand": [],
        "mine": [],
        "theirs": [],
        "my_hp": 30,
        "my_armor": 0,
        "en_hp": 30,
        "hero_atk": 0,
        "hero_att": 0,
        "bonus": 0.0,
        "watk": 0,
        "wdur": 0,
        "corpses": 0,
        "died": 0,
        "cannon": False,
        "drawn": 0,
        "played": 0,
        "wid": 0,
    }
    return {
        "w": {k: float(v) for k, v in W.items() if isinstance(v, (int, float))},
        "hold": [],
        "corpse_cost": [],
        "death_discount": [],
        "maiev": [],
        "dormant": [],
        "towers": [],
        "enemy_towers": [],
        "iwa": [],
        "enemy_nozdormu": [],
        "enemy_thoret": False,
        "sp_extra": 0,
        "wtrig": [],
        "wlife": [],
        "played_cheap": [],
        "max_hp": 30,
        "max_mana": 0,
        "enemy_hand": 0,
        "role": 0,
        "aoe_threat": None,
        "enemy_swing": 0,
        "enemy_direct": 0,
        "dk": False,
        "power_uid": None,
        "draw_uid": -777,
        "uid_start": -1000,
        "s0": empty_s,
        "hold_minion": float(W["hold_minion"]),
    }


def run(
    decks: list[Deck],
    db: dict,
    n_games: int,
    seed: int = 1,
    max_nodes: int = 2000,
    cands: int = 6,
    threads: int = 1,
    w_b: dict | None = None,
    rollouts=None,
    mcts=None,
    ro_nodes: int = 60,
) -> dict:
    """Play `n_games` games between random pairs of `decks`.

    w_b: weight overrides for side B (seats alternate); rollouts: per side (lines, samples, depth) to pick the
    turn by Monte Carlo rollouts; mcts: per side (iterations, depth, my lines, their lines). Returns
    {"results": [(deck a, deck b, winner seat or -1, turns, B's seat or -1)], "seconds", "ro_decisions",
    "ro_changed"}."""
    ctx = table(decks, db)
    if w_b:
        ctx["w_b"] = {k: float(v) for k, v in w_b.items()}
    if rollouts:
        ctx["ro"], ctx["ro_nodes"] = [tuple(r) for r in rollouts], ro_nodes
    if mcts:
        ctx["mcts"] = [tuple(m) for m in mcts]
    return _core.selfplay(ctx, n_games, seed, max_nodes, cands, threads)
