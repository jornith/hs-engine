//! Card effects. `Planner::apply` takes one effect op ([`Fx`]) and a target and returns the new position;
//! it dispatches by op name to the op groups in the submodules (damage, summon, buffs, cards, removal).
//! `cast` plays a card's effects against every legal target ([`targets`]). Triggers ([`triggers`]) and
//! deaths call back into `apply`, so an effect resolves fully (deaths, Deathrattles, "whenever" effects)
//! before the next one starts.
mod buffs;
mod cards;
mod damage;
mod removal;
mod summon;
/// Legal targets of an effect.
pub mod targets;
/// Event triggers of weapons and minions.
pub mod triggers;

use crate::planner::*;
use crate::state::*;

/// The arguments of one `apply` call, passed on to an op group.
#[derive(Clone, Copy)]
pub(crate) struct Call<'a> {
    pub f: &'a Fx,
    pub t: T,
    pub exclude: Option<i32>,
    /// spell damage added to this effect (spells only)
    pub sp: i32,
}

impl Planner {
    /// `d` damage to my hero, armor first.
    pub fn hurt_me(&self, mut s: S, d: i32) -> S {
        if d <= 0 {
            return s;
        }
        let armor = s.my_armor - d;
        s.my_armor = armor.max(0);
        s.my_hp += armor.min(0);
        s
    }

    /// Restore `n` health to my hero, up to its maximum.
    pub fn heal_me(&self, mut s: S, n: i32) -> S {
        s.my_hp = self.max_hp.min(s.my_hp + n);
        s
    }

    /// A fresh unit id for a unit created during the search (counting down from below the real ids).
    pub fn new_id(&mut self) -> i32 {
        let u = self.uid;
        self.uid -= 1;
        u
    }

    /// `s` after my hero attacked: no attack left, and the weapon loses one durability (and breaks at 0).
    pub fn swing(s: &S) -> S {
        let mut n = s.clone();
        if s.wdur <= 0 {
            n.hero_att = 0;
            return n;
        }
        if s.wdur == 1 {
            n.hero_att = 0;
            n.wdur = 0;
            n.watk = 0;
            n.hero_atk = s.hero_atk - s.watk;
            return n;
        }
        n.hero_att = 0;
        n.wdur = s.wdur - 1;
        n
    }

    /// The number of cards in hand, not counting the kinds in `skip`.
    pub fn hand_count(&self, s: &S, skip: &[u8]) -> i32 {
        s.hand
            .iter()
            .filter(|h| {
                self.info(**h)
                    .map(|i| !skip.contains(&i.kind))
                    .unwrap_or(true)
            })
            .count() as i32
    }

    /// Apply one effect `f` to target `t`. `kind` is the kind of card that causes it (spell damage counts for
    /// spells only); `exclude` is the acting minion, which board-wide effects skip.
    pub fn apply(&mut self, s: S, f: &Fx, t: T, kind: u8, exclude: Option<i32>) -> S {
        let sp: i32 = if kind == SPELL {
            s.mine.iter().map(|u| u.sp).sum::<i32>() + self.sp_extra + self.cast_sp
        } else {
            0
        };
        if f.op == 255 {
            return s;
        }
        let op = OPS[f.op as usize];
        let c = Call { f, t, exclude, sp };
        match op {
            "dmg" | "face" | "self_dmg" | "aoe" | "aoe_ls" | "halve_enemy" | "dmg_same_race"
            | "dmg_chain" | "dmg_split" | "dmg_excess" => self.op_damage(s, op, &c),
            "dmg_random" | "dmg_random_minions" | "ping" | "attack_random" | "drain_lowest" => {
                self.op_damage_random(s, op, &c)
            }
            "heal" | "heal_hero" | "heal_all" | "heal_damaged" | "lifesteal_heal" | "set_hp"
            | "all_hp" | "armor" => self.op_health(s, op, &c),
            "buff" | "buff_all" | "self_buff_allies" | "enemy_atk_temp" | "set_atk"
            | "buff_all_corpse" => self.op_buff(s, op, &c),
            "self_atk" | "self_stats" | "grow_if_full" | "self_hp_hand" | "self_buff_hand"
            | "self_hp_enemy_hand" => self.op_self_buff(s, op, &c),
            "hand_buff" | "buff_name" | "hero_atk" | "weapon" | "weapon_buff"
            | "destroy_weapon" => self.op_hand_weapon(s, op, &c),
            "summon" | "summon_corpse" | "summon_played" | "summon_copies" | "summon_adj"
            | "enemy_summon" | "dr_all" | "steal_temp" | "mask_fill" => self.op_summon(s, op, &c),
            "draw" | "draw_to" | "discover" | "gen" | "value" | "discard" | "opp_cards"
            | "corpse" | "mana" | "ramp" | "bounce" | "bounce_all" | "draw_brk" | "draw_kill"
            | "dmg_surv" | "aoe_draw" | "shuffle_self" => self.op_cards(s, op, &c),
            "destroy" | "destroy_highest" | "destroy_random" | "eat_right" | "doom_all" => {
                self.op_destroy(s, op, &c)
            }
            "destroy_all" | "transform" | "transform_all" => self.op_mass_removal(s, op, &c),
            "silence" | "freeze" | "freeze_all" | "bitter" => self.op_silence_freeze(s, op, &c),
            _ => s,
        }
    }

    /// Play the card model `ix` (acting minion `exclude`): one result per play mode and legal target, as
    /// (position after it, target, mode). Deaths and triggers resolve inside.
    pub fn cast(&mut self, s: &S, ix: usize, exclude: Option<i32>) -> Vec<(S, T, usize)> {
        let infos = self.infos.clone();
        let modes = &infos[ix].modes;
        let kind = infos[ix].kind;
        self.cast_minion = kind == MINION;
        self.cast_sp = if kind == SPELL { infos[ix].sp } else { 0 };
        let mut out = Vec::new();
        for (mi, (fx, unknown)) in modes.iter().enumerate() {
            if fx.is_empty() {
                if kind != MINION {
                    let mut ns = s.clone();
                    ns.bonus = s.bonus + if *unknown { self.w.unknown } else { 0.0 };
                    out.push((ns, T::None, mi));
                }
                continue;
            }
            // Combo effects work only when a card was played before this one this turn
            let live = |f: &Fx| f.cond != 9 || s.played > 0;
            let primary = fx.iter().position(|f| f.tgt != 0 && f.tgt != 5 && live(f));
            let targets = match primary {
                Some(p) => self.targets(s, &fx[p], exclude),
                None => vec![T::None],
            };
            for t in targets {
                let mut ns = s.clone();
                self.in_cast = true;
                self.dying = 0;
                for (j, f) in fx.iter().enumerate() {
                    if !live(f) {
                        continue;
                    }
                    let tt = if Some(j) == primary || f.tgt == 5 {
                        t
                    } else {
                        T::None
                    };
                    ns = self.apply(ns, f, tt, kind, exclude);
                }
                self.in_cast = false;
                self.dying = 0;
                out.push((ns, t, mi));
            }
        }
        out
    }
}
