"""Read the parsed-position caches written by sim_check.py: (stamp, [(pre_state, kind, ent, tgt, real), ...]).

The unpickler maps any `*.state.GameState` / `*.state.Ent` class onto hsengine's and keeps only the fields
hsengine knows, so caches written by older versions of the state module still load.
"""

import dataclasses
import pickle
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from hsengine.state import Ent, GameState  # noqa: E402

_TARGET = {"GameState": GameState, "Ent": Ent}


class _Raw:
    def __setstate__(self, st):
        d = {}
        if isinstance(st, tuple):
            for part in st:
                d.update(part or {})
        else:
            d.update(st or {})
        self.d = d


class _Unpickler(pickle.Unpickler):
    def find_class(self, module, name):
        if module.endswith("state") and name in _TARGET:
            return type(name, (_Raw,), {"target": _TARGET[name]})
        return super().find_class(module, name)


def _convert(o):
    if isinstance(o, _Raw):
        cls = o.target
        names = {f.name for f in dataclasses.fields(cls)}
        return cls(**{k: _convert(v) for k, v in o.d.items() if k in names})
    if isinstance(o, list):
        return [_convert(x) for x in o]
    if isinstance(o, tuple):
        return tuple(_convert(x) for x in o)
    return o


def load(path: str | Path) -> list[tuple]:
    """The cached events of one game file."""
    with open(path, "rb") as fh:
        got = _Unpickler(fh).load()
    return _convert(got[1])
