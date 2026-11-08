"""hsengine: a Hearthstone turn simulator and search.

    from hsengine import cards, Planner, StateTracker
    db = cards.load()
    p = Planner(game_state, db)
    score, path, final = p.search()

The rules run in Rust (`hsengine._core`); this package turns card text and game logs into its input.
"""

from .cards import Card, CardDataError
from .planner import FACE, HERO, Planner, S, U
from .rules import Fx, Info, card_model
from .state import Ent, GameState, StateTracker
from .weights import W

__all__ = [
    "Card",
    "CardDataError",
    "Ent",
    "FACE",
    "Fx",
    "GameState",
    "HERO",
    "Info",
    "Planner",
    "S",
    "StateTracker",
    "U",
    "W",
    "card_model",
]
