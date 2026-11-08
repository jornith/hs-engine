//! Evaluation weights. [`Wt`] has one field per key of the Python `W` dict (hsengine/weights.py), which
//! holds the hand-tuned values. The bindings fill it with [`Wt::set`]; unknown names are ignored, so the
//! Python side may carry keys the core does not read. `eval` and `reply` read the fields.

/// Handcrafted evaluation weights, one field per `W` key.
#[derive(Clone, Default, Debug)]
pub struct Wt {
    pub base: f64,
    pub atk: f64,
    pub hp: f64,
    pub body: f64,
    pub taunt: f64,
    pub enemy: f64,
    pub weapon: f64,
    pub weapon_dur: f64,
    pub my_hp: f64,
    pub low_hp: f64,
    pub en_hp: f64,
    pub threat: f64,
    pub draw: f64,
    pub unknown: f64,
    pub overload: f64,
    pub ramp: f64,
    pub lookahead: f64,
    pub draw_ahead: f64,
    pub enemy_removal: f64,
    pub aoe_risk: f64,
    pub aoe_model: f64,
    pub race: f64,
    pub role: f64,
}

impl Wt {
    /// Set one weight by its `W` key. Returns false for an unknown key.
    pub fn set(&mut self, k: &str, v: f64) -> bool {
        let f = match k {
            "base" => &mut self.base,
            "atk" => &mut self.atk,
            "hp" => &mut self.hp,
            "body" => &mut self.body,
            "taunt" => &mut self.taunt,
            "enemy" => &mut self.enemy,
            "weapon" => &mut self.weapon,
            "weapon_dur" => &mut self.weapon_dur,
            "my_hp" => &mut self.my_hp,
            "low_hp" => &mut self.low_hp,
            "en_hp" => &mut self.en_hp,
            "threat" => &mut self.threat,
            "draw" => &mut self.draw,
            "unknown" => &mut self.unknown,
            "overload" => &mut self.overload,
            "ramp" => &mut self.ramp,
            "lookahead" => &mut self.lookahead,
            "draw_ahead" => &mut self.draw_ahead,
            "enemy_removal" => &mut self.enemy_removal,
            "aoe_risk" => &mut self.aoe_risk,
            "aoe_model" => &mut self.aoe_model,
            "race" => &mut self.race,
            "role" => &mut self.role,
            _ => return false,
        };
        *f = v;
        true
    }
}
