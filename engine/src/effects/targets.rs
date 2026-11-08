//! Legal targets of an effect: which units or heroes an op may hit, given its scope and condition
//! (`Fx::tgt`, `Fx::cond`), with the acting minion excluded. `effects::Planner::cast` uses it to expand a
//! targeted card into one result per target. Units with equal stats are one target, which keeps the
//! search from trying the same play twice.
use crate::planner::*;
use crate::state::*;
use rustc_hash::FxHashSet;

/// One target per distinct enemy unit (equal units are one target) that the op may hit. `cast_minion` is
/// set while a minion's Battlecry is cast: only spells and powers are barred from Elusive minions.
fn enemy_units(s: &S, f: &Fx, cast_minion: bool) -> Vec<T> {
    let mut out = Vec::new();
    let mut seen: FxHashSet<[i64; 24]> = FxHashSet::default();
    for u in &s.theirs {
        if u.stealth || (u.elusive && !cast_minion) || seen.contains(&u.sig()) {
            continue;
        }
        let c = f.cond;
        if (c == 1 && u.atk < f.cond_n) || (c == 2 && u.atk > f.cond_n) {
            continue;
        }
        if (c == 3 && u.hp >= u.maxhp) || (c == 4 && u.hp < u.maxhp) {
            continue;
        }
        if (op_is(f, "set_hp") && u.hp <= f.n) || (op_is(f, "set_atk") && u.atk <= f.n) {
            continue;
        }
        seen.insert(u.sig());
        out.push(T::Unit(u.uid));
    }
    out
}

/// The first enemy unit that can be targeted: the forced target when a friendly-only effect has no friend.
fn first_enemy(s: &S) -> Vec<T> {
    s.theirs
        .iter()
        .filter(|u| !u.stealth && !u.elusive && !u.immune)
        .take(1)
        .map(|u| T::Unit(u.uid))
        .collect()
}

impl Planner {
    /// The targets `f` may take in `s`, by its scope `f.tgt` ([`tgt_code`]): 1 an enemy character, 2 an
    /// enemy minion, 3 a friendly character, 4 a friendly minion, 7 any minion, 8 any character (my own
    /// hero excluded). Other scopes have one target, `T::None`. Units with equal stats count once;
    /// `exclude` (the acting minion) never counts.
    pub fn targets(&self, s: &S, f: &Fx, exclude: Option<i32>) -> Vec<T> {
        let tgt = f.tgt;
        if tgt == 7 || tgt == 8 {
            let mut f2 = *f;
            f2.tgt = if tgt == 7 { 2 } else { 1 };
            let mut out = self.targets(s, &f2, exclude);
            let mut seen: FxHashSet<[i64; 24]> = FxHashSet::default();
            for u in &s.mine {
                if Some(u.uid) == exclude || (u.immune && u.stealth) || seen.contains(&u.sig()) {
                    continue;
                }
                seen.insert(u.sig());
                out.push(T::Unit(u.uid));
            }
            return out;
        }
        if tgt == 1 || tgt == 2 {
            let mut out = enemy_units(s, f, self.cast_minion);
            if tgt == 1 && (op_is(f, "dmg") || op_is(f, "freeze")) {
                out.push(T::Face);
            }
            return out;
        }
        if tgt == 3 || tgt == 4 {
            return self.friendly_targets(s, f, exclude);
        }
        vec![T::None]
    }

    /// targets of a friendly-scope effect (`f.tgt` 3 or 4)
    fn friendly_targets(&self, s: &S, f: &Fx, exclude: Option<i32>) -> Vec<T> {
        let tgt = f.tgt;
        let mine: Vec<&U> = s
            .mine
            .iter()
            .filter(|u| Some(u.uid) != exclude && !(u.immune && u.stealth))
            .collect();
        if op_is(f, "heal") {
            let mut out: Vec<T> = mine
                .iter()
                .filter(|u| u.hp < u.maxhp)
                .map(|u| T::Unit(u.uid))
                .collect();
            if tgt == 3 && s.my_hp < self.max_hp {
                out.push(T::Hero);
            }
            if out.is_empty() {
                out = if tgt == 3 {
                    vec![T::Hero]
                } else {
                    mine.iter().take(1).map(|u| T::Unit(u.uid)).collect()
                };
            }
            if out.is_empty() {
                // "Restore a minion" with no friendly minions: the game forces an enemy one
                out = first_enemy(s);
            }
            return out;
        }
        let mut out = Vec::new();
        let mut seen: FxHashSet<[i64; 24]> = FxHashSet::default();
        for u in mine {
            if seen.contains(&u.sig()) {
                continue;
            }
            seen.insert(u.sig());
            if self.buff_unit(u, f) != *u {
                out.push(T::Unit(u.uid));
            }
        }
        if tgt == 3 && f.cond == 5 && s.hero_att > 0 {
            out.push(T::Hero);
        }
        if tgt == 4 && out.is_empty() && s.mine.is_empty() && f.cond != 7 {
            // "Give a minion ...": with no friendly minion the game still lets you target an enemy one
            out = first_enemy(s);
        }
        out
    }
}
