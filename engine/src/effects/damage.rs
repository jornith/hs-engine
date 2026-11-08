//! Damage to minions and heroes and what follows from it: divine shields, poison, lifesteal, the
//! "took damage" triggers and deaths. `dmg_enemy_unit` and `dmg_my_unit` are the entry points every damage
//! effect, attack and trigger goes through; the op groups here (`op_damage`, `op_damage_random`,
//! `op_health`) are called from `effects::Planner::apply`. Healing lives here too because it mirrors damage.
use super::Call;
use crate::planner::*;
use crate::state::*;

impl Planner {
    /// What happens after an enemy minion took damage and survived (a damaged enemy tower summons two 1/1s).
    pub fn their_hurt(&mut self, mut s: S, old: &U, new: Option<U>, d: i32) -> S {
        if new.is_some() && self.enemy_towers.contains(&old.uid) && d > 0 && !old.divine {
            for _ in 0..2 {
                if s.theirs.len() >= 7 {
                    break;
                }
                let nid = self.new_id();
                let mut g = U::new(nid, 1, 1, 1);
                g.fresh = true;
                s.theirs.push(g);
            }
        }
        s
    }

    /// `d` damage (or poison) to the enemy unit `uid`, with its death and Deathrattle.
    pub fn dmg_enemy_unit(&mut self, mut s: S, uid: i32, d: i32, poison: bool) -> S {
        let old = match s.theirs.iter().find(|u| u.uid == uid) {
            Some(u) => *u,
            None => return s,
        };
        let new = hit(&old, d, poison);
        s.theirs = swap(&s.theirs, uid, new);
        s.bonus -= if new.is_none() { old.dr } else { 0.0 };
        s = self.their_hurt(s, &old, new, d);
        if new.is_none() {
            self.spawn(s, &old, false)
        } else {
            s
        }
    }

    /// What happens after my minion took damage: a corpse when it died (Death Knight), two 1/1s from a damaged
    /// tower, and its "took damage" triggers.
    pub fn my_hurt(&mut self, mut s: S, old: &U, new: Option<U>, d: i32, my_turn: bool) -> S {
        if new.is_none() && self.dk {
            s.corpses += 1;
        }
        if new.is_some() && self.towers.contains(&old.uid) && d > 0 && !old.divine {
            for _ in 0..2 {
                let kw = if my_turn { K_CHARGE | K_DOOMED } else { 0 };
                let (ns, _) = self.new_unit(s, 1, 1, kw, None, 0.0, 0, 0.0);
                s = ns;
            }
        }
        if new.is_some() && d > 0 && !old.divine {
            s = self.fire_unit(s, old.uid, 10);
        }
        s
    }

    /// `d` damage to my unit `uid`, with its death and Deathrattle.
    pub fn dmg_my_unit(&mut self, mut s: S, uid: i32, d: i32) -> S {
        let old = match s.mine.iter().find(|u| u.uid == uid) {
            Some(u) => *u,
            None => return s,
        };
        let new = hit(&old, d, false);
        s.mine = swap(&s.mine, uid, new);
        s.bonus += if new.is_none() { old.dr } else { 0.0 };
        s = self.my_hurt(s, &old, new, d, true);
        if new.is_none() {
            self.spawn(s, &old, true)
        } else {
            s
        }
    }

    /// `n` damage to every enemy minion except the target `t`, and to the enemy hero unless that is the target.
    fn aoe_all_but(&mut self, mut s: S, n: i32, t: T) -> S {
        let ids: Vec<i32> = s.theirs.iter().map(|u| u.uid).collect();
        for uid in ids {
            if T::Unit(uid) != t {
                s = self.dmg_enemy_unit(s, uid, n, false);
            }
        }
        if t != T::Face {
            s.en_hp -= n;
        }
        s
    }

    /// `n` damage to the sides picked by `sc`: 2..=7 the enemy minions, 4..=7 my minions (not `exclude`),
    /// 3, 5 and 6 the enemy hero, 5 and 6 my hero.
    fn aoe_scoped(&mut self, mut s: S, n: i32, sc: u8, exclude: Option<i32>) -> S {
        if matches!(sc, 2..=7) {
            let ids: Vec<i32> = s.theirs.iter().map(|u| u.uid).collect();
            for uid in ids {
                s = self.dmg_enemy_unit(s, uid, n, false);
            }
        }
        if matches!(sc, 4..=7) {
            let ids: Vec<i32> = s.mine.iter().map(|u| u.uid).collect();
            for uid in ids {
                if Some(uid) != exclude {
                    s = self.dmg_my_unit(s, uid, n);
                }
            }
        }
        if matches!(sc, 3 | 5 | 6) {
            s.en_hp -= n;
        }
        if matches!(sc, 5 | 6) {
            s = self.hurt_me(s, n);
        }
        s
    }

    /// `n` damage to all enemy minions with Lifesteal: heal for the damage actually dealt.
    fn aoe_lifesteal(&mut self, mut s: S, n: i32) -> S {
        let healed: i32 = s
            .theirs
            .iter()
            .filter(|u| !u.divine && !u.immune)
            .map(|u| n.min(u.hp).max(0))
            .sum();
        let ids: Vec<i32> = s.theirs.iter().map(|u| u.uid).collect();
        for uid in ids {
            s = self.dmg_enemy_unit(s, uid, n, false);
        }
        if healed > 0 {
            s = self.heal_me(s, healed);
        }
        s
    }

    /// `n` damage to the target minion and to every other minion that shares a type with it, on both sides.
    fn dmg_same_race(&mut self, mut s: S, n: i32, t: T) -> S {
        let uid = match t {
            T::Unit(u) => u,
            _ => return s,
        };
        let race = s
            .mine
            .iter()
            .chain(s.theirs.iter())
            .find(|u| u.uid == uid)
            .map_or(0, |u| u.race);
        let hit_ids = |side: &[U]| -> Vec<i32> {
            side.iter()
                .filter(|u| u.uid == uid || u.race & race != 0)
                .map(|u| u.uid)
                .collect()
        };
        let (mine_ids, their_ids) = (hit_ids(&s.mine), hit_ids(&s.theirs));
        for id in mine_ids {
            s = self.dmg_my_unit(s, id, n);
        }
        for id in their_ids {
            s = self.dmg_enemy_unit(s, id, n, false);
        }
        s
    }

    /// Direct damage: targeted, to the face, to the own hero, to whole boards.
    /// Damage the target minion; while the victim dies, cast again on another enemy minion until one
    /// survives. The game picks the next victim at random; this takes the lowest-health one.
    fn dmg_chain(&mut self, mut s: S, n: i32, t: T) -> S {
        let mut uid = match t {
            T::Unit(u) => u,
            _ => return s,
        };
        loop {
            s = self.dmg_enemy_unit(s, uid, n, false);
            if s.theirs.iter().any(|u| u.uid == uid) {
                return s;
            }
            let next = s
                .theirs
                .iter()
                .filter(|u| !(u.immune && u.stealth) && !u.elusive)
                .min_by_key(|u| u.hp)
                .map(|u| u.uid);
            match next {
                Some(nu) => uid = nu,
                None => return s,
            }
        }
    }

    /// `n` damage dealt one point at a time to enemy minions; the game picks at random, this always takes
    /// the lowest-health one. With `lifesteal`, my hero heals for every point that landed.
    fn dmg_split(&mut self, mut s: S, n: i32, lifesteal: bool) -> S {
        let mut dealt = 0;
        for _ in 0..n {
            let pick = s
                .theirs
                .iter()
                .filter(|u| !(u.immune || u.stealth))
                .min_by_key(|u| u.hp)
                .map(|u| u.uid);
            match pick {
                Some(uid) => {
                    s = self.dmg_enemy_unit(s, uid, 1, false);
                    dealt += 1;
                }
                None => break,
            }
        }
        if lifesteal && dealt > 0 {
            s = self.heal_me(s, dealt);
        }
        s
    }

    /// `f.a` times: `d` damage to one random enemy minion, the excess of a kill hits the enemy hero; no
    /// minion left means the whole hit goes to the hero. The random pick is a fixed policy chosen with
    /// `f.h` (+10: no overflow to the hero, no minion means no effect): 0 lowest health, 1 highest health,
    /// 2 leftmost, 3 rightmost, 4 highest attack.
    fn dmg_excess(&mut self, mut s: S, d: i32, f: &Fx) -> S {
        for _ in 0..f.a.max(1) {
            let cand = s.theirs.iter().filter(|u| !(u.immune || u.stealth));
            let pick = match f.h % 10 {
                1 => cand.max_by_key(|u| u.hp).map(|u| u.uid),
                2 => cand.map(|u| u.uid).next(),
                3 => cand.map(|u| u.uid).next_back(),
                4 => cand.max_by_key(|u| u.atk).map(|u| u.uid),
                _ => cand.min_by_key(|u| u.hp).map(|u| u.uid),
            };
            let Some(uid) = pick else {
                if f.h < 10 {
                    s.en_hp -= d;
                }
                continue;
            };
            let old = *s.theirs.iter().find(|u| u.uid == uid).unwrap();
            let killed = hit(&old, d, false).is_none();
            s = self.dmg_enemy_unit(s, uid, d, false);
            if killed && !old.divine && d > old.hp && f.h < 10 {
                s.en_hp -= d - old.hp;
            }
        }
        s
    }

    pub(crate) fn op_damage(&mut self, mut s: S, op: &str, c: &Call) -> S {
        let Call {
            f, t, exclude, sp, ..
        } = *c;
        match op {
            "dmg" => {
                let n = f.n + sp;
                if t == T::Face {
                    s.en_hp -= n;
                    return s;
                }
                match t {
                    T::Unit(uid) if s.mine.iter().any(|u| u.uid == uid) => {
                        self.dmg_my_unit(s, uid, n)
                    }
                    T::Unit(uid) => self.dmg_enemy_unit(s, uid, n, false),
                    _ => s,
                }
            }
            "face" => {
                s.en_hp = s.en_hp - f.n - sp;
                s
            }
            "self_dmg" => self.hurt_me(s, f.n),
            "halve_enemy" => {
                s.en_hp /= 2;
                s
            }
            "dmg_same_race" => self.dmg_same_race(s, f.n + sp, t),
            "aoe" if f.scope == 1 => self.aoe_all_but(s, f.n + sp, t),
            "aoe" => self.aoe_scoped(s, f.n + sp, f.scope, exclude),
            "aoe_ls" => self.aoe_lifesteal(s, f.n + sp),
            "dmg_chain" => self.dmg_chain(s, f.n + sp, t),
            "dmg_split" => self.dmg_split(s, f.n + sp, f.a == 1),
            "dmg_excess" => self.dmg_excess(s, f.n + sp, f),
            _ => s,
        }
    }

    /// Damage split at random, and damage chosen by the board (lowest, random attacker).
    pub(crate) fn op_damage_random(&mut self, mut s: S, op: &str, c: &Call) -> S {
        let Call { f, exclude, sp, .. } = *c;
        match op {
            "dmg_random" => {
                if s.theirs.is_empty() {
                    s.en_hp = s.en_hp - f.n - sp;
                    return s;
                }
                s.bonus += 0.5 * (f.n + sp) as f64;
                s
            }
            "dmg_random_minions" => {
                let hitl: Vec<i32> = s
                    .theirs
                    .iter()
                    .filter(|u| !u.stealth && !u.immune)
                    .map(|u| u.uid)
                    .collect();
                if !hitl.is_empty() && hitl.len() as i32 <= f.a {
                    for uid in hitl {
                        s = self.dmg_enemy_unit(s, uid, f.n + sp, false);
                    }
                    return s;
                }
                if !hitl.is_empty() {
                    s.bonus += 0.6 * (f.n + sp) as f64 * f.a as f64;
                }
                s
            }
            "attack_random" => {
                // the source minion attacks an enemy minion (the first one: which is random in the game)
                if let (Some(me_id), Some(foe)) = (exclude, s.theirs.first().cloned()) {
                    if let Some(me) = s.mine.iter().find(|u| u.uid == me_id).cloned() {
                        let foe_after = hit(&foe, me.atk, me.poison);
                        let me_after = hit(&me, foe.atk, foe.poison);
                        s.theirs = swap(&s.theirs, foe.uid, foe_after);
                        s.mine = swap(&s.mine, me.uid, me_after);
                    }
                }
                s
            }
            "ping" => {
                // a minion tells the others that something happened to it (event f.n): 7 = a friendly minion grew
                if let Some(src) = exclude {
                    s = self.fire(s, f.n as u8, Some(src), false);
                }
                s
            }
            "drain_lowest" => {
                // "your hero steals N Health from the lowest Health enemy": the lowest of the enemy minions and
                // their hero takes N, my hero heals N (a Divine Shield eats the hit and the heal)
                let low = s
                    .theirs
                    .iter()
                    .filter(|u| !(u.immune && u.stealth))
                    .min_by_key(|u| u.hp)
                    .cloned();
                match low {
                    Some(u) if u.hp < s.en_hp => {
                        let shielded = u.divine;
                        s = self.dmg_enemy_unit(s, u.uid, f.n, false);
                        if shielded { s } else { self.heal_me(s, f.n) }
                    }
                    _ => {
                        s.en_hp -= f.n;
                        self.heal_me(s, f.n)
                    }
                }
            }
            _ => s,
        }
    }

    /// Healing, armor and health setting.
    pub(crate) fn op_health(&mut self, mut s: S, op: &str, c: &Call) -> S {
        let Call { f, t, sp, .. } = *c;
        match op {
            "heal" => {
                if t == T::Hero {
                    return self.heal_me(s, f.n);
                }
                if let T::Unit(uid) = t {
                    if let Some(u) = s.mine.iter().find(|u| u.uid == uid).cloned() {
                        let mut n = u;
                        n.hp = u.maxhp.min(u.hp + f.n);
                        s.mine = swap(&s.mine, uid, Some(n));
                    }
                }
                s
            }
            "heal_hero" => self.heal_me(s, f.n),
            "heal_all" => {
                for u in s.mine.iter_mut() {
                    u.hp = u.maxhp.min(u.hp + f.n);
                }
                if f.scope == 5 {
                    self.heal_me(s, f.n)
                } else {
                    s
                }
            }
            "armor" => {
                s.my_armor += f.n;
                s
            }
            "heal_damaged" => {
                // "restore n Health to a random damaged friendly character": the most hurt minion, else my hero.
                // Tells the others (event 8) when something was really restored.
                let tgt = s
                    .mine
                    .iter()
                    .filter(|u| u.hp < u.maxhp)
                    .max_by_key(|u| u.maxhp - u.hp)
                    .cloned();
                let mut healed = false;
                if let Some(u) = tgt {
                    let mut n = u;
                    n.hp = u.maxhp.min(u.hp + f.n);
                    s.mine = swap(&s.mine, u.uid, Some(n));
                    healed = true;
                } else if s.my_hp < self.max_hp {
                    s = self.heal_me(s, f.n);
                    healed = true;
                }
                if healed {
                    s = self.fire(s, 8, None, false);
                }
                s
            }
            "lifesteal_heal" => self.heal_me(s, f.n + sp),
            "set_hp" => {
                for u in s.theirs.iter_mut() {
                    if T::Unit(u.uid) == t {
                        u.hp = f.n;
                        u.maxhp = f.n;
                    }
                }
                s
            }
            "all_hp" => {
                for u in s.theirs.iter_mut() {
                    u.hp = f.n;
                    u.maxhp = f.n;
                }
                for u in s.mine.iter_mut() {
                    u.hp = f.n;
                    u.maxhp = f.n;
                }
                s
            }
            _ => s,
        }
    }
}
