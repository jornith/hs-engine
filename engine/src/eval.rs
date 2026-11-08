//! Position scoring. `uv` values one unit, `evaluate` scores a position from both boards, both heroes and
//! the cards held, `threat` measures how close the enemy board is to lethal. `refined` is the score the
//! search ranks finished lines by: it plays the end of my turn (`end_of_turn`), the simulated enemy reply
//! (`reply` module) and optionally a look-ahead into my next turn (`next_turn_gain`). Scores are cached
//! per position key (`ev`, `eval_cached`).
use crate::planner::*;
use crate::state::*;
use rustc_hash::FxHashSet;

impl Planner {
    /// The value of one unit to its owner: stats by the weights plus keyword bonuses, its deathrattle and
    /// extra value. A frozen enemy is worth less.
    pub fn uv(&self, u: &U, enemy: bool) -> f64 {
        let w = &self.w;
        let a = (u.atk - u.tmp) as f64;
        let hp = u.hp as f64;
        let mut v = w.base + w.atk * a + w.hp * hp + w.body * a * hp;
        if u.divine {
            v += 0.9 + 0.3 * a + (if u.taunt { 0.5 } else { 0.0 });
        }
        if u.taunt {
            v += w.taunt;
        }
        if u.poison {
            v += 1.5;
        }
        if u.life {
            v += 0.3 * a;
        }
        if u.wf {
            v += 0.5 * a;
        }
        if u.stealth {
            v += 0.5;
        }
        if u.sp != 0 {
            v += 0.8 * u.sp as f64;
        }
        if u.reborn {
            v += 0.6 + 0.5 * a;
        }
        if enemy && u.frozen {
            v -= 0.5 * a;
        }
        v + u.dr + u.pv
    }

    /// The transposition key of a position: equal keys mean positions the search treats as the same.
    pub fn key(&self, s: &S) -> Key {
        let mut k: Vec<i64> =
            Vec::with_capacity(16 + s.hand.len() + 21 * (s.mine.len() + s.theirs.len()));
        k.push(s.mana as i64);
        let mut hand = s.hand.clone();
        hand.sort();
        k.push(hand.len() as i64);
        k.extend(hand.iter().map(|&x| x as i64));
        let mut m: Vec<[i64; 24]> = s.mine.iter().map(|u| u.sig()).collect();
        m.sort();
        k.push(m.len() as i64);
        for x in &m {
            k.extend_from_slice(x);
        }
        let mut t: Vec<[i64; 24]> = s.theirs.iter().map(|u| u.sig()).collect();
        t.sort();
        k.push(t.len() as i64);
        for x in &t {
            k.extend_from_slice(x);
        }
        k.extend_from_slice(&[
            s.my_hp as i64,
            s.my_armor as i64,
            s.en_hp as i64,
            s.hero_atk as i64,
            s.hero_att as i64,
            round3(s.bonus),
            s.watk as i64,
            s.wdur as i64,
            s.corpses as i64,
            s.played as i64,
        ]);
        k
    }

    /// The damage the enemy board could deal to my hero next turn through my Taunts.
    pub fn threat(&self, s: &S) -> i32 {
        let mut atks: Vec<i32> = Vec::new();
        for u in &s.theirs {
            if u.frozen || u.atk - u.tmp <= 0 {
                continue;
            }
            atks.push(u.atk - u.tmp);
            if u.wf {
                atks.push(u.atk - u.tmp);
            }
        }
        if self.enemy_swing != 0 {
            atks.push(self.enemy_swing);
        }
        atks.sort();
        let mut taunts: Vec<&U> = s.mine.iter().filter(|u| u.taunt && !u.stealth).collect();
        taunts.sort_by_key(|u| u.hp);
        for t in taunts {
            if atks.is_empty() {
                break;
            }
            if t.divine {
                atks.remove(0);
                if atks.is_empty() {
                    break;
                }
            }
            let mut hp = t.hp;
            if let Some(k) = atks.iter().position(|&a| a >= hp) {
                atks.remove(k);
                continue;
            }
            while hp > 0 && !atks.is_empty() {
                hp -= atks.pop().unwrap();
            }
            if hp > 0 {
                atks.clear();
            }
        }
        atks.iter().sum::<i32>() + self.enemy_direct
    }

    /// The value of the cards still in hand.
    pub fn sum_hold(&self, s: &S) -> f64 {
        let mut t = 0.0;
        for h in &s.hand {
            t += *self.hold.get(h).unwrap_or(&0.0);
        }
        t
    }

    /// The static score of a position for me. With `threat`, the enemy's next-turn damage and the race to
    /// kill each other count too. Lethal is +1000, death -1000.
    pub fn evaluate(&self, s: &S, threat: bool) -> f64 {
        let w = &self.w;
        if s.en_hp <= 0 {
            return 1000.0 + s.my_hp as f64 + s.my_armor as f64;
        }
        if s.my_hp <= 0 {
            return -1000.0;
        }
        let eff = s.my_hp + s.my_armor;
        let mut sm = 0.0;
        for u in &s.mine {
            sm += self.uv(u, false);
        }
        let mut st = 0.0;
        for u in &s.theirs {
            st += self.uv(u, true);
        }
        let mut sc = sm - w.enemy * st;
        let r = self.role as f64 * w.role;
        sc += w.my_hp * (1.0 - 0.4 * r) * eff as f64 + w.low_hp * eff.min(15) as f64
            - w.en_hp * (1.0 + 0.6 * r) * s.en_hp as f64;
        sc += self.sum_hold(s) + s.bonus;
        if s.wdur > 0 {
            sc += w.weapon * s.watk as f64 * s.wdur as f64 + w.weapon_dur * s.wdur as f64;
        }
        if threat {
            let th = self.threat(s);
            if th >= eff {
                sc -= 400.0;
            } else {
                sc -= w.threat * th as f64;
            }
            if w.race != 0.0 {
                let mine: i32 = s
                    .mine
                    .iter()
                    .filter(|u| !u.frozen)
                    .map(|u| u.atk - u.tmp)
                    .sum::<i32>()
                    + s.watk;
                let mine_ttk = if mine > 0 { ceil_div(s.en_hp, mine) } else { 9 };
                let their_ttk = if th > 0 { ceil_div(eff, th) } else { 9 };
                sc += w.race * ((their_ttk - mine_ttk).clamp(-3, 3)) as f64;
            }
        }
        sc
    }

    /// The score of ending the turn in `s0`: end-of-turn effects, the enemy's greedy reply, risk of removal
    /// and area damage, and with `ahead` what my next turn could gain.
    pub fn refined(&mut self, s0: &S, ahead: bool) -> f64 {
        if s0.en_hp <= 0 {
            return 1000.0 + s0.my_hp as f64 + s0.my_armor as f64;
        }
        let s = self.end_of_turn(s0.clone());
        if s.en_hp <= 0 {
            return 1000.0 + s.my_hp as f64 + s.my_armor as f64;
        }
        let after = self.enemy_turn(&s);
        if after.my_hp <= 0 {
            return -1000.0 + self.evaluate(&s, false) * 0.01;
        }
        let base = self.evaluate(&after, false) - self.removal_risk(&after) - self.aoe_risk(&after);
        if !ahead || self.w.lookahead == 0.0 {
            return base;
        }
        base + self.w.lookahead * self.next_turn_gain(&after, 300)
    }

    /// `s` after my end of turn: Doomed minions die, then end-of-turn effects fire left to right.
    pub fn end_of_turn(&mut self, mut s: S) -> S {
        let doomed: Vec<U> = s.mine.iter().filter(|u| u.doomed).cloned().collect();
        for u in doomed {
            s.mine.retain(|x| x.uid != u.uid);
            s.corpses += if self.dk { 1 } else { 0 };
            s = self.spawn(s, &u, true);
        }
        let snapshot = s.mine.clone();
        for u in snapshot {
            let eot = match self.info(u.uid) {
                Some(i) if !i.eot.is_empty() && !self.silenced.contains(&u.uid) => i.eot.clone(),
                _ => continue,
            };
            for f in eot {
                s = self.apply(s, &f, T::None, MINION, Some(u.uid));
            }
        }
        s
    }

    /// My hand at the start of next turn: the hand now, plus the hero power and the expected draw.
    fn next_turn_hand(&self, after: &S) -> Vec<i32> {
        let mut hand = after.hand.clone();
        if let Some(p) = self.power_uid {
            if !hand.contains(&p) {
                hand.push(p);
            }
        }
        if self.w.draw_ahead != 0.0 && hand.len() < 10 {
            hand.push(self.draw_uid);
        }
        hand
    }

    /// The position at the start of my next turn: attacks refreshed, Freeze worn off, one more mana.
    fn next_turn_start(&self, after: &S) -> S {
        let hand = self.next_turn_hand(after);
        let mut s = after.clone();
        s.mine = after
            .mine
            .iter()
            .map(|u| {
                let mut n = *u;
                n.att = if u.frozen || u.atk - u.tmp <= 0 {
                    0
                } else if u.wf {
                    2
                } else {
                    1
                };
                n.frozen = false;
                n.fresh = false;
                n.rush = false;
                n.atk = u.atk - u.tmp;
                n.tmp = 0;
                n.swung = false;
                n
            })
            .collect();
        s.theirs = after
            .theirs
            .iter()
            .map(|u| {
                let mut n = *u;
                n.frozen = false;
                n
            })
            .collect();
        s.mana = 10.min(self.max_mana + 1);
        s.hand = hand;
        s.hero_atk = after.watk;
        s.hero_att = 1;
        s
    }

    /// How much better my best next turn is than doing nothing, searched from `after` with at most
    /// `max_nodes` positions.
    pub fn next_turn_gain(&mut self, after: &S, max_nodes: usize) -> f64 {
        let s = self.next_turn_start(after);
        let k0 = self.key(&s);
        let base = self.ev(&s, &k0);
        let mut best = base;
        let mut seen: FxHashSet<Key> = FxHashSet::default();
        let mut nodes = 0usize;
        let mut stack: Vec<(S, Key)> = vec![(s, k0)];
        while let Some((st, k)) = stack.pop() {
            if nodes >= max_nodes {
                break;
            }
            if seen.contains(&k) {
                continue;
            }
            seen.insert(k.clone());
            nodes += 1;
            if st.en_hp <= 0 {
                return 25.0;
            }
            let e = self.ev(&st, &k);
            if e > best {
                best = e;
            }
            let moves = self.moves(&st);
            let mut kids: Vec<(f64, S, Key)> = Vec::new();
            for (_, ns) in moves {
                let nk = self.key(&ns);
                if seen.contains(&nk) {
                    continue;
                }
                let v = self.ev(&ns, &nk);
                kids.push((v, ns, nk));
            }
            kids.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            let n = kids.len();
            let start = n.saturating_sub(6);
            for (_, ns, nk) in kids.drain(start..) {
                stack.push((ns, nk));
            }
        }
        best - base
    }

    /// `evaluate(s, false)`, cached by key.
    pub fn ev(&mut self, s: &S, k: &Key) -> f64 {
        if let Some(v) = self.ev_cache.get(k) {
            return *v;
        }
        let v = self.evaluate(s, false);
        self.ev_cache.insert(k.clone(), v);
        v
    }
    /// `evaluate(s, true)` after Doomed minions die, cached by key.
    pub fn eval_cached(&mut self, s: &S, k: &Key) -> f64 {
        if let Some(v) = self.eval_cache.get(k) {
            return *v;
        }
        let v = if s.mine.iter().any(|u| u.doomed) {
            let e = self.end_of_turn(s.clone());
            self.evaluate(&e, true)
        } else {
            self.evaluate(s, true)
        };
        self.eval_cache.insert(k.clone(), v);
        v
    }
}
