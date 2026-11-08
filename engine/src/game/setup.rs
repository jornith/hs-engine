//! The per-turn Planner of a game: the cards of the player to move (hand, hero power, weapon), both boards
//! at the start of the turn (attacks reset, Dormant minions awake, Thoret's shields) and the opponent facts
//! the enemy reply needs (weapon, hero power damage, hand size). `Game::planner` is called once per turn by
//! `Game::turn` and by every rollout. The flags it collects ([`Marks`]) become the Planner's per-card
//! sets.
use super::{Game, Pl};
use crate::*;
use rustc_hash::FxHashMap;

/// Per-card flags of the cards in play, collected for one turn's Planner.
#[derive(Default)]
struct Marks {
    info_of: FxHashMap<i32, usize>,
    hold: FxHashMap<i32, f64>,
    corpse_cost: FxHashSet<i32>,
    death_discount: FxHashSet<i32>,
    dormant: FxHashSet<i32>,
    towers: FxHashSet<i32>,
    enemy_towers: FxHashSet<i32>,
    maiev: FxHashSet<i32>,
    noz: FxHashSet<i32>,
    wtrig: FxHashMap<i32, Vec<Fx>>,
    wlife: FxHashSet<i32>,
}

impl Game<'_> {
    /// The turn search for player `me` with `mana` to spend: a Planner over the cards in play and the start
    /// position. Units on my board wake up (attacks reset, Dormant ends).
    pub fn planner(&self, me: usize, mana: i32) -> (Planner, S) {
        let (a, b) = (&self.p[me], &self.p[1 - me]);
        let mut m = Marks::default();
        let hand = self.mark_hand(me, &mut m);
        let thoret = |pl: &Pl| {
            pl.board
                .iter()
                .any(|(_, c)| c.map(|c| self.t.cards[c].thoret).unwrap_or(false))
        };
        let (thoret_mine, thoret_theirs) = (thoret(a), thoret(b));
        let mine = self.my_board(a, thoret_mine, &mut m);
        let theirs = self.their_board(b, thoret_theirs, &mut m);
        let power_uid = -500 - me as i32;
        let p = self.new_planner(me, m, thoret_theirs, power_uid);
        let s0 = S {
            mana,
            hand,
            mine,
            theirs,
            my_hp: a.hp,
            my_armor: a.armor,
            en_hp: b.hp + b.armor,
            hero_atk: if a.wdur > 0 { a.watk } else { 0 },
            hero_att: 1,
            bonus: 0.0,
            watk: if a.wdur > 0 { a.watk } else { 0 },
            wdur: a.wdur,
            corpses: a.corpses,
            died: 0,
            cannon: false,
            drawn: 0,
            played: 0,
            wid: a.wcard.map(|c| c as i32 + 1).unwrap_or(0),
        };
        (p, s0)
    }

    /// card models and per-card flags of `me`'s hand, hero power and weapon; returns the hand's ids
    fn mark_hand(&self, me: usize, m: &mut Marks) -> Vec<i32> {
        let t = self.t;
        let a = &self.p[me];
        let mut hand = Vec::new();
        for &(uid, c) in &a.hand {
            let d = &t.cards[c];
            m.info_of.insert(uid, d.info_ix);
            m.hold.insert(uid, d.hold);
            if d.corpse_cost {
                m.corpse_cost.insert(uid);
            }
            if d.death_discount {
                m.death_discount.insert(uid);
            }
            if d.dormant {
                m.dormant.insert(uid);
            }
            if d.tower {
                m.towers.insert(uid);
            }
            if let Some(fx) = &d.wtrig {
                m.wtrig.insert(c as i32 + 1, fx.clone());
            }
            if d.wlife {
                m.wlife.insert(c as i32 + 1);
            }
            hand.push(uid);
        }
        let power_uid = -500 - me as i32;
        m.info_of.insert(power_uid, t.cards[a.power].info_ix);
        m.hold.insert(power_uid, 0.0);
        hand.push(power_uid);
        m.hold.insert(-777, t.hold_minion);
        if let Some(wc) = a.wcard {
            if let Some(fx) = &t.cards[wc].wtrig {
                m.wtrig.insert(wc as i32 + 1, fx.clone());
            }
            if t.cards[wc].wlife {
                m.wlife.insert(wc as i32 + 1);
            }
        }
        hand
    }

    /// my board at the start of my turn: attacks reset, Dormant minions awake, Thoret shields
    fn my_board(&self, a: &Pl, thoret: bool, m: &mut Marks) -> Vec<U> {
        let t = self.t;
        let mut mine = Vec::new();
        for (u, c) in &a.board {
            let mut u = *u;
            if let Some(c) = c {
                m.info_of.insert(u.uid, t.cards[*c].info_ix);
                if t.cards[*c].tower {
                    m.towers.insert(u.uid);
                }
                if t.cards[*c].maiev {
                    m.maiev.insert(u.uid);
                }
            }
            if u.immune && u.stealth {
                // Dormant for one turn: awake now
                u.immune = false;
                u.stealth = false;
            }
            u.att = if u.atk > 0 && !u.frozen {
                if u.wf { 2 } else { 1 }
            } else {
                0
            };
            u.fresh = false;
            u.rush = false;
            u.swung = false;
            if thoret && u.divine {
                u.shield = 2;
            }
            mine.push(u);
        }
        mine
    }

    fn their_board(&self, b: &Pl, thoret: bool, m: &mut Marks) -> Vec<U> {
        let t = self.t;
        let mut theirs = Vec::new();
        for (u, c) in &b.board {
            let mut u = *u;
            if let Some(c) = c {
                m.info_of.insert(u.uid, t.cards[*c].info_ix);
                if t.cards[*c].tower {
                    m.enemy_towers.insert(u.uid);
                }
                if t.cards[*c].nozdormu {
                    m.noz.insert(u.uid);
                }
            }
            if thoret && u.divine {
                u.shield = 2;
            }
            theirs.push(u);
        }
        theirs
    }

    fn new_planner(&self, me: usize, m: Marks, thoret_theirs: bool, power_uid: i32) -> Planner {
        let t = self.t;
        let (a, b) = (&self.p[me], &self.p[1 - me]);
        let pw = &t.cards[b.power];
        Planner {
            w: t.cfg[a.cfg].0.clone(),
            infos: t.infos.clone(),
            info_of: m.info_of,
            hold: m.hold,
            corpse_cost: m.corpse_cost,
            death_discount: m.death_discount,
            maiev: m.maiev,
            dormant: m.dormant,
            towers: m.towers,
            enemy_towers: m.enemy_towers,
            enemy_nozdormu: m.noz,
            enemy_thoret: thoret_theirs,
            wtrig: m.wtrig,
            wlife: m.wlife,
            sp_extra: 0,
            enemy_split: 0,
            played_cheap: a.cheap.clone(),
            max_hp: 30,
            max_mana: a.max_mana,
            enemy_hand: b.hand.len() as i32,
            role: 0,
            aoe_threat: None,
            enemy_swing: if b.wdur > 0 { b.watk } else { pw.p_swing },
            enemy_direct: pw.p_direct,
            dk: a.dk || a.corpses > 0,
            power_uid: Some(power_uid),
            draw_uid: -777,
            uid: -1000 - (self.next_id * 64) % 1_000_000,
            eval_cache: FxHashMap::default(),
            ev_cache: FxHashMap::default(),
            lent: FxHashSet::default(),
            silenced: FxHashSet::default(),
            in_cast: false,
            hero_killed: false,
            cast_minion: false,
            cast_sp: 0,
            iwa: FxHashSet::default(),
            dying: 0,
            origin: FxHashMap::default(),
            tok_adj: FxHashMap::default(),
            has_adj: false,
        }
    }
}
