//! Minions entering and leaving the board. `new_unit` summons a unit for me (board limit, keywords,
//! "whenever you summon" triggers). `spawn` is what follows a minion's death: death triggers, its aura
//! removed, Reborn, and the minions or cards its Deathrattle leaves. `afterlife` values that leftover for
//! the evaluation. `op_summon` handles the summon ops of `effects::Planner::apply`.
use super::Call;
use crate::planner::*;
use crate::state::*;
use rustc_hash::FxHashSet;

impl Planner {
    /// The value a dying minion leaves behind: its Reborn copy and Deathrattle tokens.
    pub fn afterlife(&self, u: &U, enemy: bool) -> f64 {
        let mut v = 0.0;
        if u.reborn {
            let mut c = *u;
            c.hp = 1;
            c.maxhp = 1;
            c.divine = false;
            c.reborn = false;
            v += self.uv(&c, enemy);
        }
        if let Some(i) = self.info(u.uid).filter(|_| !self.silenced.contains(&u.uid)) {
            if let Some((n, a, h, kw)) = i.drs {
                let tok = token(0, a, h, kw);
                v += n as f64 * self.uv(&tok, enemy);
            }
        }
        v
    }

    /// Take a dead minion's aura `(attack, health)` off the minions left on its side.
    fn remove_aura(s: &mut S, (da, dh): (i32, i32), mine: bool) {
        let side = if mine { &mut s.mine } else { &mut s.theirs };
        for x in side.iter_mut() {
            x.atk = (x.atk - da).max(0);
            x.hp = (x.hp - dh).max(1);
            x.maxhp = (x.maxhp - dh).max(1);
        }
    }

    /// Bring a dead Reborn minion back with 1 Health, if its side has room.
    fn reborn_copy(&mut self, mut s: S, u: &U, mine: bool) -> S {
        if u.reborn {
            let len = if mine { s.mine.len() } else { s.theirs.len() };
            if len < 7 {
                let nid = self.new_id();
                let mut back = *u;
                back.uid = nid;
                back.hp = 1;
                back.maxhp = 1;
                back.reborn = false;
                back.divine = false;
                back.att = 0;
                back.tmp = 0;
                back.frozen = false;
                back.swung = false;
                if mine {
                    s.mine.push(back)
                } else {
                    s.theirs.push(back)
                }
                if let Some(&ix) = self.info_of.get(&u.uid) {
                    self.info_of.insert(nid, ix);
                }
                let o = *self.origin.get(&u.uid).unwrap_or(&u.uid);
                self.origin.insert(nid, o);
            }
        }
        s
    }

    /// "Deathrattle: your opponent draws a card": my draw, and my "whenever you draw" triggers.
    fn opp_draw_on_death(&mut self, mut s: S, u: &U) -> S {
        let od = match self.info(u.uid) {
            Some(i) if !self.silenced.contains(&u.uid) => i.opp_draw,
            _ => 0,
        };
        for _ in 0..od {
            if 10 - (self.hand_count(&s, &[POWER, LOCACT]) + s.drawn) <= 0 {
                break;
            }
            s.bonus += self.w.draw;
            s.drawn += 1;
            s = self.fire(s, 5, None, false);
        }
        s
    }

    /// "Deathrattle: get a Coin": cards a dead minion of mine puts in my hand, as far as there is room.
    fn deathrattle_cards(&mut self, mut s: S, u: &U) -> S {
        let g = match self.info(u.uid) {
            Some(i) if !self.silenced.contains(&u.uid) => i.dr_gen,
            _ => 0,
        };
        let room = (10 - (self.hand_count(&s, &[POWER, LOCACT]) + s.drawn)).max(0);
        s.drawn += g.min(room);
        s
    }

    /// The tokens a dead minion's Deathrattle summons on its side, as far as there is room.
    fn deathrattle_tokens(&mut self, mut s: S, u: &U, mine: bool) -> S {
        let drs = match self.info(u.uid) {
            Some(i) if !self.silenced.contains(&u.uid) => i.drs,
            _ => None,
        };
        let (n, a, h, kw) = match drs {
            Some(d) => d,
            None => return s,
        };
        let tok_ix = match self.info(u.uid) {
            Some(i) if i.dr_tok != 0 => self.info_of.get(&i.dr_tok).copied(),
            _ => None,
        };
        for _ in 0..n {
            let len = if mine { s.mine.len() } else { s.theirs.len() };
            if len >= 7 {
                break;
            }
            let nid = self.new_id();
            let tok = token(nid, a, h, kw);
            if let Some(ix) = tok_ix {
                self.info_of.insert(nid, ix);
            }
            if mine {
                s.mine.push(tok)
            } else {
                s.theirs.push(tok)
            }
        }
        s
    }

    /// What follows the death of `u` (already off the board, `mine` for my side): death triggers, its aura
    /// removed, Reborn, its Deathrattle draw and summons.
    pub fn spawn(&mut self, mut s: S, u: &U, mine: bool) -> S {
        s.died += 1;
        if mine
            && s.mine.iter().any(|x| {
                self.info(x.uid)
                    .is_some_and(|i| i.trig.iter().any(|t| t.0 == 9))
            })
        {
            s = self.fire(s, 9, Some(u.uid), false); // "whenever a friendly minion dies"
        }
        if mine && self.in_cast {
            self.dying += 1;
        }
        if let Some(aura) = self.info(u.uid).and_then(|i| i.aura) {
            Self::remove_aura(&mut s, aura, mine);
        }
        s = self.reborn_copy(s, u, mine);
        if mine {
            s = self.deathrattle_cards(s, u);
        } else {
            s = self.opp_draw_on_death(s, u);
        }
        self.deathrattle_tokens(s, u, mine)
    }

    /// Summon an `a`/`h` minion with keywords `kw` for me, if there is room. `uid` reuses a known card's id
    /// (so its card model applies); `dr`, `sp` and `pv` set its Deathrattle value, Spell Damage and extra
    /// value. Returns the position and the unit.
    #[allow(clippy::too_many_arguments)]
    pub fn new_unit(
        &mut self,
        mut s: S,
        a: i32,
        h: i32,
        kw: u16,
        uid: Option<i32>,
        dr: f64,
        sp: i32,
        pv: f64,
    ) -> (S, Option<U>) {
        if s.mine.len() as i32 + if self.in_cast { self.dying } else { 0 } >= 7 {
            return (s, None);
        }
        let uid = match uid {
            Some(u) => u,
            None => self.new_id(),
        };
        let wf = kw & K_WF != 0;
        let charge = kw & K_CHARGE != 0;
        let rush = kw & K_RUSH != 0;
        let att = if charge || rush {
            if wf { 2 } else { 1 }
        } else {
            0
        };
        let mut u = U::new(uid, a, h, h);
        u.taunt = kw & K_TAUNT != 0;
        u.divine = kw & K_DIVINE != 0;
        u.att = att;
        u.stealth = kw & K_STEALTH != 0;
        u.rush = rush && !charge;
        u.poison = kw & K_POISON != 0;
        u.life = kw & K_LIFE != 0;
        u.wf = wf;
        u.fresh = true;
        u.dr = dr;
        u.sp = sp;
        u.pv = pv;
        u.reborn = kw & K_REBORN != 0;
        if kw & K_DOOMED != 0 {
            u.doomed = true;
        }
        for x in &s.mine {
            if x.uid == uid {
                continue;
            }
            if let Some(i) = self.info(x.uid) {
                if let Some((aa, ah)) = i.aura {
                    u.atk += aa;
                    u.hp += ah;
                    u.maxhp += ah;
                }
            }
        }
        s.mine.push(u);
        (s, Some(u))
    }

    /// Make one friendly minion a 1/1 (the target, else the weakest other one), then fill the board with 1/1s.
    fn mask_fill(&mut self, mut s: S, t: T, exclude: Option<i32>) -> S {
        let pick = match t {
            T::Unit(uid) => Some(uid),
            _ => s
                .mine
                .iter()
                .filter(|u| Some(u.uid) != exclude)
                .min_by_key(|u| u.atk + u.maxhp)
                .map(|u| u.uid),
        };
        let Some(uid) = pick else { return s };
        if let Some(x) = s.mine.iter_mut().find(|x| x.uid == uid) {
            x.atk = 1;
            x.hp = 1;
            x.maxhp = 1;
        }
        while s.mine.len() < 7 {
            let before = s.mine.len();
            let (ns, _) = self.new_unit(s, 1, 1, 0, None, 0.0, 0, 0.0);
            s = ns;
            if s.mine.len() == before {
                break;
            }
        }
        s
    }

    /// `n` copies of the source minion `src` (its Attack, max Health, Taunt and Divine Shield) for me.
    fn summon_copies(&mut self, mut s: S, n: i32, src: Option<i32>) -> S {
        let me = src.and_then(|e| s.mine.iter().find(|u| u.uid == e).cloned());
        if let Some(me) = me {
            for _ in 0..n {
                let kw =
                    (if me.taunt { K_TAUNT } else { 0 }) | (if me.divine { K_DIVINE } else { 0 });
                let (ns, _) = self.new_unit(s, me.atk, me.maxhp, kw, None, 0.0, 0, 0.0);
                s = ns;
            }
        }
        s
    }

    /// Take the enemy minion `t` for my side until the end of the turn (it cannot attack yet).
    fn steal_temp(&mut self, mut s: S, t: T) -> S {
        if let T::Unit(uid) = t {
            if let Some(u) = s.theirs.iter().find(|x| x.uid == uid).cloned() {
                if s.mine.len() < 7 {
                    self.lent.insert(uid);
                    s.theirs = swap(&s.theirs, uid, None);
                    let mut x = u;
                    x.att = 0;
                    x.fresh = true;
                    s.mine.push(x);
                }
            }
        }
        s
    }

    /// An `a`/`h` minion with keywords `kw` enters the enemy board, if there is room.
    fn enemy_summon(&mut self, mut s: S, a: i32, h: i32, kw: u16) -> S {
        let nid = self.new_id();
        let mut u = U::new(nid, a, h, h);
        u.taunt = kw & K_TAUNT != 0;
        u.fresh = true;
        if s.theirs.len() < 7 {
            s.theirs.push(u);
        }
        s
    }

    /// Minions entering a board.
    pub(crate) fn op_summon(&mut self, mut s: S, op: &str, c: &Call) -> S {
        let Call { f, t, exclude, .. } = *c;
        match op {
            "summon" => {
                for _ in 0..f.n {
                    let (ns, _) = self.new_unit(s, f.a, f.h, f.kw, None, 0.0, 0, 0.0);
                    s = ns;
                }
                s
            }
            "summon_corpse" => {
                if s.corpses <= 0 {
                    return s;
                }
                s.corpses -= 1;
                let (ns, _) = self.new_unit(s, f.a, f.h, f.kw, None, 0.0, 0, 0.0);
                ns
            }
            "dr_all" => {
                for u in s.mine.iter_mut() {
                    u.dr += f.n as f64 / 10.0;
                }
                s
            }
            "summon_played" => {
                let pc: Vec<(i32, i32)> = self.played_cheap.iter().take(7).cloned().collect();
                for (a, h) in pc {
                    let (ns, _) = self.new_unit(s, a, h, 0, None, 0.0, 0, 0.0);
                    s = ns;
                }
                s
            }
            "summon_copies" => self.summon_copies(s, f.n, exclude),
            "summon_adj" => {
                // a token whose aura gives f.n Attack to its neighbours (the Herald Soldier of Al'Akir)
                let before: FxHashSet<i32> = s.mine.iter().map(|u| u.uid).collect();
                let (ns, _) = self.new_unit(s, f.a, f.h, f.kw, None, 0.0, 0, 0.0);
                s = ns;
                if let Some(u) = s.mine.iter().find(|u| !before.contains(&u.uid)) {
                    self.tok_adj.insert(u.uid, f.n);
                    self.has_adj = true;
                }
                s
            }
            "mask_fill" => self.mask_fill(s, t, exclude),
            "steal_temp" => self.steal_temp(s, t),
            "enemy_summon" => self.enemy_summon(s, f.a, f.h, f.kw),
            _ => s,
        }
    }
}
