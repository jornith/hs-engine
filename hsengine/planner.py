"""Planner: a GameState and the card database -> the engine context, plus a Python view of the engine.

It builds the start position `S`, the per-card models (`Info`) and the side tables (corpse costs, towers,
weapon triggers...) and hands them to the Rust engine (`hsengine._core.Engine`). Moves, effects, deaths, the
enemy reply, the score and the search all run in Rust.
"""

import json
import re
from pathlib import Path
from typing import NamedTuple

from . import _core
from .cards import Card, race_bit, race_mask
from .rules import _TRIG_SPECIAL, CARD_FX, REMOVAL, Fx, Info, _fx_list, _triggers, card_model, norm, sim_text
from .state import Ent, GameState
from .weights import W

BUDGET = 0.6  # seconds for the turn search
MAX_NODES = 60000
HERO, FACE = "hero", "face"  # action targets: my hero / the enemy hero

# observed summons per card (stable outcomes in real games), keyed "card_id" or "card_id|h<herald>|<CLASS>"
SUMMON_OVERRIDES: dict = json.loads((Path(__file__).resolve().parent / "summon_overrides.json").read_text())

_HERALD_SOLDIER = {
    "DEATHKNIGHT": (1, 1),
    "ROGUE": (1, 1),
    "SHAMAN": (1, 2),
    "WARLOCK": (1, 1),
    "WARRIOR": (2, 1),
    "DEMONHUNTER": (2, 1),
}  # the Herald Soldier of each class (attack, health) before upgrades
_HERO_CLASS = {
    "HERO_01": "WARRIOR",
    "HERO_02": "SHAMAN",
    "HERO_03": "ROGUE",
    "HERO_04": "PALADIN",
    "HERO_05": "HUNTER",
    "HERO_06": "DRUID",
    "HERO_07": "WARLOCK",
    "HERO_08": "MAGE",
    "HERO_09": "PRIEST",
    "HERO_10": "DEMONHUNTER",
    "HERO_11": "DEATHKNIGHT",
}


# ---------------------------------------------------------------- simulation state


class U(NamedTuple):
    uid: int
    atk: int
    hp: int
    maxhp: int
    taunt: bool = False
    divine: bool = False
    att: int = 0  # attacks left this turn (mine only)
    stealth: bool = False
    rush: bool = False  # entered play this turn with Rush: may only attack minions
    poison: bool = False
    life: bool = False
    wf: bool = False
    frozen: bool = False
    fresh: bool = False  # entered play this turn
    dr: float = 0.0
    sp: int = 0
    elusive: bool = False
    immune: bool = False
    tmp: int = 0  # attack that expires at end of turn
    pv: float = 0.0  # passive value of text that is not simulated
    reborn: bool = False  # comes back with 1 Health the first time it dies
    doomed: bool = False  # dies at the end of my turn (Soulrest Ceremony)
    swung: bool = False  # already attacked this turn: Rush given later does not give it another attack
    shield: int = 0  # extra hits its Divine Shield takes (Thoret: "Your Divine Shields take three hits")
    race: int = 0  # minion type bits (see cards.race_mask); 0 when unknown


class S(NamedTuple):
    mana: int
    hand: tuple
    mine: tuple
    theirs: tuple
    my_hp: int
    my_armor: int
    en_hp: int
    hero_atk: int
    hero_att: int
    bonus: float
    nid: int  # next id for summoned tokens
    watk: int = 0  # equipped weapon (its attack is included in hero_atk)
    wdur: int = 0
    corpses: int = 0  # Death Knight corpses available
    died: int = 0  # minions that died this turn (both sides)
    wid: str = ""  # card id of my equipped weapon (for "after your hero attacks" weapons)
    drawn: int = 0  # cards drawn this turn (they are in hand for real; the search does not model them)
    played: int = 0  # cards played this turn so far (Combo)


_UCONV = [U.__annotations__[f] for f in U._fields]  # engine unit rows (all floats) -> U fields
_KWB = {
    "taunt": 1,
    "divine": 2,
    "windfury": 4,
    "charge": 8,
    "rush": 16,
    "stealth": 32,
    "lifesteal": 64,
    "poisonous": 128,
    "reborn": 256,
    "doomed": 512,
}


def _kwbits(kw) -> int:
    return sum(b for k, b in _KWB.items() if k in (kw or ()))


def _rust_fx(f: Fx) -> tuple:
    """An effect as the tuple the Rust engine reads. A `race:NAME` condition becomes `race:BITS`."""
    cond = f.cond or ""
    if cond.startswith("race:"):
        cond = f"race:{race_bit(cond[5:])}"
    return (f.op, int(f.n), int(f.a), int(f.h), _kwbits(f.kw), f.tgt or "", cond, f.scope or "")


_RUST_EVENTS = {
    "hero_attack": 1,
    "spell": 2,
    "minion_play": 3,
    "power": 4,
    "draw": 5,
    "self_attack": 6,
    "ally_grew": 7,
    "healed": 8,
    "friendly_died": 9,
    "self_damaged": 10,
}


_SPELL_NUM = {"dmg", "aoe", "dmg_random_minions", "dmg_random", "heal", "heal_all", "heal_hero"}


def _atiesh(info: Info, st: GameState) -> Info:
    """With the Atiesh weapon (TIME_890t) equipped, "Double the damage and healing of your spells": the
    numbers of a spell's damage and healing effects double."""
    if getattr(st, "my_weapon_id", "") != "TIME_890t" or info.kind != "SPELL" or not info.modes:
        return info
    modes = tuple(
        (tuple(f._replace(n=f.n * 2) if f.op in _SPELL_NUM else f for f in fx), unk) for fx, unk in info.modes
    )
    return info._replace(modes=modes)


def _hand_text_effects(e: Ent, c: Card | None, i: Info, st: GameState, db: dict) -> Info:
    """Card texts whose effect depends on the rest of my hand.

    "If you are holding a Dragon, summon another" summons one more minion when another card in hand has that
    type. Outcast "Draw another" draws an extra card when the card is played from the leftmost or rightmost
    hand slot.
    """
    if c is None or not i.modes:
        return i
    text = sim_text(c.text or "")
    held = re.search(r"If you(?:'re| are) holding an? (\w+), summon another", text)
    if held:
        bit = race_bit(held[1].upper())
        if any(o is not e and race_mask(db.get(o.card_id)) & bit for o in st.hand):
            i = i._replace(
                modes=tuple(
                    (tuple(f._replace(n=f.n + 1) if f.op == "summon" else f for f in fx), unk) for fx, unk in i.modes
                )
            )
    if "Outcast: Draw another" in text and st.hand and (e is st.hand[0] or e is st.hand[-1]):
        i = i._replace(modes=tuple((tuple(fx) + (Fx("draw", 1),), unk) for fx, unk in i.modes))
    return i


_TOKEN_UID = 10**9  # numbers of the models of summoned minions: far from any entity id


class Planner:
    """One position: `s0` is the start state, `info` the card model of every entity id in it."""

    def __init__(self, st: GameState, db: dict[str, Card]):
        self._ranked: list | None = None
        self._n_ranked = 0
        self._db = db
        self._tok: dict[str, int] = {}  # card id -> number of its model, for minions a Deathrattle summons
        self._eng = self._ctx = None  # the Rust engine and its context, built on first use
        self._wcodes: dict = {}  # weapon card id <-> number for the engine
        self._wname: dict = {}
        self.info: dict[int, Info] = {}
        self.printed: dict[int, tuple] = {}
        self.races: dict[int, int] = {}  # entity id -> minion type mask of its card
        # ids for units created during the search, unique across all branches (the engine shares one counter)
        self._uid = -1000
        self.max_hp = st.my_max_hp or 30
        self.max_mana = st.max_mana
        self.enemy_hand = st.enemy_hand_size
        # cards held aside return to the hand as room frees up; with room in hand the copies left over are
        # stale, so they count only when the hand starts full (a fitted condition, see docs/ACCURACY.md)
        self.held_back = st.held_back if len(st.hand) >= 10 else 0
        self.role = getattr(st, "role", 0)  # +1 beatdown, -1 control, 0 unknown
        self.aoe_threat = None  # optional (damage, chance) of their likeliest board clear, set by the caller
        # the card I draw next turn is unknown: model it as an average minion for next turn's mana
        c = min(5, max(1, self.max_mana + 1))
        self.draw_uid = -777
        self.info[self.draw_uid] = Info(
            self.draw_uid, "deck card", "MINION", c, c, c + 1, frozenset(), 0, (), 0.0, None, 0
        )

        hand = self._hand(st, db)
        self.power_uid = st.hero_power.eid if st.hero_power else None
        mine, theirs = self._boards(st, db)
        # Nozdormu, the Bronze Aspect (CATA_473) on their board: at the end of their turn their minions get
        # Divine Shield, those that have one +3/+3 instead (enemy_turn)
        self.enemy_nozdormu = {e.eid for e in st.enemy_board if e.card_id == "CATA_473"}
        self.enemy_split = self._enemy_split(st, db)
        # Spell Damage that is not on a minion (the hero, a passive, an enchantment): total minus the minions'
        self.sp_extra = max(0, getattr(st, "spell_power", 0) - sum(e.sp for e in st.board))
        self.enemy_thoret = any(e.card_id == "EDR_258" for e in st.enemy_board)
        self.played_cheap = [
            (db[c].atk, db[c].health)
            for c in getattr(st, "my_played", [])
            if c in db and db[c].type == "MINION" and db[c].cost == 1
        ]
        armor = st.my_armor
        hero_att = 1 if (st.hero_can_attack or st.hero_atk == 0) else 0
        self.s0 = S(
            st.mana,
            tuple(hand),
            mine,
            theirs,
            st.my_hp - armor,
            armor,
            st.enemy_hp,
            st.hero_atk,
            hero_att,
            0.0,
            -1000,
            st.my_weapon_atk,
            st.my_weapon_dur,
            getattr(st, "corpses", 0),
            0,
            getattr(st, "my_weapon_id", ""),
            0,
            getattr(st, "cards_played", 0),
        )
        self._hand_marks(st, db)
        self._weapons(st, db)
        self._enemy_reach(st, db)
        self._jade(st, db)
        self._summon_overrides(st, db)
        self._herald(st, db)
        self.hold = {uid: self._hold(self.info[uid]) for uid in hand}
        if self.power_uid is not None:
            self.hold[self.power_uid] = 0.0
        self.hold[self.draw_uid] = W["hold_minion"]

    def _unit(self, e: Ent, mine: bool, db: dict) -> U:
        """The engine unit of one minion on the board; registers its card model."""
        i = card_model(e, db.get(e.card_id))
        self.info[e.eid] = i
        c0 = db.get(e.card_id)
        race = self.races[e.eid] = race_mask(c0)
        if c0 is not None and c0.type == "MINION" and c0.health > 0:
            self.printed[e.eid] = (c0.atk, c0.health)  # what Silence resets a minion on the board to
        k = e.kw
        rush_only = mine and e.fresh and "rush" in k and "charge" not in k
        if "dormant" in k:  # asleep: can't attack, can't be attacked or damaged, doesn't block
            return U(
                e.eid,
                e.atk,
                e.health,
                e.maxhp or e.health,
                False,
                False,
                0,
                True,
                False,
                False,
                False,
                False,
                False,
                e.fresh,
                i.dr,
                0,
                True,
                True,
                0,
                i.pv,
                race=race,
            )
        # Frail Ghoul: "At the end of your turn, this minion dies" -> trading it costs nothing
        txt = (db.get(e.card_id).text or "") if db.get(e.card_id) else ""
        doomed = mine and "end of your turn, this minion dies" in sim_text(txt)
        return U(
            e.eid,
            e.atk,
            e.health,
            e.maxhp or e.health,
            e.taunt,
            e.divine,
            e.attacks if (mine and e.can_attack) else 0,
            e.stealth,
            rush_only,
            "poisonous" in k,
            "lifesteal" in k,
            "windfury" in k,
            "frozen" in k,
            e.fresh,
            i.dr,
            e.sp,
            "elusive" in k,
            "immune" in k,
            0,
            i.pv,
            "reborn" in k,
            doomed=doomed,
            race=race,
        )

    def _hand(self, st: GameState, db: dict) -> list:
        """Registers the card models of my hand, hero power and usable locations; returns their uids in play order."""
        hand = []
        self.dormant: set = set()  # minions in hand that enter play Dormant (asleep for N turns)
        for e in st.hand:
            c = db.get(e.card_id)
            if c is not None and c.type == "MINION" and "Dormant" in (c.text or ""):
                self.dormant.add(e.eid)
            i = _hand_text_effects(e, c, _atiesh(card_model(e, db.get(e.card_id)), st), st, db)
            self.info[e.eid] = i
            self.races[e.eid] = race_mask(c)
            hand.append(e.eid)
        if st.hero_power:  # registered even when used: it is back next turn (lookahead)
            hp = st.hero_power
            self.info[hp.eid] = card_model(hp, db.get(hp.card_id), power=True)
            # a Passive hero power (Imbue: "Blessing of the Infinite"...) works by itself, it is never clicked
            passive = "<b>Passive</b>" in ((db.get(hp.card_id).text or "") if db.get(hp.card_id) else "")
            if hp.ready and not passive:
                hand.append(hp.eid)
        for loc in st.locations:  # activating a location on my board: a free action with its effect
            spec = CARD_FX.get(loc.card_id, {})
            if loc.ready and spec.get("activate"):
                base = card_model(loc, db.get(loc.card_id))
                self.info[loc.eid] = base._replace(kind="LOCACT", cost=0, modes=((_fx_list(spec["activate"]), False),))
                hand.append(loc.eid)
        return hand

    def _boards(self, st: GameState, db: dict) -> tuple:
        """Both boards as engine units, with the board-wide rules applied (Thoret, Egg of Khelos)."""
        mine = tuple(self._unit(e, True, db) for e in st.board)
        theirs = tuple(self._unit(e, False, db) for e in st.enemy_board)
        # Thoret the Unbreakable (EDR_258): that side's Divine Shields take three hits to break
        if any(e.card_id == "EDR_258" for e in st.enemy_board):
            theirs = tuple(u._replace(shield=2) if u.divine else u for u in theirs)
        if any(e.card_id == "EDR_258" for e in st.board):
            mine = tuple(u._replace(shield=2) if u.divine else u for u in mine)
        # breaking THEIR Egg of Khelos only brings their 20/20 closer: never a target for my attacks
        theirs = tuple(
            u._replace(immune=True)
            if any(e.eid == u.uid and e.card_id.startswith("DINO_410") for e in st.enemy_board)
            else u
            for u in theirs
        )
        return mine, theirs

    def _enemy_reach(self, st: GameState, db: dict) -> None:
        # enemy reach next turn besides minions: weapon swing (blocked by Taunt) and hero power (direct)
        self.enemy_swing = st.enemy_weapon_atk
        self.enemy_direct = 0
        if st.enemy_power:
            pi = card_model(st.enemy_power, db.get(st.enemy_power.card_id), power=True)
            for f in pi.modes[0][0] if pi.modes else ():
                if f.op in ("face", "dmg"):
                    self.enemy_direct += f.n
                elif f.op == "hero_atk" and not self.enemy_swing:
                    self.enemy_swing = f.n
                elif f.op == "weapon" and not self.enemy_swing:
                    self.enemy_swing = f.a

    def _jade(self, st: GameState, db: dict) -> None:
        # Jade Golem: "Summon a {1}/{1} Jade Golem" grows with every golem summoned this game
        j = getattr(st, "jade", 0) + 1
        for e in st.hand:
            c = db.get(e.card_id)
            if c is not None and re.search(r"Summon (a|two)\b[^.]*Jade Golem", sim_text(c.text)) and e.eid in self.info:
                i = self.info[e.eid]
                n = 2 if re.search(r"Summon two\b[^.]*Jade Golem", sim_text(c.text)) else 1
                gol = tuple(Fx("summon", 1, j + k, j + k) for k in range(n))
                self.info[e.eid] = i._replace(
                    modes=tuple(
                        (tuple(f for f in fx if f.op != "summon") + gol, False) for fx, _ in (i.modes or (((), False),))
                    )
                )

    def _summon_overrides(self, st: GameState, db: dict) -> None:
        # what real games show each card summons (stable outcomes only): wins over the text reading
        for e in st.hand:
            c = db.get(e.card_id)
            if c is None or e.eid not in self.info or c.type not in ("SPELL", "MINION", "WEAPON", "LOCATION"):
                continue
            cls = _HERO_CLASS.get((getattr(st, "my_hero", "") or "")[:7], "")
            key = f"{c.id}|h{min(getattr(st, 'herald', 0), 4)}|{cls}" if "Herald" in (c.text or "") else c.id
            ls = SUMMON_OVERRIDES.get(key)
            if not ls or not ls["summons"]:
                continue
            i = self.info[e.eid]
            want = sorted((u[0], u[1]) for u in ls["summons"])
            for fx, _ in i.modes or ():
                have = sorted((f.a, f.h) for f in fx if f.op == "summon" for _ in range(f.n))
                if have == want:
                    break
            else:
                gol = tuple(Fx("summon", 1, u[0], u[1], frozenset(u[2:])) for u in ls["summons"])
                self.info[e.eid] = i._replace(
                    modes=tuple(
                        (tuple(f for f in fx if f.op != "summon") + gol, False) for fx, _ in (i.modes or (((), False),))
                    )
                )

    def _herald(self, st: GameState, db: dict) -> None:
        hcls = _HERO_CLASS.get((getattr(st, "my_hero", "") or "")[:7], "")
        if hcls in _HERALD_SOLDIER:
            # Herald: the Soldier of my class, stronger after every 2 Heralds (x2, then x4); the Shaman's one gives its
            # neighbours +its multiplier Attack
            a0, h0 = _HERALD_SOLDIER[hcls]
            m = 1 if getattr(st, "herald", 0) < 2 else 2 if st.herald < 4 else 4
            for e in st.hand:
                c = db.get(e.card_id)
                if (
                    c is None
                    or e.eid not in self.info
                    or c.type not in ("MINION", "SPELL")
                    or not re.search(r"Herald\*{0,2} \{0\}\.", re.sub(r"</?b>", "", c.text or ""))
                    or re.search(r"Deathrattle:?(</b>)?:? ?(<b>)?Herald", c.text or "")
                ):
                    continue
                i = self.info[e.eid]
                sol = Fx("summon_adj", m, a0 * m, h0 * m) if hcls == "SHAMAN" else Fx("summon", 1, a0 * m, h0 * m)
                self.info[e.eid] = i._replace(
                    modes=tuple(
                        (tuple(f for f in fx if f.op != "summon") + (sol,), unk)
                        for fx, unk in (i.modes or (((), False),))
                    )
                )

    def _enemy_split(self, st: GameState, db: dict) -> int:
        """Their delayed damage at the start of their next turn (Seal of Embers: 6 split among my side)."""
        n = 0
        for cid in getattr(st, "enemy_pending", ()) or ():
            c = db.get(cid)
            m = re.search(
                r"start of your next turn,? deal \$?(\d+) damage randomly split among all enemies",
                sim_text(c.text) if c else "",
                re.I,
            )
            if m:
                n += int(m.group(1))
        return n

    def _hand_marks(self, st: GameState, db: dict) -> None:
        """Per-card flags the engine needs for my hand and board (card ids, Corpse costs, towers...)."""
        self.cid = {e.eid: e.card_id for e in st.hand}
        # cards paid with Corpses instead of Mana; Maiev on my board makes played minions +3/+3 and Dormant
        self.corpse_cost = {
            e.eid
            for e in st.hand
            if db.get(e.card_id) is not None and "Corpses</b> instead" in (db[e.card_id].text or "")
        }
        self.death_discount = {
            e.eid
            for e in st.hand
            if db.get(e.card_id) is not None
            and "less for each minion that died this turn" in sim_text(db[e.card_id].text)
        }
        self.maiev = {e.eid for e in st.board if e.card_id == "JAIL_850"}
        self.dk = bool(getattr(st, "corpses", 0)) or bool(st.hero_power and st.hero_power.card_id.startswith("HERO_11"))
        # Ghoul Tower: "After this minion takes damage, summon two 1/1 Frail Ghouls" (Charge, die at turn end)
        self.towers = {e.eid for e in st.board + st.hand if e.card_id == "JAIL_440"}
        # their Ghoul Tower works the same: every hit it survives gives them two Frail Ghouls
        self.enemy_towers = {e.eid for e in st.enemy_board if e.card_id == "JAIL_440"}
        self.iwa = {e.eid for e in st.board if "iwa" in e.kw}  # immune while attacking (no damage back)

    def _weapons(self, st: GameState, db: dict) -> None:
        # weapons' "After your hero attacks, ..." by card id (the equipped one and those in hand)
        self.wtrig: dict = {}
        self.wlife: set = set()  # weapons with Lifesteal (Axe of Cenarius): the hero heals what it deals
        for cid in {getattr(st, "my_weapon_id", "")} | {e.card_id for e in st.hand}:
            c = db.get(cid)
            if c is not None and c.type == "WEAPON" and re.match(r"\s*Lifesteal", sim_text(c.text)):
                self.wlife.add(cid)
            if c is not None and c.type == "WEAPON" and cid not in _TRIG_SPECIAL:
                fx = [fx for ev, fx, _ in _triggers(norm(c.text or "")) if ev == "hero_attack"]
                if fx:
                    self.wtrig[cid] = fx[0]
            if c is not None and c.type == "WEAPON" and "wtrig" in CARD_FX.get(cid, {}):
                self.wtrig[cid] = _fx_list(CARD_FX[cid]["wtrig"])  # a hand-written rule wins

    # ------------------------------------------------ evaluation
    @staticmethod
    def _hold(i: Info) -> float:
        """Value of keeping a card in hand. Removal is flexible, so holding it is worth more than a body."""
        if i.kind in ("POWER", "LOCACT"):
            return 0.0
        if i.kind in ("MINION", "WEAPON", "LOCATION", "HERO"):
            return W["hold_minion"]
        fx, unknown = i.modes[0] if i.modes else ((), True)
        if fx and all(f.op == "mana" for f in fx):
            return W["hold_coin"]
        if fx and all(f.op in ("draw", "draw_to", "discover") for f in fx):
            return W["draw"] - 1.2  # a cantrip / Discover replaces itself: cast it when mana is spare
        if any(f.op in REMOVAL for m in i.modes for f in m[0]):
            return W["hold_removal"]
        return W["hold_unknown"] if unknown and not fx else W["hold_spell"]

    # ------------------------------------------------ the rules: all in the Rust engine
    @property
    def eng(self):
        """The Rust engine for this position. Built on first use, so fields set after the constructor
        (aoe_threat, enemy_direct) are in it."""
        if self._eng is None:
            self._eng = _core.Engine(self._rust_ctx())
        return self._eng

    def _d(self, s: S) -> dict:
        return {
            "mana": s.mana,
            "hand": list(s.hand),
            "mine": [[float(x) for x in u] for u in s.mine],
            "theirs": [[float(x) for x in u] for u in s.theirs],
            "my_hp": s.my_hp,
            "my_armor": s.my_armor,
            "en_hp": s.en_hp,
            "hero_atk": s.hero_atk,
            "hero_att": s.hero_att,
            "bonus": float(s.bonus),
            "watk": s.watk,
            "wdur": s.wdur,
            "corpses": s.corpses,
            "died": s.died,
            "cannon": s.wid == "JAIL_450",
            "drawn": s.drawn,
            "played": s.played,
            "wid": self._wcode(s.wid),
        }

    def _s(self, d: dict) -> S:
        def side(vs):
            return tuple(U(*(c(x) for c, x in zip(_UCONV, v, strict=False))) for v in vs)

        return S(
            d["mana"],
            tuple(d["hand"]),
            side(d["mine"]),
            side(d["theirs"]),
            d["my_hp"],
            d["my_armor"],
            d["en_hp"],
            d["hero_atk"],
            d["hero_att"],
            d["bonus"],
            -1000,
            d["watk"],
            d["wdur"],
            d["corpses"],
            d["died"],
            self._wname.get(d["wid"], ""),
            d["drawn"],
            d.get("played", 0),
        )

    def _wcode(self, cid: str) -> int:
        if not cid:
            return 0
        if cid not in self._wcodes:
            self._wcodes[cid] = len(self._wcodes) + 1
            self._wname[self._wcodes[cid]] = cid
        return self._wcodes[cid]

    def moves(self, s: S) -> list[tuple[tuple, S]]:
        return [(act, self._s(d)) for act, d in self.eng.moves(self._d(s))]

    def _apply(self, s: S, f: Fx, t, kind: str, exclude: int | None = None) -> S:
        fx = (f.op, int(f.n), int(f.a), int(f.h), _kwbits(f.kw), f.tgt or "", f.cond or "", f.scope or "")
        return self._s(self.eng.apply(self._d(s), fx, t, kind, exclude))

    def end_of_turn(self, s: S) -> S:
        return self._s(self.eng.end_of_turn(self._d(s)))

    def enemy_turn(self, s: S) -> S:
        return self._s(self.eng.enemy_turn(self._d(s)))

    def evaluate(self, s: S, threat: bool = True) -> float:
        return self.eng.evaluate(self._d(s), threat)

    def refined(self, s: S, ahead: bool = True) -> float:
        return self.eng.refined(self._d(s), ahead)

    def threat(self, s: S) -> int:
        return self.eng.threat(self._d(s))

    # ------------------------------------------------ search
    @staticmethod
    def key(s: S) -> tuple:
        return (
            s.mana,
            tuple(sorted(s.hand)),
            tuple(sorted(u[1:] for u in s.mine)),
            tuple(sorted(u[1:] for u in s.theirs)),
            s.my_hp,
            s.my_armor,
            s.en_hp,
            s.hero_atk,
            s.hero_att,
            round(s.bonus, 3),
            s.watk,
            s.wdur,
            s.corpses,
            s.played,
        )

    def _line(self, i: int) -> list:
        return [(act, self._s(d)) for act, d in self.eng.line(i)]

    def search(self, budget: float = BUDGET, candidates: int = 40, max_nodes: int = MAX_NODES) -> tuple[float, list, S]:
        """The turn search (Rust): DFS over my turn with the fast score keeping the best `candidates` end states,
        then those re-ranked by the score after the simulated enemy reply and the next-turn lookahead."""
        score, self.nodes, lethal, n = self.eng.search(float(budget), int(candidates), int(max_nodes))
        path = self._line(-1)
        self._ranked, self._n_ranked = None, (0 if lethal else n)
        return score, path, path[-1][1] if path else self.s0

    @property
    def ranked(self) -> list:
        """Best lines first (for the look-ahead): [(score, [(action, state)], final state)]."""
        if self._ranked is None and self._n_ranked:
            out = []
            for i, r in enumerate(self.eng.ranked_scores()):
                path = self._line(i)
                out.append((r, path, path[-1][1] if path else self.s0))
            self._ranked = out
        return self._ranked if self._ranked is not None else []

    @ranked.setter
    def ranked(self, v: list) -> None:
        self._ranked, self._n_ranked = v, 0

    def _rust_info(self, uid: int, i) -> dict:
        """One card's entry in the context for the Rust engine."""
        wcode = self._wcode
        fx = _rust_fx
        ev = _RUST_EVENTS
        return {
            "uid": uid,
            "name": i.kin or i.name,
            "kind": i.kind,
            "cost": int(i.cost),
            "atk": int(self.printed.get(uid, (i.atk, i.hp))[0]),
            "hp": int(self.printed.get(uid, (i.atk, i.hp))[1]),
            "kw": _kwbits(i.kw),
            "sp": int(i.sp or 0),
            "modes": [([fx(f) for f in m], bool(unk)) for m, unk in (i.modes or ())],
            "dr": float(i.dr or 0.0),
            "aura": (int(i.aura[0]), int(i.aura[1])) if i.aura else None,
            "overload": int(i.overload or 0),
            "adj": int(i.adj or 0),
            "opp_draw": int(i.opp_draw or 0),
            "dr_gen": int(i.dr_gen or 0),
            "race": self.races.get(uid, 0),
            "drs": (int(i.drs[0]), int(i.drs[1]), int(i.drs[2]), _kwbits(i.drs[3])) if i.drs else None,
            "dr_tok": self._tok.get(i.dr_card, 0),
            "eot": [fx(f) for f in (i.eot or ())],
            "pv": float(i.pv or 0.0),
            "cannon": self.cid.get(uid, "") == "JAIL_450",
            "trig": [
                (ev[e], [fx(f) for f in fxs], 1 if filt == "battlecry" else 0)
                for e, fxs, filt in (i.trig or ())
                if e in ev
            ],
            "wid": wcode(self.cid.get(uid, "")) if i.kind == "WEAPON" else 0,
        }

    def _rust_infos(self) -> list[dict]:
        """The context entries for every known card."""
        known = [(uid, i) for uid, i in self.info.items() if i is not None]
        # a Deathrattle that summons a minion with a Deathrattle of its own (a chain of eggs): the summoned
        # minion's model goes into the context under a number of its own
        made: list[tuple[int, Info]] = []
        queue = [i.dr_card for _, i in known if i.dr_card]
        while queue:
            cid = queue.pop()
            card = self._db.get(cid)
            if cid in self._tok or card is None:
                continue
            uid = _TOKEN_UID + len(self._tok)
            self._tok[cid] = uid
            e = Ent(uid, cid, card.cost, card.atk, card.health, maxhp=card.health)
            m = card_model(e, card)
            made.append((uid, m))
            if m.dr_card:
                queue.append(m.dr_card)
        return [self._rust_info(uid, i) for uid, i in known + made]

    def _rust_ctx(self) -> dict:
        if self._ctx is not None:
            return self._ctx
        wcode = self._wcode
        fx = _rust_fx
        infos = self._rust_infos()
        s0 = self.s0
        self._ctx = {
            "w": {k: float(v) for k, v in W.items() if isinstance(v, (int, float))},
            "infos": infos,
            "hold": [(k, float(v)) for k, v in self.hold.items()],
            "corpse_cost": list(self.corpse_cost),
            "death_discount": list(self.death_discount),
            "maiev": list(self.maiev),
            "dormant": list(self.dormant),
            "towers": list(self.towers),
            "enemy_towers": list(self.enemy_towers),
            "iwa": list(self.iwa),
            "enemy_nozdormu": list(self.enemy_nozdormu),
            "enemy_thoret": bool(self.enemy_thoret),
            "sp_extra": int(self.sp_extra),
            "enemy_split": int(self.enemy_split),
            "wtrig": [(wcode(cid), [fx(f) for f in fxs]) for cid, fxs in self.wtrig.items()],
            "wlife": [wcode(cid) for cid in self.wlife],
            "played_cheap": [(int(a), int(h)) for a, h in self.played_cheap],
            "max_hp": int(self.max_hp),
            "max_mana": int(self.max_mana),
            "enemy_hand": int(self.enemy_hand),
            "role": int(self.role or 0),
            "aoe_threat": (int(self.aoe_threat[0]), float(self.aoe_threat[1])) if self.aoe_threat else None,
            "enemy_swing": int(self.enemy_swing or 0),
            "enemy_direct": int(self.enemy_direct or 0),
            "dk": bool(self.dk),
            "power_uid": self.power_uid,
            "draw_uid": self.draw_uid,
            "uid_start": self._uid,
            "s0": self._d(s0),
        }
        return self._ctx
