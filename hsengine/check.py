"""Engine vs a logged game: replay one real action and compare the engine's result with what the game logged.

An outcome is the 5-tuple (my board, enemy board, my health + armor, enemy health, hand size); a board is the
sorted (attack, health) pairs of its minions. `Case` holds everything needed to replay one action without the
card database: the engine context, the action and the cards that were in hand.
"""

from dataclasses import dataclass, field

from . import _core
from .planner import FACE, HERO, Planner
from .state import GameState
from .weights import W

DIMS = ("my_board", "enemy_board", "my_hp", "enemy_hp", "hand")


def real_outcome(st: GameState) -> tuple:
    """The outcome as the game logged it."""
    return (
        tuple(sorted((e.atk, e.health) for e in st.board)),
        tuple(sorted((e.atk, e.health) for e in st.enemy_board)),
        st.my_hp,
        st.enemy_hp,
        len(st.hand),
    )


def outcome(d: dict, hand_ids: set, held_back: int = 0) -> tuple:
    """The outcome of an engine state dict. An enemy's `tmp` is an attack reduction until my next turn: the
    game already shows it reduced. `held_back` cards wait outside the hand and fill the free slots (max 10)."""
    hand = sum(1 for h in d["hand"] if h in hand_ids) + int(d["drawn"])
    hand += max(0, min(held_back, 10 - hand))
    return (
        tuple(sorted((int(u[1]), int(u[2])) for u in d["mine"])),
        tuple(sorted((int(u[1] - u[18]), int(u[2])) for u in d["theirs"])),
        int(d["my_hp"] + d["my_armor"]),
        int(d["en_hp"]),
        hand,
    )


def snapshot(d: dict) -> dict:
    """An engine state dict as comparable JSON-ready values: floats rounded, the search-only score left out."""

    def r(x):
        return round(float(x), 3)

    return {
        k: ([[r(x) for x in u] for u in v] if k in ("mine", "theirs") else list(v) if k == "hand" else v)
        for k, v in d.items()
        if k != "bonus"
    }


def diff(a: tuple, b: tuple) -> list[str]:
    return [DIMS[i] for i in range(5) if a[i] != b[i]]


@dataclass
class Case:
    ctx: dict  # the engine context (Planner._rust_ctx), "w" optional
    kind: str  # "PLAY" or "ATTACK"
    src: object  # entity id, or HERO for a hero attack
    tgt: object  # entity id, HERO, FACE or None
    hand_ids: list = field(default_factory=list)  # the cards in hand (not the hero power, not locations)
    held_back: int = 0  # held-back cards that return to the hand when a slot frees up (not stored in fixtures)

    def candidates(self) -> list[tuple]:
        """Outcomes of every engine move that is this action. The engine tries one of several identical units
        only (same stats, same moves), so units match by stats and cards by (name, cost, attack, health, kind)."""
        eng = self.engine()
        ctx = self.ctx
        s0 = ctx["s0"]
        units = {int(u[0]): tuple(u[1:]) for u in s0["mine"] + s0["theirs"]}
        infos = {i["uid"]: (i["name"], i["cost"], i["atk"], i["hp"], i["kind"]) for i in ctx["infos"]}
        sig = infos.get(self.src) if self.kind == "PLAY" else None

        def twin(x, y):
            return x == y or (x in units and y in units and units[x] == units[y])

        hand = set(self.hand_ids)
        out = []
        for act, d in eng.moves(s0):
            if act[0] != self.kind.lower():
                continue
            if not (twin(act[1], self.src) or (sig is not None and infos.get(act[1]) == sig)):
                continue
            if self.tgt is not None and act[2] is not None and not twin(act[2], self.tgt):
                continue
            out.append(outcome(d, hand, self.held_back))
        return out

    def plays(self) -> list[tuple]:
        """(target, mode, state snapshot) of every engine move that plays exactly `src`, sorted."""
        eng = self.engine()
        out = [
            (act[2], act[3], snapshot(d))
            for act, d in eng.moves(self.ctx["s0"])
            if act[0] == "play" and act[1] == self.src
        ]
        return sorted(out, key=repr)

    def engine(self):
        ctx = self.ctx if "w" in self.ctx else {**self.ctx, "w": {k: float(v) for k, v in W.items()}}
        return _core.Engine(ctx)

    def judge(self, real: tuple) -> list[str] | None:
        """The dimensions the closest engine result gets wrong ([] = exact), None if the engine has no such move."""
        cands = self.candidates()
        if not cands:
            return None
        return diff(min(cands, key=lambda c: len(diff(c, real))), real)


def _fx(f) -> tuple:
    return tuple(f)


def _opt(v):
    return tuple(v) if v is not None else None


def ctx_from_json(ctx: dict) -> dict:
    """An engine context read back from JSON: the engine wants tuples where JSON has lists."""
    infos = []
    for i in ctx["infos"]:
        i = dict(i)
        i["modes"] = [([_fx(f) for f in fx], bool(unk)) for fx, unk in i["modes"]]
        i["trig"] = [(ev, [_fx(f) for f in fx], filt) for ev, fx, filt in i["trig"]]
        i["eot"] = [_fx(f) for f in i["eot"]]
        i["aura"], i["drs"] = _opt(i["aura"]), _opt(i["drs"])
        infos.append(i)
    return {
        **ctx,
        "infos": infos,
        "hold": [tuple(h) for h in ctx["hold"]],
        "wtrig": [(c, [_fx(f) for f in fx]) for c, fx in ctx["wtrig"]],
        "played_cheap": [tuple(x) for x in ctx["played_cheap"]],
        "aoe_threat": _opt(ctx["aoe_threat"]),
    }


def case_from_json(rec: dict) -> Case:
    """A fixture record (tests/fixtures/cards/*.jsonl.gz) as a Case."""
    return Case(ctx_from_json(rec["ctx"]), rec["kind"], rec["src"], rec["tgt"], rec["hand"])


def case_from_state(st: GameState, db: dict, kind: str, ent: int, tgt: int) -> tuple[Case, Planner]:
    """The Case for one logged action: `ent` acted on `tgt` (entity ids from the log, 0 = no target)."""
    p = Planner(st, db)
    enemy = {e.eid for e in st.enemy_board}
    mine = {e.eid for e in st.board}
    if tgt == 0:
        t = None
    elif tgt == st.hero_id:
        t = HERO
    elif tgt in enemy or tgt in mine:
        t = tgt
    else:
        t = FACE
    src = HERO if (kind == "ATTACK" and ent == st.hero_id) else ent
    ctx = dict(p._rust_ctx())
    ctx.pop("w", None)
    return Case(ctx, kind, src, t, sorted(p.cid), p.held_back), p
