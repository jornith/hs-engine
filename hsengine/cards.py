"""Card database: id -> Card, read from a CardDefs.xml through the `hearthstone` package.

The repository ships no card data. With no path, `hearthstone` reads the file from the `hearthstone_data`
package if it is installed, else downloads it. `load()` raises `CardDataError` with the fix when the file
cannot be found or read (see docs/DATA.md).
"""

import os
from dataclasses import dataclass
from functools import lru_cache


class CardDataError(RuntimeError):
    """The card definitions are missing or unreadable; the message says how to get them."""


@dataclass(frozen=True, slots=True)
class Card:
    id: str
    name: str
    cost: int
    atk: int
    health: int
    type: str
    text: str  # English rules text, one line; the rule parser reads it
    races: tuple[
        str, ...
    ] = ()  # minion type names (CardDefs.xml `CARDRACE` tags), e.g. ("ELEMENTAL",); "ALL" is every type


_HOW_TO_FIX = (
    "card data not found: pip install hearthstone hearthstone_data (or let hearthstone download "
    "CardDefs.xml), or pass your own file to load(path=...). See docs/DATA.md."
)


@lru_cache(maxsize=4)
def load(path: str | None = None, locale: str = "enUS") -> dict[str, Card]:
    """All cards by id. `path` points at a CardDefs.xml; by default `hearthstone` finds or downloads one."""
    if path is not None and not os.path.isfile(path):
        raise CardDataError(f"{_HOW_TO_FIX} (no such file: {path})")
    try:
        from hearthstone import cardxml
    except ImportError as e:
        raise CardDataError(_HOW_TO_FIX) from e
    try:
        db, _ = cardxml.load(path, locale=locale) if path else cardxml.load(locale=locale)
    except (ImportError, OSError, ValueError) as e:
        raise CardDataError(f"{_HOW_TO_FIX} ({e})") from e
    if not db:
        raise CardDataError(_HOW_TO_FIX)
    return {
        cid: Card(
            cid,
            c.name,
            c.cost,
            c.atk,
            c.health,
            c.type.name,
            (c.description or "").replace("\n", " "),
            tuple(r.name for r in c.races),
        )
        for cid, c in db.items()
    }


# One bit per minion type for the engine's type masks. Types after the 30th share the last bit.
_RACES = (
    "BEAST UNDEAD ELEMENTAL MECHANICAL DEMON DRAGON MURLOC PIRATE NAGA QUILBOAR DRAENEI HUMAN ORC TOTEM "
    "ABERRATION OLDGOD NIGHTELF TAUREN TROLL DWARF GNOME WORGEN CELESTIAL OGRE BLOODELF GOLEM FURBOLG GOBLIN "
    "TREANT CENTAUR"
).split()
ALL_RACES = 0x7FFFFFFF  # the mask of a minion with every type (a card whose type is ALL)


def race_bit(name: str) -> int:
    """The mask bit of one minion type name; 0 for no type, `ALL_RACES` for ALL."""
    if name == "ALL":
        return ALL_RACES
    if not name:
        return 0
    return 1 << (_RACES.index(name) if name in _RACES else len(_RACES))


def race_mask(card: Card | None) -> int:
    """The engine's type mask of a card: the OR of the bits of its types (0 when it has none or is unknown)."""
    mask = 0
    for name in card.races if card is not None else ():
        mask |= race_bit(name)
    return mask
