//! Stat changes. `buff_unit` applies one buff effect to a unit; the op groups `op_buff` (board),
//! `op_self_buff` (the acting minion) and `op_hand_weapon` (cards in hand, hero attack, weapon) are called
//! from `effects::Planner::apply`. Auras are not here: `moves` applies them when the card is played and
//! `summon::spawn` removes them when it dies; adjacency auras are recomputed after every move.
use super::Call;
use crate::planner::*;
use crate::state::*;

impl Planner {
    /// `u` after the buff effect `f` (stats, keywords; a temporary buff with condition 5; condition 7 limits it to minions with a shared type bit).
    pub fn buff_unit(&self, u: &U, f: &Fx) -> U {
        if f.cond == 7 && u.race & f.cond_n as u32 == 0 {
            return *u; // "your Elementals": only minions of that type
        }
        let temp = f.cond == 5;
        let kw = f.kw;
        let mut att = u.att;
        if kw & K_WF != 0 && !u.wf && att > 0 {
            att += 1;
        }
        if (kw & K_CHARGE != 0 || kw & K_RUSH != 0) && u.fresh && att == 0 && !u.swung {
            att = if u.wf || kw & K_WF != 0 { 2 } else { 1 };
        }
        let mut n = *u;
        n.atk = u.atk + f.a;
        n.hp = u.hp + if temp { 0 } else { f.h };
        n.maxhp = u.maxhp + if temp { 0 } else { f.h };
        n.tmp = u.tmp + if temp { f.a } else { 0 };
        n.taunt = u.taunt || kw & K_TAUNT != 0;
        n.divine = u.divine || kw & K_DIVINE != 0;
        n.wf = u.wf || kw & K_WF != 0;
        n.stealth = u.stealth || kw & K_STEALTH != 0;
        n.life = u.life || kw & K_LIFE != 0;
        n.poison = u.poison || kw & K_POISON != 0;
        n.att = att;
        n.rush = u.rush || (kw & K_RUSH != 0 && kw & K_CHARGE == 0 && u.fresh && u.att == 0);
        n.reborn = u.reborn || kw & K_REBORN != 0;
        n
    }

    /// Stat changes on minions on the board.
    pub(crate) fn op_buff(&mut self, mut s: S, op: &str, c: &Call) -> S {
        let Call { f, t, exclude, .. } = *c;
        match op {
            "self_buff_allies" => {
                let n = s.mine.iter().filter(|u| Some(u.uid) != exclude).count() as i32;
                let me = exclude.and_then(|e| s.mine.iter().find(|u| u.uid == e).cloned());
                let me = match me {
                    Some(m) if n != 0 => m,
                    _ => return s,
                };
                let mut x = me;
                x.atk = me.atk + f.a * n;
                x.hp = me.hp + f.h * n;
                x.maxhp = me.maxhp + f.h * n;
                s.mine = swap(&s.mine, me.uid, Some(x));
                s
            }
            "buff" => {
                if t == T::Hero {
                    s.hero_atk += f.a;
                    return s;
                }
                if let T::Unit(uid) = t {
                    if let Some(u) = s.mine.iter().find(|u| u.uid == uid).cloned() {
                        let n = self.buff_unit(&u, f);
                        s.mine = swap(&s.mine, uid, Some(n));
                    }
                }
                s
            }
            "buff_all" => {
                let mine: Vec<U> = s
                    .mine
                    .iter()
                    .map(|u| {
                        if f.scope == 8 && Some(u.uid) == exclude {
                            *u
                        } else {
                            self.buff_unit(u, f)
                        }
                    })
                    .collect();
                s.mine = mine;
                if f.scope == 5 && f.a != 0 && s.hero_att > 0 {
                    s.hero_atk += f.a;
                }
                s
            }
            "enemy_atk_temp" => {
                for u in s.theirs.iter_mut() {
                    u.tmp += f.n.min((u.atk - u.tmp).max(0));
                }
                s
            }
            "buff_all_corpse" => {
                let big = s.corpses >= f.n;
                let a = if big { f.h } else { f.a };
                s.corpses -= if big { f.n } else { 0 };
                for u in s.mine.iter_mut() {
                    u.atk += a;
                }
                s
            }
            "set_atk" => {
                for u in s.theirs.iter_mut() {
                    if T::Unit(u.uid) == t {
                        u.atk = f.n;
                    }
                }
                s
            }
            _ => s,
        }
    }

    /// Stat changes of the acting minion itself.
    pub(crate) fn op_self_buff(&mut self, mut s: S, op: &str, c: &Call) -> S {
        let Call { f, exclude, .. } = *c;
        match op {
            "self_hp_enemy_hand" => {
                let me = match exclude.and_then(|e| s.mine.iter().find(|u| u.uid == e).cloned()) {
                    Some(m) => m,
                    None => return s,
                };
                let loss = f.n * self.enemy_hand;
                let mut n = me;
                n.hp = (me.hp - loss).max(1);
                n.maxhp = (me.maxhp - loss).max(1);
                s.mine = swap(&s.mine, me.uid, Some(n));
                s
            }
            "self_atk" => {
                if let Some(me) = exclude.and_then(|e| s.mine.iter().find(|u| u.uid == e).cloned())
                {
                    let mut x = me;
                    x.atk += f.n;
                    s.mine = swap(&s.mine, me.uid, Some(x));
                }
                s
            }
            "self_hp_hand" => {
                let me = match exclude.and_then(|e| s.mine.iter().find(|u| u.uid == e).cloned()) {
                    Some(m) => m,
                    None => return s,
                };
                let k = f.n * self.hand_count(&s, &[POWER, LOCACT]);
                let mut n = me;
                n.hp = me.hp + k;
                n.maxhp = me.maxhp + k;
                s.mine = swap(&s.mine, me.uid, Some(n));
                s
            }
            "self_buff_hand" => {
                let me = exclude.and_then(|e| s.mine.iter().find(|u| u.uid == e).cloned());
                let n = self.hand_count(&s, &[POWER]);
                if let Some(me) = me {
                    let mut x = me;
                    x.hp = me.hp + f.h * n;
                    x.maxhp = me.maxhp + f.h * n;
                    s.mine = swap(&s.mine, me.uid, Some(x));
                }
                s
            }
            "grow_if_full" => {
                // "if this is at full Health, gain +a/+h"
                if let Some(me) = exclude.and_then(|e| s.mine.iter().find(|u| u.uid == e).cloned())
                {
                    if me.hp >= me.maxhp {
                        let mut x = me;
                        x.atk += f.a;
                        x.hp += f.h;
                        x.maxhp += f.h;
                        s.mine = swap(&s.mine, me.uid, Some(x));
                    }
                }
                s
            }
            "self_stats" => {
                if let Some(me) = exclude.and_then(|e| s.mine.iter().find(|u| u.uid == e).cloned())
                {
                    let mut x = me;
                    x.atk += f.a;
                    x.hp += f.h;
                    x.maxhp += f.h;
                    s.mine = swap(&s.mine, me.uid, Some(x));
                }
                s
            }
            _ => s,
        }
    }

    /// Buffs to cards in hand, the hero's attack and the weapon.
    pub(crate) fn op_hand_weapon(&mut self, mut s: S, op: &str, c: &Call) -> S {
        let Call { f, exclude, .. } = *c;
        match op {
            "hand_buff" => {
                let n = s
                    .hand
                    .iter()
                    .filter(|h| self.info(**h).map(|i| i.kind == MINION).unwrap_or(false))
                    .count() as i32;
                s.bonus += 0.5 * (f.a + f.h) as f64 * n as f64;
                s
            }
            "hero_atk" => {
                s.hero_atk += f.n;
                s
            }
            "weapon" => {
                s.hero_atk = s.hero_atk - s.watk + f.a;
                s.watk = f.a;
                s.wdur = f.h;
                s
            }
            "destroy_weapon" => {
                s.bonus += 1.0 + 1.2 * self.enemy_swing as f64;
                s
            }
            "weapon_buff" => {
                if s.wdur > 0 {
                    s.hero_atk += f.n;
                    s.watk += f.n;
                }
                s
            }
            "buff_name" => {
                // buff my other minions that share the source's name (the Rafaams): +a/+h
                let me = match exclude.and_then(|e| self.info(e)) {
                    Some(i) => i.name,
                    None => return s,
                };
                let ids: Vec<i32> = s
                    .mine
                    .iter()
                    .filter(|u| Some(u.uid) != exclude)
                    .map(|u| u.uid)
                    .collect();
                for uid in ids {
                    if self.info(uid).is_some_and(|i| i.name == me) {
                        if let Some(x) = s.mine.iter_mut().find(|x| x.uid == uid) {
                            x.atk += f.a;
                            x.hp += f.h;
                            x.maxhp += f.h;
                        }
                    }
                }
                s
            }
            _ => s,
        }
    }
}
