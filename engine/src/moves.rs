//! Legal moves from a position. `Planner::moves` lists every attack (minions, then my hero) and every
//! card I can afford, each with the position after it; plays go through `effects::Planner::cast`, so one
//! card gives one move per mode and target. Units with equal stats are tried once. A minion played next
//! to an adjacency aura gets extra moves for the other slots, and adjacency bonuses are recomputed after
//! every move. The search in `search` calls `moves` at every node.
use crate::planner::*;
use crate::state::*;
use rustc_hash::FxHashSet;

impl Planner {
    /// The adjacent-minion Attack aura of unit `uid` (0 when silenced or it has none).
    pub fn adj_of(&self, uid: i32) -> i32 {
        if self.silenced.contains(&uid) {
            return 0;
        }
        self.info(uid)
            .map_or(0, |i| i.adj)
            .max(self.tok_adj.get(&uid).cloned().unwrap_or(0))
    }

    /// Recompute the adjacent-aura attack of every minion on a side; `apply` changes attack, otherwise only
    /// the bookkeeping.
    pub fn adj_side(&self, side: &mut [U], apply: bool) {
        let n = side.len();
        let bonus: Vec<i32> = (0..n)
            .map(|i| {
                (if i > 0 {
                    self.adj_of(side[i - 1].uid)
                } else {
                    0
                }) + (if i + 1 < n {
                    self.adj_of(side[i + 1].uid)
                } else {
                    0
                })
            })
            .collect();
        for (u, b) in side.iter_mut().zip(bonus) {
            if apply {
                u.atk = (u.atk + b - u.adjb).max(0);
            }
            u.adjb = b;
        }
    }

    /// Refresh the adjacency bookkeeping of both boards without changing attack (for a start position).
    pub fn norm_adj(&self, s: &mut S) {
        if self.has_adj {
            self.adj_side(&mut s.mine, false);
            self.adj_side(&mut s.theirs, false);
        }
    }

    /// Every legal move from `s` with the position after it. A minion played next to an adjacency aura gets
    /// one extra move per slot to its left.
    pub fn moves(&mut self, s: &S) -> Vec<(Act, S)> {
        let mut out = self.moves_raw(s);
        if self.has_adj {
            let mut extra: Vec<(Act, S)> = Vec::new();
            for (act, ns) in out.iter_mut() {
                // a minion played next to an adjacency aura: it can be put into any slot (the block of it and what it summoned)
                if let Act::Play { uid, .. } = act {
                    if !s.mine.iter().any(|u| u.uid == *uid) {
                        if let Some(ix) = ns.mine.iter().position(|u| u.uid == *uid) {
                            if ix > 0 && ns.mine.iter().any(|u| self.adj_of(u.uid) != 0) {
                                for k in 0..ix {
                                    let mut alt = ns.clone();
                                    let block: Vec<U> = alt.mine.drain(ix..).collect();
                                    for (j, u) in block.into_iter().enumerate() {
                                        alt.mine.insert(k + j, u);
                                    }
                                    extra.push((act.clone(), alt));
                                }
                            }
                        }
                    }
                }
                self.adj_side(&mut ns.mine, true);
                self.adj_side(&mut ns.theirs, true);
            }
            for (_, ns) in extra.iter_mut() {
                self.adj_side(&mut ns.mine, true);
                self.adj_side(&mut ns.theirs, true);
            }
            out.extend(extra);
        }
        out
    }
    /// Every move from `s` before slot choices: minion attacks, hero attacks, then cards in hand. Identical
    /// attackers, targets and cards (same stats) are tried once.
    pub fn moves_raw(&mut self, s: &S) -> Vec<(Act, S)> {
        let mut out: Vec<(Act, S)> = Vec::new();
        let visible: Vec<U> = s.theirs.iter().filter(|u| !u.stealth).cloned().collect();
        let taunts: Vec<U> = visible.iter().filter(|u| u.taunt).cloned().collect();
        let taunted = !taunts.is_empty();
        let pool: &[U] = if taunted { &taunts } else { &visible };
        self.minion_attacks(s, pool, taunted, &mut out);
        if s.hero_att > 0 && s.hero_atk > 0 {
            self.hero_attacks(s, pool, taunted, &mut out);
        }
        self.card_plays(s, &mut out);
        out
    }

    fn minion_attacks(&mut self, s: &S, pool: &[U], taunted: bool, out: &mut Vec<(Act, S)>) {
        let mut seen: FxHashSet<[i64; 24]> = FxHashSet::default();
        for a in s.mine.clone() {
            if a.att <= 0 || a.atk <= 0 || seen.contains(&a.sig()) {
                continue;
            }
            seen.insert(a.sig());
            let mut tseen: FxHashSet<[i64; 24]> = FxHashSet::default();
            for t in pool {
                if tseen.contains(&t.sig()) {
                    continue;
                }
                tseen.insert(t.sig());
                let ns = self.attack_unit(s, a, t);
                let act = Act::Attack {
                    src: T::Unit(a.uid),
                    tgt: T::Unit(t.uid),
                };
                out.push((act, ns));
            }
            if !taunted && !a.rush {
                let ns = self.attack_face(s, a);
                let act = Act::Attack {
                    src: T::Unit(a.uid),
                    tgt: T::Face,
                };
                out.push((act, ns));
            }
        }
    }

    /// my minion `a` attacks enemy minion `t`: damage both ways, deaths, lifesteal, "after attacking" triggers
    fn attack_unit(&mut self, s: &S, a: U, t: &U) -> S {
        let nt = hit(t, a.atk, a.poison);
        let na0 = if self.iwa.contains(&a.uid) {
            Some(a)
        } else {
            hit(&a, t.atk, t.poison)
        };
        let bonus = s.bonus - (if nt.is_none() { t.dr } else { 0.0 })
            + (if na0.is_none() { a.dr } else { 0.0 });
        let na = na0.map(|mut x| {
            x.att -= 1;
            x.stealth = false;
            x.swung = true;
            x
        });
        let mut ns = s.clone();
        ns.theirs = swap(&s.theirs, t.uid, nt);
        ns.mine = swap(&s.mine, a.uid, na);
        ns.bonus = bonus;
        ns = self.their_hurt(ns, t, nt, a.atk);
        if nt.is_none() {
            ns = self.spawn(ns, t, false);
        }
        ns = self.my_hurt(ns, &a, na, t.atk, true);
        if na.is_none() {
            ns = self.spawn(ns, &a, true);
        }
        if a.life && !t.divine {
            ns = self.heal_me(ns, a.atk);
        }
        self.fire_self(ns, a.uid)
    }

    fn attack_face(&mut self, s: &S, a: U) -> S {
        let mut ns = s.clone();
        ns.en_hp = s.en_hp - a.atk;
        let mut x = a;
        x.att -= 1;
        x.stealth = false;
        x.swung = true;
        ns.mine = swap(&s.mine, a.uid, Some(x));
        if a.life {
            ns = self.heal_me(ns, a.atk);
        }
        self.fire_self(ns, a.uid)
    }

    /// my hero attacks with its weapon or attack bonus (Corpse Cannon adds a 1/1 Charge ghoul per swing)
    fn hero_attacks(&mut self, s: &S, pool: &[U], taunted: bool, out: &mut Vec<(Act, S)>) {
        let mut swung = Planner::swing(s);
        if s.cannon && s.wdur > 0 {
            let (ns, _) = self.new_unit(swung, 1, 1, K_CHARGE | K_DOOMED, None, 0.0, 0, 0.0);
            swung = ns;
        }
        let mut tseen: FxHashSet<[i64; 24]> = FxHashSet::default();
        for t in pool {
            if tseen.contains(&t.sig()) {
                continue;
            }
            tseen.insert(t.sig());
            let ns = self.hurt_me(swung.clone(), t.atk);
            let mut ns = self.dmg_enemy_unit(ns, t.uid, s.hero_atk, false);
            if self.wlife.contains(&s.wid) && !t.divine && ns.my_hp > 0 {
                ns = self.heal_me(ns, s.hero_atk);
            }
            if ns.my_hp > 0 {
                self.hero_killed = !ns.theirs.iter().any(|u| u.uid == t.uid);
                let ns = self.fire(ns, 1, None, false);
                self.hero_killed = false;
                let act = Act::Attack {
                    src: T::Hero,
                    tgt: T::Unit(t.uid),
                };
                out.push((act, ns));
            }
        }
        if !taunted {
            let mut ns = swung.clone();
            ns.en_hp = s.en_hp - s.hero_atk;
            if self.wlife.contains(&s.wid) {
                ns = self.heal_me(ns, s.hero_atk);
            }
            let ns = self.fire(ns, 1, None, false);
            out.push((
                Act::Attack {
                    src: T::Hero,
                    tgt: T::Face,
                },
                ns,
            ));
        }
    }

    /// A hash of the effects of all play modes of a card.
    pub(crate) fn effects_hash(i: &Info) -> i64 {
        let mut eh: i64 = 7;
        for (fx, _) in &i.modes {
            for f in fx {
                let vals = [
                    f.op as i64,
                    f.n as i64,
                    f.a as i64,
                    f.h as i64,
                    f.tgt as i64,
                    f.scope as i64,
                    f.cond as i64,
                ];
                for v in vals {
                    eh = eh.wrapping_mul(1_000_003).wrapping_add(v);
                }
            }
            eh = eh.wrapping_mul(31).wrapping_add(1);
        }
        eh
    }

    /// every affordable card in hand, once per (name, cost, attack, health, kind, effects)
    fn card_plays(&mut self, s: &S, out: &mut Vec<(Act, S)>) {
        let mut seen_sig: FxHashSet<(i32, i32, i32, i32, u8, i64)> = FxHashSet::default();
        for &uid in &s.hand {
            let ix = match self.info_of.get(&uid) {
                Some(&ix) => ix,
                None => continue,
            };
            let infos = self.infos.clone(); // Arc: a reference, not a deep copy of the card's effects
            let i = &infos[ix];
            // cards of one name with different effects (the Shatter halves) are different moves
            let sig = (i.name, i.cost, i.atk, i.hp, i.kind, Self::effects_hash(i));
            let by_corpse = self.corpse_cost.contains(&uid);
            let cost = if self.death_discount.contains(&uid) {
                (i.cost - s.died).max(0)
            } else {
                i.cost
            };
            let cant = if by_corpse {
                s.corpses < cost
            } else {
                cost > s.mana
            };
            if seen_sig.contains(&sig) || cant {
                continue;
            }
            seen_sig.insert(sig);
            let n0 = out.len();
            let mut b = s.clone();
            b.mana = s.mana - if by_corpse { 0 } else { cost };
            b.hand = s.hand.iter().cloned().filter(|&h| h != uid).collect();
            b.bonus = s.bonus - self.w.overload * i.overload as f64;
            b.corpses = s.corpses - if by_corpse { i.cost } else { 0 };
            self.play_by_kind(b, ix, uid, out);
            if i.kind != POWER && i.kind != LOCACT {
                for o in out[n0..].iter_mut() {
                    o.1.played += 1;
                }
            }
        }
    }

    /// Play the card `ix` from hand (mana already paid in `b`) by its kind; each result goes to `out`.
    fn play_by_kind(&mut self, b: S, ix: usize, uid: i32, out: &mut Vec<(Act, S)>) {
        let infos = self.infos.clone();
        let i = &infos[ix];
        let none = Act::Play {
            uid,
            t: T::None,
            mi: 0,
        };
        if i.kind == MINION {
            self.play_minion(b, ix, uid, out);
        } else if i.kind == WEAPON {
            let mut ns = b.clone();
            ns.hero_atk = b.hero_atk - b.watk + i.atk;
            ns.watk = i.atk;
            ns.wdur = i.hp;
            ns.cannon = i.cannon;
            ns.wid = i.wid;
            out.push((none, ns));
        } else if i.kind == LOCATION {
            let mut ns = b.clone();
            ns.bonus = b.bonus + self.w.unknown;
            out.push((none, ns));
        } else if matches!(i.kind, SPELL | POWER | HERO_K | LOCACT) {
            let ev = match i.kind {
                SPELL => 2,
                POWER => 4,
                _ => 0,
            };
            for (ns, t, mi) in self.cast(&b, ix, None) {
                let ns = if ev != 0 {
                    self.fire(ns, ev, None, false)
                } else {
                    ns
                };
                out.push((Act::Play { uid, t, mi }, ns));
            }
        }
    }

    /// a minion card from hand (mana already paid in `b`): summon, Maiev and Dormant, its aura, then its
    /// Battlecry against every target, then "whenever you play a minion" triggers
    fn play_minion(&mut self, b: S, ix: usize, uid: i32, out: &mut Vec<(Act, S)>) {
        let infos = self.infos.clone();
        let i = &infos[ix];
        let (mut b, u) = self.new_unit(b, i.atk, i.hp, i.kw, Some(uid), i.dr, i.sp, i.pv);
        let mut u = match u {
            Some(u) => u,
            None => return,
        };
        if i.race != 0 {
            u.race = i.race;
            if let Some(x) = b.mine.iter_mut().find(|x| x.uid == uid) {
                x.race = i.race;
            }
        }
        if !self.maiev.is_empty()
            && b.mine
                .iter()
                .any(|x| x.uid != uid && self.maiev.contains(&x.uid))
        {
            if let Some(u2) = b.mine.iter().find(|x| x.uid == uid).cloned() {
                let mut u2 = u2;
                u2.atk += 3;
                u2.hp += 3;
                u2.maxhp += 3;
                b.mine = swap(&b.mine, uid, Some(u2));
            }
            self.dormant.insert(uid);
        }
        if self.dormant.contains(&uid) {
            let mut asleep = u;
            asleep.att = 0;
            asleep.rush = false;
            asleep.taunt = false;
            asleep.divine = false;
            asleep.stealth = true;
            asleep.immune = true;
            b.mine = swap(&b.mine, uid, Some(asleep));
        }
        if let Some((aa, ah)) = i.aura {
            for x in b.mine.iter_mut() {
                if x.uid != uid {
                    x.atk += aa;
                    x.hp += ah;
                    x.maxhp += ah;
                }
            }
        }
        let bc = !i.modes.is_empty();
        let casts = if bc {
            self.cast(&b, ix, Some(uid))
        } else {
            Vec::new()
        };
        if casts.is_empty() {
            let ns = self.fire(b, 3, Some(uid), bc);
            let act = Act::Play {
                uid,
                t: T::None,
                mi: 0,
            };
            out.push((act, ns));
        } else {
            for (ns, t, mi) in casts {
                let ns = self.fire(ns, 3, Some(uid), bc);
                out.push((Act::Play { uid, t, mi }, ns));
            }
        }
    }
}
