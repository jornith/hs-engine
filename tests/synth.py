"""Synthetic cards and position builders shared by the tests (no card database needed).

Every card here has a made-up id and rules text written in the card-database style, so the rule parser compiles
it the same way it compiles real cards. `card`, `minion` and `power` build entities for a `GameState`; `state`
builds my turn with both heroes at 30 health unless told otherwise.
"""

import itertools

from hsengine.cards import Card
from hsengine.state import Ent, GameState


def _c(cid, name, cost, atk, hp, typ, text="", races=()):
    return cid, Card(cid, name, cost, atk, hp, typ, text, races)


DB = dict(
    [
        _c("T_ARCANE", "Test Shot", 1, 0, 0, "SPELL", "Deal $2 damage."),
        _c("T_BOLT", "Test Bolt", 1, 0, 0, "SPELL", "Deal $3 damage. <b>Overload:</b> (1)"),
        _c("T_FROST", "Test Frost", 2, 0, 0, "SPELL", "Deal $3 damage to a character and <b>Freeze</b> it."),
        _c("T_SHOCK", "Test Shock", 1, 0, 0, "SPELL", "<b>Silence</b> a minion, then deal $1 damage to it."),
        _c("T_HOLY", "Test Holy Fire", 3, 0, 0, "SPELL", "Deal $2 damage to all enemies."),
        _c("T_HEAL", "Test Light", 2, 0, 0, "SPELL", "Restore #8 Health."),
        _c("T_KILL", "Test Doom", 3, 0, 0, "SPELL", "Destroy a minion."),
        _c("T_WINDS", "Test Winds", 2, 0, 0, "SPELL", "Give a minion <b>Windfury</b>."),
        _c("T_ROAR", "Test Roar", 3, 0, 0, "SPELL", "Give your characters +2 Attack this turn."),
        _c("T_DRAW", "Test Study", 1, 0, 0, "SPELL", "Draw a card."),
        _c("T_GOLEM", "Test Golem", 3, 2, 3, "MINION", "<b>Deathrattle:</b> Summon a 2/1 Damaged Golem."),
        _c("T_LEADER", "Test Leader", 3, 2, 2, "MINION", "Your other minions have +1 Attack."),
        _c("T_RIDER", "Test Rider", 3, 3, 1, "MINION", "<b>Charge</b>"),
        _c("T_GUARD", "Test Guard", 4, 3, 5, "MINION", "<b>Taunt</b>"),
        _c("T_YETI", "Test Yeti", 4, 4, 5, "MINION", ""),
        _c("T_RAPTOR", "Test Raptor", 2, 3, 2, "MINION", ""),
        _c("T_NEST", "Test Nest", 2, 0, 0, "SPELL", "Summon a 1/1 Whelp. If you're holding a Dragon, summon another."),
        _c("T_WYRM", "Test Wyrm", 5, 4, 4, "MINION", "", ("DRAGON",)),
        _c("T_SEER", "Test Seer", 1, 0, 0, "SPELL", "Draw a card. <b>Outcast:</b> Draw another."),
        _c("TIME_005t2", "Zelenyj Rafaamov", 3, 3, 3, "MINION", "Rule comes from the shipped rule files."),
        _c("TIME_005t3", "Issledovatel Rafaamov", 3, 3, 3, "MINION", ""),
        _c("T_BOAR", "Test Boar", 1, 1, 1, "MINION", ""),
        _c("T_OGRE", "Test Ogre", 6, 6, 7, "MINION", ""),
        _c("T_GHOUL", "Test Ghoul", 1, 1, 1, "MINION", "<b>Charge</b>. At the end of your turn, this minion dies."),
        _c("T_FISH", "Test Fish", 1, 2, 3, "MINION", "", ("MURLOC",)),
        _c("T_WOLF", "Test Wolf", 1, 2, 3, "MINION", "", ("BEAST",)),
        _c("T_CHIMERA", "Test Chimera", 2, 2, 3, "MINION", "", ("BEAST", "MURLOC")),
        _c("T_STORM", "Test Storm", 2, 0, 0, "SPELL", "Rule comes from a hand-written entry in the test."),
        _c("T_CHANT", "Test Chant", 2, 0, 0, "SPELL", "Rule comes from a hand-written entry in the test."),
        _c("T_MASK", "Test Mask", 2, 0, 0, "SPELL", "Rule comes from a hand-written entry in the test."),
        _c("T_AXE", "Test Axe", 2, 3, 2, "WEAPON", ""),
        _c("T_PINGER", "Test Pinger", 2, 2, 2, "MINION", "<b>Battlecry:</b> Deal 1 damage to a minion."),
        _c("T_BLAST", "Test Blast", 3, 0, 0, "SPELL", "Rule comes from a hand-written entry in the test."),
        _c("T_COINER", "Test Coiner", 1, 1, 1, "MINION", "Rule comes from a hand-written entry in the test."),
        _c("T_SPARK", "Test Spark", 1, 0, 0, "SPELL", "Rule comes from a hand-written entry in the test."),
        _c("T_CHEST", "Test Chest", 2, 0, 3, "MINION", "<b>Deathrattle:</b> Fill your opponent's hand with Coins."),
        _c("MEND_500", "Test Ley Bolt", 2, 0, 0, "SPELL", "Rule comes from the shipped rule files."),
        _c("MEND_502", "Test Ley Summon", 2, 0, 0, "SPELL", "Rule comes from the shipped rule files."),
        _c("MEND_504", "Test Ley Draw", 2, 0, 0, "SPELL", "Rule comes from the shipped rule files."),
        _c("CATA_785", "Test Combo Blast", 2, 0, 0, "SPELL", "Rule comes from the shipped rule files."),
        _c("CATA_485", "Test Spill", 2, 0, 0, "SPELL", "Rule comes from the shipped rule files."),
        _c("FIR_911", "Test Upgrade", 2, 0, 0, "SPELL", "Draw {0} cards."),
        _c(
            "T_BP_FACE",
            "Test Steady Shot",
            2,
            0,
            0,
            "HERO_POWER",
            "<b>Hero Power</b> Deal $2 damage to the enemy hero.",
        ),
        _c("T_BP_ARMOR", "Test Armor Up", 2, 0, 0, "HERO_POWER", "<b>Hero Power</b> Gain 2 Armor."),
        _c("T_BP_PING", "Test Fireblast", 2, 0, 0, "HERO_POWER", "<b>Hero Power</b> Deal $1 damage."),
        _c("GAME_005", "The Coin", 0, 0, 0, "SPELL", "Gain 1 Mana Crystal this turn only."),
    ]
)
_eid = itertools.count(100)


def card(cid, *, kw=(), eid=None):
    """A card in my hand."""
    c = DB[cid]
    return Ent(eid or next(_eid), cid, c.cost, c.atk, c.health, maxhp=c.health, kw=frozenset(kw))


def minion(cid, atk, hp, *, kw=(), ready=True, mine=True, maxhp=None, eid=None):
    """A minion on the board (`ready`: it can attack this turn)."""
    kw = frozenset(kw)
    can = ready and mine and "frozen" not in kw
    return Ent(
        eid or next(_eid),
        cid,
        DB[cid].cost,
        atk,
        hp,
        "taunt" in kw,
        can,
        "divine" in kw,
        "stealth" in kw,
        (2 if "windfury" in kw else 1) if can else 0,
        True,
        maxhp or hp,
        kw,
    )


def power(cid, *, eid=None):
    """My hero power, ready to use."""
    return Ent(eid or next(_eid), cid, 2, 0, 0, ready=True)


def state(**kw) -> GameState:
    """My turn 10 with both heroes at 30; keyword arguments set `GameState` fields."""
    st = GameState(turn=kw.pop("turn", 10), my_turn=True, my_hp=30, enemy_hp=30, max_mana=10, main_action=True)
    for k, v in kw.items():
        setattr(st, k, v)
    if "max_mana" not in kw:
        st.max_mana = max(st.mana, 1)
    return st
