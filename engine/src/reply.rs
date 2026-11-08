//! The simulated enemy reply. After my turn ends, `enemy_turn` plays a greedy opponent: random pings it
//! announced (`enemy_split`), then every enemy minion that can attack, strongest first, each taking the trade
//! that is worst for me or going face; then its weapon swing and direct hero-power damage. The result is a
//! position, not a search: one plausible reply, cheap enough to run for every candidate line.
//! `removal_risk` and `aoe_risk` are score penalties for what the enemy hand might still do.
use crate::planner::*;
use crate::state::*;

/// The parts of a position the enemy reply changes.
struct Reply {
    mine: Vec<U>,
    theirs: Vec<U>,
    my_hp: i32,
    my_armor: i32,
    bonus: f64,
    en_hp: i32,
}

impl Reply {
    fn hurt(&mut self, d: i32) {
        let left = self.my_armor - d;
        self.my_armor = left.max(0);
        self.my_hp += left.min(0);
    }

    fn lifesteal(&mut self, atk: i32) {
        self.en_hp = self.en_hp.max(30).min(self.en_hp + atk);
    }

    /// my minions the enemy can attack: the Taunts if there are any, else all that are not Stealthed
    fn guards(&self) -> (Vec<U>, bool) {
        let taunts: Vec<U> = self
            .mine
            .iter()
            .filter(|m| m.taunt && !m.stealth)
            .cloned()
            .collect();
        if !taunts.is_empty() {
            return (taunts, true);
        }
        (
            self.mine.iter().filter(|m| !m.stealth).cloned().collect(),
            false,
        )
    }
}

impl Planner {
    /// The position after one greedy enemy turn (see the module doc).
    pub fn enemy_turn(&mut self, s0: &S) -> S {
        let s = s0.clone();
        let mut r = Reply {
            mine: s.mine.clone(),
            theirs: s.theirs.clone(),
            my_hp: s.my_hp,
            my_armor: s.my_armor,
            bonus: s.bonus,
            en_hp: s.en_hp,
        };
        self.reply_split(&mut r);
        for uid in Self::reply_swings(&r.theirs) {
            self.reply_attack(&mut r, &s, uid);
        }
        self.reply_weapon(&mut r);
        r.hurt(self.enemy_direct);
        self.reply_end(&mut r);
        let mut out = s;
        out.mine = r.mine;
        out.theirs = r.theirs;
        out.my_hp = r.my_hp;
        out.my_armor = r.my_armor;
        out.bonus = r.bonus;
        out.en_hp = r.en_hp;
        out
    }

    /// `enemy_split` one-damage hits spread over my minions and my hero in turn
    fn reply_split(&self, r: &mut Reply) {
        for k in 0..self.enemy_split.max(0) as usize {
            let alive: Vec<U> = r.mine.iter().filter(|m| !m.immune).cloned().collect();
            let j = k % (alive.len() + 1);
            if j == alive.len() {
                r.hurt(1);
                continue;
            }
            let m = alive[j];
            let nm = hit(&m, 1, false);
            r.mine.retain(|x| x.uid != m.uid);
            if let Some(x) = nm {
                r.mine.push(x);
            } else {
                r.bonus += m.dr;
            }
        }
    }

    /// enemy attackers, strongest first, Windfury ones twice
    fn reply_swings(theirs: &[U]) -> Vec<i32> {
        let mut order: Vec<U> = theirs
            .iter()
            .filter(|u| !u.frozen && u.atk - u.tmp > 0)
            .cloned()
            .collect();
        order.sort_by_key(|u| -u.atk);
        let mut swings = Vec::new();
        for u in &order {
            for _ in 0..(if u.wf { 2 } else { 1 }) {
                swings.push(u.uid);
            }
        }
        swings
    }

    /// the minion `a` attacks: the trade that costs me most minus what it costs them, unless going face
    /// (0.6 per damage, 50 if lethal) is worth more. None = face (or no legal target behind Taunts).
    fn reply_target(&self, r: &Reply, a: &U, atk: i32) -> (Option<U>, bool) {
        let (pool, taunted) = r.guards();
        let mut best: Option<U> = None;
        let mut best_sc = if taunted {
            0.0
        } else {
            0.6 * atk as f64
                + (if r.my_hp + r.my_armor <= atk {
                    50.0
                } else {
                    0.0
                })
        };
        for m in &pool {
            let gain = self.uv(m, false)
                - match hit(m, atk, a.poison) {
                    Some(x) => self.uv(&x, false),
                    None => self.afterlife(m, false),
                };
            let loss = self.uv(a, true)
                - match hit(a, m.atk, m.poison) {
                    Some(x) => self.uv(&x, true),
                    None => self.afterlife(a, true),
                };
            if gain - loss > best_sc {
                best = Some(*m);
                best_sc = gain - loss;
            }
        }
        (best, taunted)
    }

    /// one enemy attack by unit `uid`
    fn reply_attack(&mut self, r: &mut Reply, s: &S, uid: i32) {
        let a = match r.theirs.iter().find(|x| x.uid == uid) {
            Some(a) => *a,
            None => return,
        };
        if r.my_hp <= 0 {
            return;
        }
        let atk = a.atk - a.tmp;
        match self.reply_target(r, &a, atk) {
            (Some(best), _) => self.reply_trade(r, s, &a, &best, atk),
            (None, false) => {
                r.hurt(atk);
                if a.life {
                    r.lifesteal(atk);
                }
            }
            (None, true) => {}
        }
    }

    /// enemy `a` attacks my `best`: damage both ways, lifesteal, Ghoul Tower, deaths and what they leave
    fn reply_trade(&mut self, r: &mut Reply, s: &S, a: &U, best: &U, atk: i32) {
        let nm = hit(best, atk, a.poison);
        let na = hit(a, best.atk, best.poison);
        if a.life && !best.divine {
            r.lifesteal(atk);
        }
        if best.life && !a.divine && best.atk > 0 {
            r.my_hp = self.max_hp.min(r.my_hp + best.atk);
        }
        r.mine.retain(|x| x.uid != best.uid);
        if let Some(x) = nm {
            r.mine.push(x);
            if self.towers.contains(&best.uid) && !best.divine && r.mine.len() < 7 {
                let mut tmp = s.clone();
                tmp.mine = r.mine.clone();
                r.mine = self.my_hurt(tmp, best, Some(x), atk, false).mine;
            }
        }
        r.theirs.retain(|x| x.uid != a.uid);
        if let Some(x) = na {
            r.theirs.push(x);
        }
        r.bonus +=
            (if nm.is_none() { best.dr } else { 0.0 }) - (if na.is_none() { a.dr } else { 0.0 });
        if nm.is_none() || na.is_none() {
            let mut tmp = s.clone();
            tmp.mine = std::mem::take(&mut r.mine);
            tmp.theirs = std::mem::take(&mut r.theirs);
            if nm.is_none() {
                tmp = self.spawn(tmp, best, true);
            }
            if na.is_none() {
                tmp = self.spawn(tmp, a, false);
            }
            r.mine = tmp.mine;
            r.theirs = tmp.theirs;
        }
    }

    /// the enemy weapon (`enemy_swing` attack): into my lowest-health Taunt, else my hero
    fn reply_weapon(&self, r: &mut Reply) {
        if self.enemy_swing == 0 {
            return;
        }
        let (pool, taunted) = r.guards();
        if !taunted {
            r.hurt(self.enemy_swing);
            return;
        }
        let mut t = pool[0];
        for x in &pool {
            if x.hp < t.hp {
                t = *x;
            }
        }
        let nt = hit(&t, self.enemy_swing, false);
        r.mine.retain(|x| x.uid != t.uid);
        match nt {
            Some(x) => r.mine.push(x),
            None => r.bonus += t.dr,
        }
    }

    /// end of the enemy turn: minions I took for a turn go back; their Nozdormu shields or grows their board
    fn reply_end(&self, r: &mut Reply) {
        if !self.lent.is_empty() && r.mine.iter().any(|u| self.lent.contains(&u.uid)) {
            let back: Vec<U> = r
                .mine
                .iter()
                .filter(|u| self.lent.contains(&u.uid))
                .cloned()
                .collect();
            r.mine.retain(|u| !self.lent.contains(&u.uid));
            r.theirs.extend(back);
        }
        if !self.enemy_nozdormu.is_empty()
            && r.theirs
                .iter()
                .any(|u| self.enemy_nozdormu.contains(&u.uid))
        {
            for u in r.theirs.iter_mut() {
                if u.divine {
                    u.atk += 3;
                    u.hp += 3;
                    u.maxhp += 3;
                } else {
                    u.divine = true;
                    u.shield = if self.enemy_thoret { 2 } else { 0 };
                }
            }
        }
    }

    /// Penalty for my best minions being exposed to removal still in the enemy hand.
    pub fn removal_risk(&self, after: &S) -> f64 {
        if after.mine.is_empty() || self.enemy_hand == 0 || self.w.enemy_removal == 0.0 {
            return 0.0;
        }
        let mut best = f64::NEG_INFINITY;
        for u in &after.mine {
            let v = self.uv(u, false);
            if v > best {
                best = v;
            }
        }
        let x = self.enemy_hand as f64 / 3.0;
        self.w.enemy_removal * (if x < 1.0 { x } else { 1.0 }) * best
    }

    /// Penalty for my board being exposed to a board clear the enemy may hold.
    pub fn aoe_risk(&self, after: &S) -> f64 {
        if self.w.aoe_model != 0.0 {
            if let Some((dmg, p)) = self.aoe_threat {
                let mut t = 0.0;
                for u in &after.mine {
                    if u.hp <= dmg {
                        t += self.uv(u, false);
                    }
                }
                return self.w.aoe_model * p * t;
            }
        }
        if self.enemy_hand < 2 || self.w.aoe_risk == 0.0 {
            return 0.0;
        }
        let mut small: Vec<f64> = after
            .mine
            .iter()
            .filter(|u| u.hp <= 3)
            .map(|u| self.uv(u, false))
            .collect();
        small.sort_by(|a, b| b.partial_cmp(a).unwrap());
        let mut t = 0.0;
        for v in small.iter().skip(2) {
            t += v;
        }
        self.w.aoe_risk * t
    }
}
