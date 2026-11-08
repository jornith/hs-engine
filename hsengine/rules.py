"""Card text -> effects: the rule parser, the card_fx overrides and the per-card model (`Info`) for the engine.

The English rules text is normalised and matched clause by clause against a small effect vocabulary (`Fx`).
Hand-written rules in card_fx/*.json win over the parser; see docs/RULES_FORMAT.md.
"""

import dataclasses
import functools
import json
import re
from pathlib import Path
from typing import NamedTuple

from .cards import Card
from .state import Ent
from .weights import W

RULES_DIR = Path(__file__).resolve().parent / "card_fx"


def sim_text(t: str | None) -> str:
    return re.sub(r"<[^>]+>|\[x\]|[&_]", "", (t or "").replace("\n", " "))


class Fx(NamedTuple):
    op: str
    n: int = 0
    a: int = 0
    h: int = 0
    kw: frozenset = frozenset()
    tgt: str = ""  # enemy_char | enemy_minion | friendly_char | friendly_minion | same | ""
    # target filter, e.g. "atk>=5", "atk<=3", "damaged", "undamaged"; "temp" for buffs; "combo" = the effect
    # works only after another card was played this turn
    cond: str = ""
    scope: str = ""  # area effects: enemies | enemy_minions | minions | chars | other_chars | mine | others


NUM = {"a": 1, "an": 1, "one": 1, "two": 2, "three": 3, "four": 4, "five": 5, "six": 6, "seven": 7}
_KW = (
    ("taunt", "Taunt"),
    ("divine", "Divine Shield"),
    ("windfury", "Windfury"),
    ("charge", "Charge"),
    ("rush", "Rush"),
    ("stealth", "Stealth"),
    ("lifesteal", "Lifesteal"),
    ("poisonous", "Poisonous"),
    ("reborn", "Reborn"),
)
REMOVAL = {
    "dmg",
    "destroy",
    "transform",
    "aoe",
    "bounce",
    "silence",
    "set_hp",
    "set_atk",
    "destroy_random",
    "dmg_random_minions",
    "all_hp",
    "destroy_all",
    "freeze",
}


def _n(s: str) -> int:
    return NUM.get(s.lower()) or int(s)


def norm(text: str) -> str:
    """Card text -> plain English rules text: '$d2' / '#3' / '<b>' / '[x]' / '&' / '_' / reminders removed."""
    t = text.split("@")[0]
    t = re.sub(r"<[^>]+>", "", t)
    t = t.replace("[x]", "").replace("&", " ").replace("_", " ").replace("\n", " ")
    t = re.sub(r"\$[a-z]?(\d+)", r"\1", t)
    t = re.sub(r"#(\d+)", r"\1", t)
    t = re.sub(r"\([^)]*\)", "", t)
    t = re.sub(r"^\s*Hero Power\s*", "", t)
    return re.sub(r"\s+", " ", t).strip()


def _buffspec(c: str) -> tuple | None:
    """'+2/+3 and Taunt', 'Divine Shield', '+3 Attack this turn' -> (atk, hp, keywords, temporary)."""
    a = h = 0
    if m := re.search(r"\+(\d+)/\+(\d+)", c):
        a, h = int(m[1]), int(m[2])
    else:
        if m := re.search(r"\+(\d+) Attack", c, re.I):
            a = int(m[1])
        if m := re.search(r"\+(\d+) Health", c, re.I):
            h = int(m[1])
    kw = frozenset(k for k, word in _KW if word.lower() in c.lower())
    if not (a or h or kw):
        return None
    return a, h, kw, bool(re.search(r"this turn|until", c, re.I))


def _buff(c: str, tgt: str = "", scope: str = "") -> Fx | None:
    b = _buffspec(c)
    if b is None:
        return None
    return Fx(
        "buff_all" if scope else "buff", a=b[0], h=b[1], kw=b[2], tgt=tgt, scope=scope, cond="temp" if b[3] else ""
    )


def _summon_kw(rest: str) -> frozenset:
    kw = frozenset(k for k, word in _KW if word.lower() in rest.lower())
    if re.search(r"dies at (the )?end of (the |your )?turn", rest, re.I):  # Ghoul Charge's 1/1 Ghoul
        kw |= {"doomed"}
    return kw


_I = re.I
CLAUSES: list[tuple[re.Pattern, callable]] = [
    (re.compile(p, _I), f)
    for p, f in [
        # damage
        (r"^Deal (\d+) damage to all other characters", lambda m: Fx("aoe", int(m[1]), scope="other_chars")),
        (r"^Deal (\d+) damage to all characters", lambda m: Fx("aoe", int(m[1]), scope="chars")),
        (r"^Deal (\d+) damage to all other minions", lambda m: Fx("aoe", int(m[1]), scope="other_minions")),
        (r"^Deal (\d+) damage to all minions", lambda m: Fx("aoe", int(m[1]), scope="minions")),
        (r"^Deal (\d+) damage to all enemy minions", lambda m: Fx("aoe", int(m[1]), scope="enemy_minions")),
        (r"^Deal (\d+) damage to all enemies", lambda m: Fx("aoe", int(m[1]), scope="enemies")),
        (r"^Deal (\d+) damage to the enemy hero", lambda m: Fx("face", int(m[1]))),
        (r"^Deal (\d+) damage to your hero", lambda m: Fx("self_dmg", int(m[1]))),
        (r"^Deal (\d+) damage randomly split", lambda m: Fx("dmg_random", int(m[1]))),
        (
            r"^Deal (\d+) damage to (two|three) random enemy minions",
            lambda m: Fx("dmg_random_minions", int(m[1]), a=_n(m[2])),
        ),
        (r"^Deal (\d+) damage to a random enemy minion", lambda m: Fx("dmg_random_minions", int(m[1]), a=1)),
        (r"^Deal (\d+) damage to a random enemy", lambda m: Fx("dmg_random", int(m[1]))),
        (
            r"^Deal (\d+) damage to an enemy and (\d+) damage to all other enemies",
            lambda m: [Fx("dmg", int(m[1]), tgt="enemy_char"), Fx("aoe", int(m[2]), scope="other_enemies", tgt="same")],
        ),
        (
            r"^Deal (\d+) damage to an undamaged minion",
            lambda m: Fx("dmg", int(m[1]), tgt="enemy_minion", cond="undamaged"),
        ),
        (r"^Deal (\d+) damage to an enemy minion", lambda m: Fx("dmg", int(m[1]), tgt="enemy_minion")),
        # "a minion" is any minion, mine too (players do hit their own: deathrattles, "if it dies" spells)
        (r"^Deal (\d+) damage to an? minion", lambda m: Fx("dmg", int(m[1]), tgt="any_minion")),
        (r"^Deal (\d+) damage to an? (?:enemy character|enemy)$", lambda m: Fx("dmg", int(m[1]), tgt="enemy_char")),
        # plain "Deal N damage" / "to a character": any target, my own minions too (mages ping their own)
        (r"^Deal (\d+) damage(?: to an? character)?$", lambda m: Fx("dmg", int(m[1]), tgt="any_char")),
        # healing
        (r"^Restore (\d+) Health to all friendly characters", lambda m: Fx("heal_all", int(m[1]), scope="chars")),
        (r"^Restore (\d+) Health to all friendly minions", lambda m: Fx("heal_all", int(m[1]), scope="mine")),
        (r"^Restore (\d+) Health to your hero", lambda m: Fx("heal_hero", int(m[1]))),
        (r"^Restore an? (?:friendly )?minion to full Health", lambda m: Fx("heal", 99, tgt="friendly_minion")),
        (r"^Restore (\d+) Health to an? (?:friendly )?minion", lambda m: Fx("heal", int(m[1]), tgt="friendly_minion")),
        (
            r"^Restore (\d+) Health(?: to an? (?:friendly )?character)?$",
            lambda m: Fx("heal", int(m[1]), tgt="friendly_char"),
        ),
        # buffs
        (r"^give it (.+)", lambda m: _buff(m[1], tgt="same")),
        (r"^Give your hero \+(\d+) Attack this turn", lambda m: Fx("hero_atk", int(m[1]))),
        (r"^Give your characters (.+)", lambda m: _buff(m[1], scope="chars")),
        (r"^Give your other minions (.+)", lambda m: _buff(m[1], scope="others")),
        (r"^Give (?:all )?your minions (.+)", lambda m: _buff(m[1], scope="mine")),
        (r"^Give all (?:other )?friendly minions (.+)", lambda m: _buff(m[1], scope="mine")),
        (
            r"^Give all minions in your hand (.+)",
            lambda m: (lambda b: b and Fx("hand_buff", a=b[0], h=b[1]))(_buffspec(m[1])),
        ),
        (r"^Give a (?:friendly )?character (.+)", lambda m: _buff(m[1], tgt="friendly_char")),
        (r"^Give an? (?:friendly )?minion (.+)", lambda m: _buff(m[1], tgt="friendly_minion")),
        (r"^Give your opponent (\w+) Bananas?", lambda m: Fx("opp_cards", _n(m[1]))),
        (r"^\+(\d+) Attack this turn", lambda m: Fx("hero_atk", int(m[1]))),
        (
            r"^Gain \+(\d+)/\+(\d+) for each other friendly minion",
            lambda m: Fx("self_buff_allies", a=int(m[1]), h=int(m[2])),
        ),
        # control
        (
            r"^Destroy a minion with (\d+) or more Attack",
            lambda m: Fx("destroy", tgt="enemy_minion", cond=f"atk>={m[1]}"),
        ),
        (
            r"^Destroy a minion with (\d+) or less Attack",
            lambda m: Fx("destroy", tgt="enemy_minion", cond=f"atk<={m[1]}"),
        ),
        (r"^Destroy a damaged (?:enemy )?minion", lambda m: Fx("destroy", tgt="enemy_minion", cond="damaged")),
        (r"^Destroy a random enemy minion", lambda m: Fx("destroy_random")),
        (r"^Destroy the highest Attack enemy minion", lambda m: Fx("destroy_highest")),
        (r"^Destroy all minions", lambda m: Fx("destroy_all")),
        (r"^Destroy an enemy minion", lambda m: Fx("destroy", tgt="enemy_minion")),
        (r"^Destroy a minion", lambda m: Fx("destroy", tgt="any_minion")),  # players also pick their own
        (
            r"^Transform an? (?:enemy )?minion into an? (\d+)/(\d+)(.*)",
            lambda m: Fx("transform", a=int(m[1]), h=int(m[2]), kw=_summon_kw(m[3]), tgt="enemy_minion"),
        ),
        (
            r"^Silence an? (?:enemy )?minion, then deal (\d+) damage to it",
            lambda m: [Fx("silence", tgt="enemy_minion"), Fx("dmg", int(m[1]), tgt="same")],
        ),
        (r"^Silence an enemy minion", lambda m: Fx("silence", tgt="enemy_minion")),
        # "Silence a minion" can (and with no enemy minion must) hit my own: a targeted battlecry is not optional
        (r"^Silence an? minion", lambda m: Fx("silence", tgt="any_minion")),
        (r"^Freeze it$", lambda m: Fx("freeze", tgt="same")),
        (r"^Freeze (?:them|all enemy minions)", lambda m: Fx("freeze_all")),
        (r"^Freeze an? (?:enemy minion|minion)", lambda m: Fx("freeze", tgt="enemy_minion")),
        # "Freeze an enemy": their hero is a legal target too (it then can't attack)
        (r"^Freeze an? (?:enemy character|enemy|character)", lambda m: Fx("freeze", tgt="enemy_char")),
        (r"^Change a minion's Health to (\d+)", lambda m: Fx("set_hp", int(m[1]), tgt="enemy_minion")),
        (r"^Change a minion's Attack to (\d+)", lambda m: Fx("set_atk", int(m[1]), tgt="enemy_minion")),
        (r"^Change the Health of all minions to (\d+)", lambda m: Fx("all_hp", int(m[1]))),
        (
            r"^Return an enemy minion to (?:its owner's|your opponent's) hand",
            lambda m: Fx("bounce", tgt="enemy_minion"),
        ),
        (
            r"^Return an? minion to (?:its owner's|your opponent's) hand",
            lambda m: Fx("bounce", tgt="any_minion"),
        ),
        # resources
        (r"^Gain (\d+) Mana Crystals? this turn only", lambda m: Fx("mana", int(m[1]))),
        (r"^Refresh (\d+) Mana Crystals?", lambda m: Fx("mana", int(m[1]))),
        (r"^Gain \+(\d+) Attack$", lambda m: Fx("self_atk", int(m[1]))),
        (r"^Gain \+(\d+)/\+(\d+)$", lambda m: Fx("self_stats", a=int(m[1]), h=int(m[2]))),
        (r"^Both players draw an? ", lambda m: [Fx("draw", 1), Fx("opp_cards", 1)]),
        (r"^Gain an? empty Mana Crystal", lambda m: Fx("ramp", 1)),
        (r"^Gain (\d+) Armor", lambda m: Fx("armor", int(m[1]))),
        (r"^\+(\d+) Armor", lambda m: Fx("armor", int(m[1]))),
        (r"^Equip an? (\d+)/(\d+)", lambda m: Fx("weapon", a=int(m[1]), h=int(m[2]))),
        (r"^Draw (a|an|one|two|three|\d+) cards?", lambda m: Fx("draw", _n(m[1]))),
        (r"^Discover", lambda m: Fx("discover", 1)),  # a card in hand, but not a draw ("whenever you draw" stays quiet)
        (r"^Add (a|an|one|two|three|\d+) .+ to your hand", lambda m: Fx("draw", _n(m[1]))),
        (r"^Draw an? \w+, \w+,? and an? \w+", lambda m: Fx("draw", 3)),
        (r"^Draw an? [A-Z]\w*", lambda m: Fx("draw", 1)),
        (r"^Give your weapon \+(\d+) Attack", lambda m: Fx("weapon_buff", int(m[1]))),
        (r"^Gain \+(\d+) Health for each card in your hand", lambda m: Fx("self_buff_hand", h=int(m[1]))),
        (
            r"^Summon all three Animal Companions",
            lambda m: [
                Fx("summon", 1, 4, 2, frozenset({"charge"})),
                Fx("summon", 1, 2, 4),
                Fx("summon", 1, 4, 4, frozenset({"taunt"})),
            ],
        ),
        (r"^Summon (two|three) copies of this minion", lambda m: Fx("summon_copies", _n(m[1]))),
        (r"^Take (\d+) damage", lambda m: Fx("self_dmg", int(m[1]))),
        (r"^Discard (a|an|one|two|three|\d+) random cards?", lambda m: Fx("discard", _n(m[1]))),
        # summons
        (
            r"^Summon (a|an|one|two|three|four|\d+) (\d+)/(\d+)(.*) for your opponent",  # their side of the board
            lambda m: [Fx("enemy_summon", 0, int(m[2]), int(m[3]), _summon_kw(m[4]))] * _n(m[1]),
        ),
        (
            r"^Summon (a|an|one|two|three|four|\d+) (\d+)/(\d+)(.*)",
            lambda m: Fx("summon", _n(m[1]), int(m[2]), int(m[3]), _summon_kw(m[4])),
        ),
        (r"^Summon (a|an|one|two|three|four|\d+) \{0\} Silver Hand Recruits?", lambda m: Fx("summon", _n(m[1]), 1, 1)),
        (r"^Summon a random Beast Companion", lambda m: Fx("summon", 1, 4, 3)),
        (r"^Summon a random basic Totem", lambda m: Fx("summon", 1, 0, 2)),
        (r"^Summon a random (\d+)-Cost minion", lambda m: Fx("summon", 1, int(m[1]), int(m[1]) + 1)),
    ]
]
# clauses that are conditions, triggers or keyword lines: not an effect, but not "unknown" either
_SKIP = re.compile(
    r"^(If |Whenever|After |At the |Combo|Secret|Overload|Spell Damage|Your other|Adjacent|"
    r"Can't|Costs|Lifesteal|Taunt|Divine Shield|Charge|Rush|Windfury|Stealth|Poisonous|Reborn|"
    r"Elusive|Deathrattle|Battlecry|Outcast|Frenzy|Corrupt|Tradeable|Infuse|Manathirst|Spellburst)",
    re.I,
)
_VERB = r"(?:Draw|Restore|Gain|Freeze|give|take|deal|summon|destroy|return|equip|discover|add)"


@functools.lru_cache(maxsize=8192)
def _parse_cached(text: str) -> tuple[tuple, bool]:
    return _parse(text, None)


def parse(text: str, missed: list | None = None) -> tuple[tuple, bool]:
    """Normalised effect text -> (effects, has_unknown_parts). Unparsed clauses go to `missed` if given."""
    return _parse(text, missed) if missed is not None else _parse_cached(text)


def _parse(text: str, missed: list | None) -> tuple[tuple, bool]:
    fx, unknown = [], False
    for sent in re.split(r"(?<=[.!;])\s+", text):
        for c in re.split(rf"\s+and\s+(?={_VERB}\b)", sent.strip().rstrip(".;"), flags=re.I):
            c = c.strip()
            if not c:
                continue
            for pat, build in CLAUSES:
                if m := pat.match(c):
                    f = build(m)
                    if f:
                        fx.extend(f if isinstance(f, list) else [f])
                        break
            else:
                if not _SKIP.match(c):
                    unknown = True
                    if missed is not None:
                        missed.append(c)
    return tuple(fx), unknown


_LEAD_KW = re.compile(
    r"^((?:Shattered|Shatter|Rewind|Prepare|Lifesteal|Tradeable|Twinspell|Echo|Rush|Taunt|Charge|Divine Shield|"
    r"Windfury|Stealth|Poisonous|Reborn|Elusive|Discover)\b[,.]?\s+)+(?=[A-Z])"
)


def strip_lead_keywords(text: str) -> tuple[str, frozenset]:
    """'Lifesteal Deal 3 damage to a minion.' -> ('Deal 3 damage to a minion.', {'lifesteal'})."""
    m = _LEAD_KW.match(text)
    if not m or m.group(0).startswith("Discover"):  # "Discover a ..." is an effect, not a keyword line
        return text, frozenset()
    kws = frozenset(k for k, w in _KW if w.lower() in m.group(0).lower())
    return text[m.end() :], kws


def _section(text: str, key: str) -> str:
    m = re.search(rf"{key}[^:]*:\s*(.+?)(?=(?:Deathrattle|Battlecry|Combo|Overload|Spellburst|Frenzy)[^:]*:|$)", text)
    return m.group(1).strip() if m else ""


def _modes(text: str) -> list[tuple[tuple, bool]]:
    """'Choose One - X; or Y' -> two modes; anything else -> one."""
    m = re.match(r"Choose One\s*-\s*(.+)", text)
    if not m:
        return [parse(text)]
    out = []
    for opt in re.split(r";\s*or\s+", m.group(1)):
        opt = re.sub(r"^(\d+ damage)", r"Deal \1", opt.strip())
        out.append(parse(opt))
    return out


def _uv_raw(a: int, h: int, kw: frozenset = frozenset()) -> float:
    v = W["base"] + W["atk"] * a + W["hp"] * h + W["body"] * a * h
    if "taunt" in kw:
        v += W["taunt"]
    if "divine" in kw:
        v += 0.9 + 0.3 * a
    return v


def _dr_value(fx: tuple) -> float:
    v = 0.0
    for f in fx:
        if f.op == "summon":
            v += 0.85 * f.n * _uv_raw(f.a, f.h, f.kw)
        elif f.op == "draw":
            v += W["draw"] * f.n
        elif f.op == "weapon":
            v += 0.8 * (f.a + f.h)
        elif f.op in ("dmg", "face", "aoe", "dmg_random", "dmg_random_minions"):
            v += 0.6 * f.n
        else:
            v += 1.0
    return v or 1.5


class Info(NamedTuple):
    uid: int
    name: str
    kind: str  # MINION SPELL WEAPON POWER
    cost: int
    atk: int
    hp: int
    kw: frozenset
    sp: int
    modes: tuple  # tuple of (effects, unknown) for battlecry / spell / power
    dr: float
    aura: tuple | None  # (atk, hp) given to my other minions while this is alive
    overload: int
    drs: tuple | None = None  # deathrattle summon (count, atk, hp, keywords): put on the board when it dies
    eot: tuple = ()  # effects at the end of my turn while it is on my board
    pv: float = 0.0  # value of ongoing text the simulator cannot play out (auras, triggers, ...)
    trig: tuple = ()  # ((event, effects, filter), ...): "After your hero attacks, ..." and the like
    adj: int = 0  # "Adjacent minions have +N Attack"
    opp_draw: int = 0  # "Deathrattle: Your opponent draws a card": cards the other side draws when it dies
    dr_gen: int = 0  # "Deathrattle: Get a Coin": cards put in my hand when it dies
    dr_card: str = ""  # card id of the minion the Deathrattle summons, when that minion has a Deathrattle of its own
    kin: str = ""  # a key shared by cards that act on each other by name (the Rafaams); empty for the rest


# The Timethief Rafaam cards buff one another by name; their names differ and change with the card language, so
# they are tied together by card id instead.
_KIN_PREFIX = "TIME_005"
_KIN = "rafaam"

CARD_FX: dict = {}
for _f in sorted(RULES_DIR.glob("*.json")):
    CARD_FX.update(json.loads(_f.read_text()))  # hand-written rules, one file per set; later files win


def _fx_list(items: list) -> tuple:
    return tuple(
        Fx(
            d["op"],
            d.get("n", 0),
            d.get("a", 0),
            d.get("h", 0),
            frozenset(d.get("kw", ())),
            d.get("tgt", ""),
            d.get("cond", ""),
            d.get("scope", ""),
        )
        for d in items
    )


_NUM = {"a": 1, "an": 1, "one": 1, "two": 2, "three": 3, "four": 4, "five": 5}
_GEN = [
    re.compile(r"\bGet (a|an|one|two|three|four|five|\d+)\b", re.I),
    re.compile(r"\bAdd (a|an|one|two|three|four|five|\d+)\b[^.]*? to your hand", re.I),
]


def generated_cards(text: str) -> int:
    """Cards this card puts in my hand right away ("Get two...", "Add a ... to your hand", "Discover"),
    besides draws. Deathrattles and later triggers do not count."""
    t = re.split(r"Deathrattle:|At the end of|At the start of|Whenever|After you|After a|After this", text)[0]
    n = 0
    for rx in _GEN:
        for m in rx.finditer(t):
            w = m.group(1).lower()
            n += int(w) if w.isdigit() else _NUM.get(w, 1)
    if not n and re.search(r"\bDiscover\b|\bChoose [^.]*? to add to your hand", t, re.I):
        n = 1
    return n


def _ley_line(info: Info, cid: str, script: tuple) -> Info:
    """Ley-line cards: the card's own script tags carry the live numbers ({0} = damage, cost or discount,
    {1} = repeats or cards; no second tag means 1). Their level is not the hero power's Imbue level."""
    n0, n1 = script[0], (script[1] if len(script) > 1 else 1)

    def fix(f: Fx) -> Fx:
        if cid == "MEND_500" and f.op in ("dmg_random_minions", "dmg_excess"):
            return f._replace(n=n0, a=n1)
        if cid == "MEND_502" and f.op == "summon":
            return f._replace(n=n1, a=n0, h=n0)  # a random n0-Cost minion: about n0/n0
        if cid == "MEND_504" and f.op == "draw":
            return f._replace(n=n1)
        return f

    return info._replace(modes=tuple((tuple(fix(f) for f in fx), unk) for fx, unk in info.modes))


def card_model(e: Ent, card: Card | None, power: bool = False) -> Info:
    script = getattr(e, "script", ())
    live = None
    if card and len(script) >= 2 and re.search(r"\{0\}/\{1\}", card.text or ""):
        # "Summon a {0}/{1} Whelp": the numbers live in the entity's script tags (they grow during the game);
        # they win over the parser and over a hand translation's average (Twilight Egg was taken for a 4/4)
        live = (script[0], script[1])
        card = dataclasses.replace(card, text=card.text.replace("{0}", str(script[0])).replace("{1}", str(script[1])))
    info = _card_model_spec(e, card, power)
    if live:
        dr_part = (card.text or "").split("Deathrattle", 1)
        if info.drs and len(dr_part) > 1 and f"{live[0]}/{live[1]}" in dr_part[1]:
            info = info._replace(drs=(info.drs[0], live[0], live[1], info.drs[3]))
        elif info.modes and f"{live[0]}/{live[1]}" in dr_part[0]:
            info = info._replace(
                modes=tuple(
                    (tuple(f._replace(a=live[0], h=live[1]) if f.op == "summon" else f for f in fx), unk)
                    for fx, unk in info.modes
                )
            )
    if card and card.id == "FIR_911" and script and info.modes:
        # "Draw {0} cards": it upgrades every turn it stays in hand, the live number is the script value
        info = info._replace(
            modes=tuple(
                (tuple(f._replace(n=script[0]) if f.op == "draw" else f for f in fx), unk) for fx, unk in info.modes
            )
        )
    if card and card.id in ("MEND_500", "MEND_502", "MEND_504") and script and info.modes:
        info = _ley_line(info, card.id, script)
    if (
        card and card.id == "JAIL_EVENT_101hp" and script and info.modes
    ):  # "Deal @ damage": the live number is the script value
        info = info._replace(
            modes=tuple(
                (tuple(f._replace(n=script[0]) if f.op == "dmg_random" else f for f in fx), unk)
                for fx, unk in info.modes
            )
        )
    if (
        card
        and info.modes
        and re.search(r"\bDiscover\b", card.text or "")
        and not re.search(r"\bDraw\b", card.text or "", re.I)
    ):
        # hand translations wrote Discover as a draw: it is not one for "whenever you draw" triggers
        info = info._replace(
            modes=tuple(
                (tuple(f._replace(op="discover") if f.op == "draw" else f for f in fx), unk) for fx, unk in info.modes
            )
        )
    if card and info.modes and info.kind not in ("MINION_TOKEN",):
        g = generated_cards(sim_text(card.text))
        if g and not any(f.op in ("draw", "draw_to", "gen", "discover") for fx, _ in info.modes for f in fx):
            # the hand grows: the evaluation counts cards in hand, the draw bonus is not given twice
            info = info._replace(modes=tuple((fx + (Fx("gen", g),), unk) for fx, unk in info.modes))
    if card and "Deathrattle" in (card.text or ""):
        m = re.search(
            r"Deathrattle:\W*(?:[^.]*?\. )?Your opponent draws (a|an|one|two|three|\d+) cards?",
            sim_text(card.text),
            re.I,
        )
        if m:
            w = m[1].lower()
            info = info._replace(opp_draw=int(w) if w.isdigit() else _NUM.get(w, 1))
        elif re.search(r"Deathrattle:\W*Fill your opponent's hand", sim_text(card.text), re.I):
            info = info._replace(opp_draw=10)  # Coins up to the hand limit
    return info


def _card_model_spec(e: Ent, card: Card | None, power: bool = False) -> Info:
    base = _card_model(e, card, power)
    if card and re.search(r"\. It dies at (the )?end of (the |your )?turn", sim_text(card.text)):
        # "Summon a 1/1 Ghoul with Charge. It dies at end of turn." - the second sentence is about the summon
        base = base._replace(
            modes=tuple(
                (
                    tuple(f._replace(kw=f.kw | {"doomed"}) if f.op == "summon" else f for f in fx),
                    unk and not any(f.op == "summon" for f in fx),
                )
                for fx, unk in base.modes
            )
        )
    if (
        card
        and (m := re.search(r"Adjacent minions have \+(\d+|\{0\}) Attack", sim_text(card.text)))
        and "silenced" not in e.kw
    ):
        base = base._replace(adj=int(m[1]) if m[1].isdigit() else int((getattr(e, "script", ()) or (1,))[0]))
    spec = CARD_FX.get(card.id) if card else None
    if not spec:
        return base
    # a hand-written rule (card_fx/*.json) wins over the regex parser
    modes = base.modes
    if "modes" in spec:
        modes = tuple((_fx_list(m), False) for m in spec["modes"])
    elif "play" in spec:
        modes = ((_fx_list(spec["play"]), False),)
    elif "activate" in spec and base.kind == "LOCATION":
        charges = max(1, card.health)
        modes = ((_fx_list(spec["activate"]) + (Fx("value", 15 * (charges - 1)),), False),)
    dr, drs = base.dr, base.drs
    if "dr" in spec and "silenced" not in e.kw:
        fx = _fx_list(spec["dr"])
        summ = [f for f in fx if f.op == "summon"]
        rest = tuple(f for f in fx if f.op != "summon")
        drs = (summ[0].n, summ[0].a, summ[0].h, summ[0].kw) if summ else None
        dr = _dr_value(rest) if rest else 0.0
    if "dr_gen" in spec and "silenced" not in e.kw:
        base = base._replace(dr_gen=int(spec["dr_gen"]))
    aura = tuple(spec["aura"]) if "aura" in spec else base.aura
    eot = (_fx_list(spec["eot"]) if "eot" in spec else base.eot) if "silenced" not in e.kw else ()
    pv = float(spec.get("pv", 0.0)) if "silenced" not in e.kw else 0.0
    trig = base.trig
    if "trig" in spec:  # [[event, [effects], filter], ...] written by hand
        trig = tuple((ev, _fx_list(fxs), filt) for ev, fxs, filt in spec["trig"]) if "silenced" not in e.kw else ()
    if card.id == "CATA_725t" and "silenced" not in e.kw:  # the +{0}/+{0} is the soldier's script value
        n = (getattr(e, "script", ()) or (1,))[0]
        eot = (Fx("eat_right", a=n, h=n),)
    dr_card = str(spec.get("dr_card", "")) if "silenced" not in e.kw else ""
    return base._replace(modes=modes, dr=dr, drs=drs, aura=aura, eot=eot, pv=pv, trig=trig, dr_card=dr_card)


# ---------------------------------------------------------------- triggers (the event rules a full engine has)
# event name, text that starts the sentence, filter on the triggering card
_TRIGGERS = [
    ("hero_attack", r"After your hero attacks, ", ""),
    ("spell", r"After you cast a spell, ", ""),
    ("minion_play", r"After you play a Battlecry minion, ", "battlecry"),
    ("minion_play", r"After you play a minion, ", ""),
    ("power", r"After you use your Hero ?Power, ", ""),
    ("draw", r"Whenever you draw a card, ", ""),
    ("self_attack", r"After this attacks, ", ""),
    ("friendly_died", r"(?:Whenever|After) a friendly minion dies, ", ""),
    ("self_damaged", r"Whenever this (?:minion )?takes damage, ", ""),
]
_TRIG_SPECIAL = {"JAIL_850", "JAIL_450", "JAIL_440"}  # Maiev, Corpse Cannon, Ghoul Tower: handled by hand


def _triggers(text: str) -> tuple:
    out = []
    for ev, lead, filt in _TRIGGERS:
        m = re.search(lead + r"([^.]+)\.?", text, re.I)
        if not m:
            continue
        fx, unknown = parse(m.group(1)[0].upper() + m.group(1)[1:])
        if fx:
            out.append((ev, fx, filt))
    return tuple(out)


def _card_model(e: Ent, card: Card | None, power: bool = False) -> Info:
    name = card.name if card else f"#{e.eid}"
    kind = "POWER" if power else (card.type if card else "MINION")
    raw = card.text if card else ""
    text = norm(raw)
    ov = re.search(r"Overload:\s*\((\d+)\)", re.sub(r"<[^>]+>", "", raw))
    overload = int(ov[1]) if ov else 0
    kw = e.kw
    dr, drs = 0.0, None
    if ("deathrattle" in kw or "Deathrattle" in text) and "silenced" not in kw:
        fx = parse(_section(text, "Deathrattle"))[0]
        summ = [f for f in fx if f.op == "summon"]
        rest = tuple(f for f in fx if f.op != "summon")
        if summ:  # summoned bodies are simulated for real; only the rest is a flat value
            drs = (summ[0].n, summ[0].a, summ[0].h, summ[0].kw)
            dr = _dr_value(rest) if rest else 0.0
        else:
            dr = _dr_value(fx)
    aura = None
    if kind == "MINION":
        bc = _section(text, "Battlecry")
        modes = tuple([parse(bc)]) if bc else ()
        if m := re.search(r"Your other minions have \+(\d+)(?:/\+(\d+)| Attack)", text):
            aura = (int(m[1]), int(m[2] or 0))
    elif kind in ("SPELL", "POWER"):
        body, lead = strip_lead_keywords(text)
        modes = tuple(_modes(body))
        if "lifesteal" in lead:
            modes = tuple(
                (fx + tuple(Fx("lifesteal_heal", f.n) for f in fx if f.op in ("dmg", "face")), unk) for fx, unk in modes
            )
    else:
        modes = ()
    trig = (
        _triggers(text)
        if kind in ("MINION", "WEAPON") and "silenced" not in kw and card is not None and card.id not in _TRIG_SPECIAL
        else ()
    )
    eot = ()
    if kind == "MINION" and "silenced" not in kw and (m := re.search(r"At the end of your turn,? ([^.]+)", text)):
        # "At the end of your turn, deal 1 damage to a random enemy / gain +1/+1": read like any effect text
        fx, _unk = parse(m.group(1)[0].upper() + m.group(1)[1:])
        eot = fx
    kin = _KIN if card is not None and card.id.startswith(_KIN_PREFIX) else ""
    return Info(
        e.eid,
        name,
        kind,
        e.cost,
        e.atk,
        e.health,
        kw,
        e.sp,
        modes,
        dr,
        aura,
        overload,
        drs,
        eot=eot,
        trig=trig,
        kin=kin,
    )
