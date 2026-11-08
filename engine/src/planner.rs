//! The planner: the card models of one turn and the bookkeeping the rules share during its search.
//! [`Planner`] is built once per turn (by `hsengine` through the Python bindings, or by `game::setup` in
//! self-play) and then passed to every move, effect and evaluation as `&mut self`. The other modules add
//! their methods to it: `moves`, `effects`, `eval`, `reply`. Positions themselves are plain values
//! ([`crate::state::S`]), copied on every move.
use crate::state::*;
use crate::weights::Wt;
use rustc_hash::{FxHashMap, FxHashSet};

/// The transposition key of a position (see [`Planner::key`]).
pub type Key = Vec<i64>;

/// The card models of one turn and the state the rules share during its search. It holds the weights,
/// the compiled cards ([`Info`]) by unit id, per-card flags the rules check (corpse costs, Dormant,
/// towers and so on), facts about the opponent for the enemy reply, the id counter for new units and the
/// evaluation caches. One Planner serves one turn; the search mutates only its caches and counters.
pub struct Planner {
    pub w: Wt,
    pub infos: std::sync::Arc<Vec<Info>>,
    pub info_of: FxHashMap<i32, usize>,
    pub hold: FxHashMap<i32, f64>,
    pub corpse_cost: FxHashSet<i32>,
    pub death_discount: FxHashSet<i32>,
    pub maiev: FxHashSet<i32>,
    pub dormant: FxHashSet<i32>,
    pub towers: FxHashSet<i32>,
    pub enemy_towers: FxHashSet<i32>,
    pub enemy_nozdormu: FxHashSet<i32>,
    pub enemy_thoret: bool,
    pub wtrig: FxHashMap<i32, Vec<Fx>>,
    pub wlife: FxHashSet<i32>,
    pub sp_extra: i32,
    pub enemy_split: i32,
    pub played_cheap: Vec<(i32, i32)>,
    pub max_hp: i32,
    pub max_mana: i32,
    pub enemy_hand: i32,
    pub role: i32,
    pub aoe_threat: Option<(i32, f64)>,
    pub enemy_swing: i32,
    pub enemy_direct: i32,
    pub dk: bool,
    pub power_uid: Option<i32>,
    pub draw_uid: i32,
    pub uid: i32,
    pub eval_cache: FxHashMap<Key, f64>,
    pub ev_cache: FxHashMap<Key, f64>,
    pub lent: FxHashSet<i32>, // their minions I control until the end of their turn
    pub silenced: FxHashSet<i32>, // silenced during the search: no deathrattle / trigger / end of turn
    pub iwa: FxHashSet<i32>,      // my minions that take no damage while attacking
    pub cast_sp: i32, // Spell Damage the played spell itself carries (Kalecgos-like buffs in hand)
    pub in_cast: bool,
    /// the hero's swing being resolved killed the minion it hit (read by `draw_kill`)
    pub hero_killed: bool,
    /// a minion's Battlecry is being cast: Elusive minions can be targeted
    pub cast_minion: bool, // death phase: inside one card's resolution
    pub dying: i32, // my minions killed during it (they keep their place)
    pub tok_adj: FxHashMap<i32, i32>, // summoned token -> its adjacent-attack aura
    pub has_adj: bool,
    pub origin: FxHashMap<i32, i32>, // reborn copy -> the minion it came back from
}

/// `round(x, 3) * 1000` as Python computes it, so keys match the Python side exactly.
pub fn round3(x: f64) -> i64 {
    // round(x, 3) as Python does it: correctly rounded; only near-ties need the exact (string) path
    let y = x * 1000.0;
    let frac = (y - y.trunc()).abs();
    if (frac - 0.5).abs() > 1e-6 {
        return y.round() as i64;
    }
    let s = format!("{:.3}", x);
    let v: f64 = s.parse().unwrap_or(x);
    (v * 1000.0).round() as i64
}

impl Planner {
    /// A planner for `cards` (uid, model) with empty special-card sets, 30 max health and 10 max mana.
    /// Callers fill the public fields that their position needs.
    pub fn new(w: Wt, cards: Vec<(i32, Info)>) -> Planner {
        let mut infos = Vec::with_capacity(cards.len());
        let mut info_of = FxHashMap::default();
        for (uid, i) in cards {
            info_of.insert(uid, infos.len());
            infos.push(i);
        }
        let has_adj = infos.iter().any(|i| i.adj != 0);
        Planner {
            w,
            infos: std::sync::Arc::new(infos),
            info_of,
            hold: FxHashMap::default(),
            corpse_cost: FxHashSet::default(),
            death_discount: FxHashSet::default(),
            maiev: FxHashSet::default(),
            dormant: FxHashSet::default(),
            towers: FxHashSet::default(),
            enemy_towers: FxHashSet::default(),
            enemy_nozdormu: FxHashSet::default(),
            enemy_thoret: false,
            wtrig: FxHashMap::default(),
            wlife: FxHashSet::default(),
            sp_extra: 0,
            enemy_split: 0,
            played_cheap: Vec::new(),
            max_hp: 30,
            max_mana: 10,
            enemy_hand: 0,
            role: 0,
            aoe_threat: None,
            enemy_swing: 0,
            enemy_direct: 0,
            dk: false,
            power_uid: None,
            draw_uid: -777,
            uid: -1000,
            eval_cache: FxHashMap::default(),
            ev_cache: FxHashMap::default(),
            lent: FxHashSet::default(),
            silenced: FxHashSet::default(),
            iwa: FxHashSet::default(),
            cast_sp: 0,
            in_cast: false,
            hero_killed: false,
            cast_minion: false,
            dying: 0,
            tok_adj: FxHashMap::default(),
            has_adj,
            origin: FxHashMap::default(),
        }
    }

    /// The card model of unit or card `uid`.
    pub fn info(&self, uid: i32) -> Option<&Info> {
        self.info_of.get(&uid).map(|&i| &self.infos[i])
    }
}
