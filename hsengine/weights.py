"""Hand-tuned evaluation weights for the turn search.

Callers may change values in `W` before building a Planner; `hsengine.planner` passes them to the Rust core,
which ignores unknown keys.
"""

W = {
    "base": 0.5,
    "atk": 0.8625,
    "hp": 0.9,
    "body": 0.1,
    "taunt": 1.2,
    "enemy": 1.15,
    "weapon": 0.35,
    "weapon_dur": 0.3,
    "my_hp": 0.45,
    "low_hp": 0.5,
    "en_hp": 0.4,
    "threat": 0.1,
    "hold_minion": 1.6,
    "hold_removal": 3.6,
    "hold_spell": 1.4625,
    "hold_unknown": 2.2,
    "hold_coin": 1.5,
    "draw": 2.2,
    "unknown": 2.0,
    "overload": 1.2,
    "ramp": 2.5,
    "lookahead": 0.4,
    "draw_ahead": 0.0,
    "enemy_removal": 0.0,
    "aoe_risk": 0.0,
    "aoe_model": 0.0,
    "race": 0.0,
    "role": 0.0,
}
