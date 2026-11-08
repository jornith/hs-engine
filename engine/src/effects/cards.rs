//! Card and resource effects: draws (including those that depend on a weapon break, a kill, a survivor or
//! deaths), Discover and generated cards (scored, since their identity is unknown to the search),
//! discards, mana, Corpses, minions returned to hand, and a Battlecry minion shuffled back into the deck. One op group,
//! `op_cards`, called from `effects::Planner::apply`. Drawing also fires the "whenever you draw"
//! triggers through `triggers::fire`.
use super::Call;
use crate::planner::*;
use crate::state::*;

impl Planner {
    /// Draw up to `n` cards (a full hand burns the rest without a penalty), firing the draw triggers.
    fn draw_free(&mut self, mut s: S, n: i32) -> S {
        let room = (10 - (self.hand_count(&s, &[POWER, LOCACT]) + s.drawn)).max(0);
        let k = n.min(room);
        s.bonus += self.w.draw * k as f64;
        s.drawn += k;
        for _ in 0..k {
            s = self.fire(s, 5, None, false);
        }
        s
    }

    /// `n` damage to every minion on both sides.
    fn hit_all_minions(&mut self, mut s: S, n: i32) -> S {
        let ids: Vec<i32> = s.mine.iter().map(|u| u.uid).collect();
        for uid in ids {
            s = self.dmg_my_unit(s, uid, n);
        }
        let ids: Vec<i32> = s.theirs.iter().map(|u| u.uid).collect();
        for uid in ids {
            s = self.dmg_enemy_unit(s, uid, n, false);
        }
        s
    }

    /// Damage a minion: if it survives, draw a card; if it dies, summon an `a`/`h` minion for me.
    fn dmg_surv(&mut self, mut s: S, c: &Call) -> S {
        let uid = match c.t {
            T::Unit(u) => u,
            _ => return s,
        };
        let n = c.f.n + c.sp;
        s = if s.mine.iter().any(|u| u.uid == uid) {
            self.dmg_my_unit(s, uid, n)
        } else {
            self.dmg_enemy_unit(s, uid, n, false)
        };
        if s.mine.iter().chain(s.theirs.iter()).any(|u| u.uid == uid) {
            self.draw_free(s, 1)
        } else {
            self.new_unit(s, c.f.a, c.f.h, 0, None, 0.0, 0, 0.0).0
        }
    }

    /// Cards and resources: draws, generated cards, discards, mana, Corpses, returning minions to hand.
    pub(crate) fn op_cards(&mut self, mut s: S, op: &str, c: &Call) -> S {
        let Call { f, t, exclude, .. } = *c;
        match op {
            "opp_cards" => {
                s.bonus -= 1.3 * f.n as f64;
                s
            }
            "draw" => {
                // a full hand (10) burns the rest: no card, no "whenever you draw" trigger
                let room = (10 - (self.hand_count(&s, &[POWER, LOCACT]) + s.drawn)).max(0);
                let k = f.n.min(room);
                s.bonus = s.bonus + self.w.draw * k as f64 - 1.0 * (f.n - k) as f64;
                s.drawn += k;
                for _ in 0..k {
                    s = self.fire(s, 5, None, false);
                }
                s
            }
            "draw_brk" => {
                // the weapon broke with this swing: draw
                if s.wdur <= 0 {
                    s = self.draw_free(s, f.n);
                }
                s
            }
            "draw_kill" => {
                // the hero's swing killed a minion: draw
                if self.hero_killed {
                    s = self.draw_free(s, f.n);
                }
                s
            }
            "dmg_surv" => self.dmg_surv(s, c),
            "aoe_draw" => {
                // damage every minion, draw a card for each that died
                let before = (s.mine.len() + s.theirs.len()) as i32;
                s = self.hit_all_minions(s, f.n + c.sp);
                let died = (before - (s.mine.len() + s.theirs.len()) as i32).max(0);
                self.draw_free(s, died)
            }
            "shuffle_self" => {
                // "Shuffle this into your deck" (Battlecry minion): it leaves the board at once
                if let Some(e) = exclude {
                    s.mine.retain(|u| u.uid != e);
                }
                s
            }
            "corpse" => {
                s.corpses += f.n;
                s
            }
            "draw_to" => {
                let cards = self.hand_count(&s, &[POWER, LOCACT]) + s.drawn;
                let k = (f.n - cards).max(0);
                s.bonus += self.w.draw * k as f64;
                s.drawn += k;
                s
            }
            "mana" => {
                s.mana += f.n;
                s
            }
            "ramp" => {
                s.bonus += if self.max_mana < 10 { self.w.ramp } else { 0.0 };
                s
            }
            "discover" => {
                let room = (10 - (self.hand_count(&s, &[POWER, LOCACT]) + s.drawn)).max(0);
                let k = f.n.min(room);
                s.bonus += self.w.draw * k as f64;
                s.drawn += k;
                s
            }
            "gen" => {
                let room = (10 - (self.hand_count(&s, &[POWER, LOCACT]) + s.drawn)).max(0);
                s.drawn += f.n.min(room);
                s
            }
            "value" => {
                s.bonus += f.n as f64 / 10.0;
                s
            }
            "discard" => {
                let others = self.hand_count(&s, &[POWER]);
                s.bonus -= 2.0 * f.n.min(others) as f64;
                s
            }
            "bounce" => {
                if let T::Unit(uid) = t {
                    s.theirs = swap(&s.theirs, uid, None);
                } else {
                    s.theirs = swap(&s.theirs, i32::MIN, None);
                }
                s.bonus -= 1.5;
                s
            }
            "bounce_all" => {
                let mut lost = 0.0;
                for u in &s.theirs {
                    lost += self.uv(u, true);
                }
                s.theirs.clear();
                s.bonus -= self.w.enemy * lost * 0.5;
                s
            }
            _ => s,
        }
    }
}
