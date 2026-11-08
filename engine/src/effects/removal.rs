//! Removal: destroy, transform, silence and freeze, single-target and board-wide. The op groups
//! `op_destroy`, `op_mass_removal` and `op_silence_freeze` are called from `effects::Planner::apply`;
//! the deaths they cause go through the same paths as deaths from damage.
use super::Call;
use crate::planner::*;
use crate::state::*;

impl Planner {
    /// Single-target and chosen removal.
    pub(crate) fn op_destroy(&mut self, mut s: S, op: &str, c: &Call) -> S {
        let Call { f, t, exclude, .. } = *c;
        match op {
            "destroy" => {
                if let T::Unit(uid) = t {
                    if let Some(u) = s.theirs.iter().find(|u| u.uid == uid).cloned() {
                        s.theirs = swap(&s.theirs, uid, None);
                        s.bonus -= u.dr;
                        return self.spawn(s, &u, false);
                    }
                    if let Some(u) = s.mine.iter().find(|u| u.uid == uid).cloned() {
                        // my own minion (any-minion targets): same as it dying in combat
                        s.mine = swap(&s.mine, uid, None);
                        s.bonus += u.dr;
                        if self.dk {
                            s.corpses += 1;
                        }
                        return self.spawn(s, &u, true);
                    }
                }
                s
            }
            "destroy_highest" => {
                if s.theirs.is_empty() {
                    return s;
                }
                let mut best = s.theirs[0];
                let mut bv = self.uv(&best, false);
                for u in s.theirs.iter().skip(1) {
                    let v = self.uv(u, false);
                    if u.atk > best.atk || (u.atk == best.atk && v > bv) {
                        best = *u;
                        bv = v;
                    }
                }
                s.theirs = swap(&s.theirs, best.uid, None);
                s.bonus -= best.dr;
                self.spawn(s, &best, false)
            }
            "eat_right" => {
                // "destroy the minion to the right to gain +a/+h"
                if let Some(ix) = exclude.and_then(|e| s.mine.iter().position(|u| u.uid == e)) {
                    if ix + 1 < s.mine.len() {
                        let gone = s.mine[ix + 1].uid;
                        let mut x = s.mine[ix];
                        x.atk += f.a;
                        x.hp += f.h;
                        x.maxhp += f.h;
                        s.mine = swap(&s.mine, x.uid, Some(x));
                        s.mine = swap(&s.mine, gone, None);
                    }
                }
                s
            }
            "destroy_random" => {
                if s.theirs.is_empty() {
                    return s;
                }
                let mut best = s.theirs[0];
                let mut bv = self.uv(&best, false);
                for u in s.theirs.iter().skip(1) {
                    let v = self.uv(u, false);
                    if v < bv {
                        best = *u;
                        bv = v;
                    }
                }
                s.theirs = swap(&s.theirs, best.uid, None);
                s.bonus -= best.dr;
                self.spawn(s, &best, false)
            }
            "doom_all" => {
                for u in s.mine.iter_mut() {
                    u.doomed = true;
                }
                s
            }
            _ => s,
        }
    }

    /// Board-wide removal and transformation.
    pub(crate) fn op_mass_removal(&mut self, mut s: S, op: &str, c: &Call) -> S {
        let Call { f, t, exclude, .. } = *c;
        match op {
            "destroy_all" => {
                if f.cond == 1 {
                    let k = f.cond_n;
                    let dead_m: Vec<U> = s
                        .mine
                        .iter()
                        .filter(|u| u.atk - u.tmp >= k)
                        .cloned()
                        .collect();
                    let dead_t: Vec<U> = s.theirs.iter().filter(|u| u.atk >= k).cloned().collect();
                    let mut sm = 0.0;
                    for u in &dead_m {
                        sm += u.dr;
                    }
                    let mut st = 0.0;
                    for u in &dead_t {
                        st += u.dr;
                    }
                    s.mine.retain(|u| !dead_m.contains(u));
                    s.theirs.retain(|u| !dead_t.contains(u));
                    s.bonus = s.bonus + sm - st;
                    return s;
                }
                let mut sm = 0.0;
                for u in s.mine.iter().filter(|u| Some(u.uid) != exclude) {
                    sm += u.dr;
                }
                let mut st = 0.0;
                for u in &s.theirs {
                    st += u.dr;
                }
                s.theirs.clear();
                s.mine.retain(|u| Some(u.uid) == exclude);
                s.bonus = s.bonus + sm - st;
                s
            }
            "transform" => {
                for u in s.theirs.iter_mut() {
                    if T::Unit(u.uid) == t {
                        let mut n = U::new(u.uid, f.a, f.h, f.h);
                        n.taunt = f.kw & K_TAUNT != 0;
                        *u = n;
                    }
                }
                s
            }
            "transform_all" => {
                for u in s.mine.iter_mut() {
                    if Some(u.uid) != exclude {
                        *u = U::new(u.uid, f.a, f.h, f.h);
                    }
                }
                for u in s.theirs.iter_mut() {
                    *u = U::new(u.uid, f.a, f.h, f.h);
                }
                s
            }
            _ => s,
        }
    }

    /// Freeze a minion and its neighbours; destroy any that are damaged.
    fn bitter_freeze(&mut self, mut s: S, t: T) -> S {
        let uid = match t {
            T::Unit(u) => u,
            _ => return s,
        };
        let mine = s.mine.iter().any(|u| u.uid == uid);
        let side = if mine { &s.mine } else { &s.theirs };
        let ix = match side.iter().position(|u| u.uid == uid) {
            Some(i) => i,
            None => return s,
        };
        let hit: Vec<U> = side[ix.saturating_sub(1)..=(ix + 1).min(side.len() - 1)].to_vec();
        for u in hit {
            if u.hp < u.maxhp {
                if mine {
                    s.mine = swap(&s.mine, u.uid, None);
                    s.bonus += u.dr;
                } else {
                    s.theirs = swap(&s.theirs, u.uid, None);
                    s.bonus -= u.dr;
                }
                s = self.spawn(s, &u, mine);
            } else {
                let side = if mine { &mut s.mine } else { &mut s.theirs };
                if let Some(x) = side.iter_mut().find(|x| x.uid == u.uid) {
                    x.frozen = true;
                }
            }
        }
        s
    }

    /// Silence the minion `t`: its text and enchantments go, only the printed stats stay.
    fn silence_unit(&mut self, mut s: S, t: T) -> S {
        if let T::Unit(uid) = t {
            self.silenced.insert(uid);
            // buffs go too: the printed stats, health no higher than the printed one (8/9 -> 6/6, 3/10 -> 3/1)
            let printed = self
                .info_of
                .get(&uid)
                .map(|&i| (self.infos[i].atk, self.infos[i].hp))
                .filter(|&(_, h)| h > 0);
            for u in s.theirs.iter_mut().chain(s.mine.iter_mut()) {
                if u.uid == uid {
                    if let Some((a, h)) = printed {
                        u.atk = a;
                        u.maxhp = h;
                        u.hp = u.hp.min(h);
                    }
                    u.taunt = false;
                    u.divine = false;
                    u.dr = 0.0;
                    u.poison = false;
                    u.life = false;
                    u.wf = false;
                    u.frozen = false;
                    u.stealth = false;
                    u.reborn = false;
                    u.doomed = false;
                    u.pv = 0.0;
                    u.sp = 0;
                }
            }
        }
        s
    }

    /// Silence and Freeze.
    pub(crate) fn op_silence_freeze(&mut self, mut s: S, op: &str, c: &Call) -> S {
        let Call { t, .. } = *c;
        match op {
            "bitter" => self.bitter_freeze(s, t),
            "silence" => self.silence_unit(s, t),
            "freeze" => {
                for u in s.theirs.iter_mut() {
                    if T::Unit(u.uid) == t {
                        u.frozen = true;
                    }
                }
                s
            }
            "freeze_all" => {
                for u in s.theirs.iter_mut() {
                    u.frozen = true;
                }
                s
            }
            _ => s,
        }
    }
}
