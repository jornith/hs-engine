//! Whole games between two decks, for self-play. A [`Table`] holds the compiled cards and settings shared
//! by every game; a [`Game`] holds both players ([`Pl`]: deck, hand, board, hero) and plays turns until a
//! hero dies. Each turn builds a [`Planner`] for the player to move (`setup`), runs the turn search with the
//! hand-tuned weights and commits the chosen line back into the game. The optional look-ahead
//! (`lookahead`: rollouts or a tree search over whole turns) can replace the search's pick.
#![allow(clippy::should_implement_trait, clippy::too_many_arguments)]
mod lookahead;
mod setup;

use crate::*;
use rustc_hash::FxHashMap;
use std::sync::atomic::AtomicUsize;

/// Rollout decisions taken, and how many of them changed the search's pick.
pub static RO_STATS: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];

fn search(
    p: &mut Planner,
    s0: &S,
    cands: usize,
    max_nodes: usize,
) -> (f64, Vec<Step>, S, bool, Vec<(f64, Vec<Step>, S)>, usize) {
    let r = run_search(p, s0, 30.0, cands, max_nodes);
    (r.score, r.path, r.last, r.lethal, r.ranked, r.nodes)
}

/// A card as a game knows it: its model for the planner plus the flags the Python planner reads from the
/// card text.
#[derive(Clone)]
pub struct CardDef {
    pub info_ix: usize,
    pub kw: u16,
    pub hold: f64,
    pub corpse_cost: bool,
    pub death_discount: bool,
    pub dormant: bool,
    pub tower: bool,
    pub maiev: bool,
    pub nozdormu: bool,
    pub thoret: bool,
    pub wlife: bool,
    pub wtrig: Option<Vec<Fx>>,
    pub p_direct: i32,
    pub p_swing: i32,
    pub cheap: bool, // a 1-Cost minion (for "summon each 1-Cost minion you've played")
}

/// Hand ids from here up are cards the other player knows (they are never re-dealt).
pub const KNOWN: i32 = 900_000_000;

/// What every game of a self-play run shares: the compiled cards, the weights (per side config for duels),
/// the rollout and tree-search settings per config, and the Coin card.
pub struct Table {
    pub cards: Vec<CardDef>,
    pub infos: std::sync::Arc<Vec<Info>>,
    pub cfg: Vec<(Wt,)>, // weights per side config (0: A, 1: B) for duels
    pub ro: Vec<(usize, usize, usize)>, // per config: rollout lines, samples, depth (0 = off)
    pub ro_nodes: usize,
    pub ro_worst: bool,
    pub mcts: Vec<(usize, usize, usize, usize)>, // per config: iterations, depth (turns), my lines, their lines
    pub w: Wt,
    pub coin: usize,
    pub hold_minion: f64,
}

/// One player of a game: deck, hand (id, card), board (unit, card), hero, mana, weapon, hero power,
/// fatigue and Death Knight corpses. `cfg` picks the weight and look-ahead config of this side.
#[derive(Clone)]
pub struct Pl {
    pub deck: Vec<usize>,
    pub hand: Vec<(i32, usize)>,
    pub board: Vec<(U, Option<usize>)>,
    pub hp: i32,
    pub armor: i32,
    pub max_mana: i32,
    pub overload: i32,
    pub fatigue: i32,
    pub corpses: i32,
    pub watk: i32,
    pub wdur: i32,
    pub wcard: Option<usize>,
    pub power: usize,
    pub dk: bool,
    pub cheap: Vec<(i32, i32)>,
    pub turns: i32,
    pub cfg: usize,
}

/// A small xorshift random generator, so a seed gives the same game on every platform.
pub struct Rng(pub u64);
impl Rng {
    /// The next 64 random bits.
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// A random number below `n` (0 when `n` is 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    /// Shuffle `v` in place (Fisher-Yates).
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

/// One game in progress: the shared table, both players, the random stream, card ids and search budget.
pub struct Game<'a> {
    pub t: &'a Table,
    pub p: [Pl; 2],
    pub rng: Rng,
    pub next_id: i32,
    pub cardof: FxHashMap<i32, usize>,
    pub max_nodes: usize,
    pub cands: usize,
    pub tno: i32,          // the game turn being played (both players count)
    pub depth_left: usize, // > 0 inside a rollout: no nested rollouts
    pub last_pick: u64,    // score bits of the line the last rollout pick chose
}

impl<'a> Game<'a> {
    /// A new hand entry for card `c` with a fresh id.
    pub fn card(&mut self, c: usize) -> (i32, usize) {
        self.next_id += 1;
        self.cardof.insert(self.next_id, c);
        (self.next_id, c)
    }

    /// Player `i` draws `n` cards: fatigue on an empty deck, a burned card on a full hand.
    pub fn draw(&mut self, i: usize, n: i32) {
        for _ in 0..n {
            if self.p[i].deck.is_empty() {
                self.p[i].fatigue += 1;
                let f = self.p[i].fatigue;
                let pl = &mut self.p[i];
                let left = pl.armor - f;
                pl.armor = left.max(0);
                pl.hp += left.min(0);
            } else if self.p[i].hand.len() >= 10 {
                self.p[i].deck.pop();
            } else {
                let c = self.p[i].deck.pop().unwrap();
                let h = self.card(c);
                self.p[i].hand.push(h);
            }
        }
    }

    /// Play one turn of player `me`: mana and draw, the turn search, the optional look-ahead pick, then commit
    /// the chosen line.
    pub fn turn(&mut self, me: usize) {
        {
            let pl = &mut self.p[me];
            pl.max_mana = (pl.max_mana + 1).min(10);
            pl.turns += 1;
        }
        self.draw(me, 1);
        if self.p[me].hp <= 0 {
            return;
        }
        let mana = (self.p[me].max_mana - self.p[me].overload).max(0);
        self.p[me].overload = 0;
        let (mut pln, s0) = self.planner(me, mana);
        let (_, mut path, mut fin, lethal, ranked, _) =
            search(&mut pln, &s0, self.cands, self.max_nodes);
        let ro = self.t.ro[self.p[me].cfg.min(self.t.ro.len() - 1)];
        let mc = self.t.mcts[self.p[me].cfg.min(self.t.mcts.len() - 1)];
        if mc.0 > 0 && !lethal && ranked.len() > 1 && self.depth_left == 0 {
            if let Some((p2, f2)) = self.mcts_pick(me, &mut pln, &ranked, mc.0, mc.1, mc.2, mc.3) {
                path = p2;
                fin = f2;
            }
        } else if ro.0 > 1 && !lethal && ranked.len() > 1 && self.depth_left == 0 {
            if let Some((p2, f2)) = self.rollout_pick(me, &mut pln, &ranked, ro) {
                path = p2;
                fin = f2;
            }
        }
        self.commit(me, &mut pln, &path, fin);
    }

    /// Write a played line back into the game: end of turn, both boards and heroes, weapon, overload, mana
    /// gained, cards drawn and the cards that left the hand.
    pub fn commit(&mut self, me: usize, pln: &mut Planner, path: &[Step], fin: S) {
        let end = pln.end_of_turn(fin);
        let t = self.t;
        // played cards: overload, ramp, 1-cost minions remembered
        for st in path {
            if let Act::Play { uid, mi, .. } = st.act {
                if let Some(&c) = self.cardof.get(&uid) {
                    let info = &t.infos[t.cards[c].info_ix];
                    self.p[me].overload += info.overload;
                    if t.cards[c].cheap {
                        self.p[me].cheap.push((info.atk, info.hp));
                    }
                    if let Some((fx, _)) = info.modes.get(mi) {
                        for f in fx {
                            if op_is(f, "ramp") {
                                self.p[me].max_mana = (self.p[me].max_mana + 1).min(10);
                            }
                        }
                    }
                }
            }
        }
        let before_theirs = self.p[1 - me].board.len();
        let keep: FxHashSet<i32> = end.hand.iter().cloned().collect();
        self.p[me].hand.retain(|(uid, _)| keep.contains(uid));
        let mut mine = Vec::new();
        for u in &end.mine {
            let mut u = *u;
            u.frozen = false; // frozen minions thaw at the end of their owner's turn
            u.tmp = 0;
            u.att = 0;
            mine.push((u, self.cardof.get(&u.uid).cloned()));
        }
        self.p[me].board = mine;
        let theirs: Vec<(U, Option<usize>)> = end
            .theirs
            .iter()
            .map(|u| (*u, self.cardof.get(&u.uid).cloned()))
            .collect();
        let opp_total = self.p[1 - me].hp + self.p[1 - me].armor;
        let dmg = opp_total - end.en_hp;
        {
            let o = &mut self.p[1 - me];
            o.board = theirs;
            if dmg > 0 {
                let left = o.armor - dmg;
                o.armor = left.max(0);
                o.hp += left.min(0);
            } else if dmg < 0 {
                o.hp -= dmg; // healed (their lifesteal in my turn is rare): keep it simple
            }
            if o.dk {
                o.corpses += (before_theirs as i32 - o.board.len() as i32).max(0);
            }
        }
        {
            let pl = &mut self.p[me];
            pl.hp = end.my_hp;
            pl.armor = end.my_armor;
            pl.watk = end.watk;
            pl.wdur = end.wdur;
            pl.wcard = if end.wdur > 0 && end.wid > 0 {
                Some(end.wid as usize - 1)
            } else {
                None
            };
            if pl.dk || end.corpses > 0 {
                pl.corpses = end.corpses.max(0);
            }
        }
        self.draw(me, end.drawn);
    }

    /// Play turns until a hero dies or `max_turns`: the winner (0 or 1), or -1 for a draw or no winner.
    pub fn play(&mut self, max_turns: i32) -> i32 {
        for turn in self.tno..max_turns {
            self.tno = turn;
            let me = (turn % 2) as usize;
            self.turn(me);
            let (a, b) = (self.p[0].hp, self.p[1].hp);
            if a <= 0 && b <= 0 {
                return -1;
            }
            if a <= 0 {
                return 1;
            }
            if b <= 0 {
                return 0;
            }
        }
        -1
    }
}

/// A new game between two decks (cards, hero power, Death Knight flag): shuffled decks, opening hands of 3
/// and 4 cards after a simple mulligan (cards costing 4 or more go back), and the Coin for the second
/// player.
pub fn new_game<'a>(
    t: &'a Table,
    decks: [&(Vec<usize>, usize, bool); 2],
    seed: u64,
    max_nodes: usize,
    cands: usize,
) -> Game<'a> {
    let mk = |d: &(Vec<usize>, usize, bool)| Pl {
        deck: d.0.clone(),
        hand: vec![],
        board: vec![],
        hp: 30,
        armor: 0,
        max_mana: 0,
        overload: 0,
        fatigue: 0,
        corpses: 0,
        watk: 0,
        wdur: 0,
        wcard: None,
        power: d.1,
        dk: d.2,
        cheap: vec![],
        turns: 0,
        cfg: 0,
    };
    let mut g = Game {
        t,
        p: [mk(decks[0]), mk(decks[1])],
        rng: Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1),
        next_id: 1000,
        cardof: FxHashMap::default(),
        max_nodes,
        cands,
        depth_left: 0,
        last_pick: 0,
        tno: 0,
    };
    for i in 0..2 {
        let mut deck = std::mem::take(&mut g.p[i].deck);
        g.rng.shuffle(&mut deck);
        let n = if i == 0 { 3 } else { 4 };
        let mut hand: Vec<usize> = (0..n).map(|_| deck.pop().unwrap()).collect();
        // simple mulligan: 4+ cost cards go back
        let back: Vec<usize> = hand
            .iter()
            .cloned()
            .filter(|&c| t.infos[t.cards[c].info_ix].cost >= 4)
            .collect();
        hand.retain(|&c| t.infos[t.cards[c].info_ix].cost < 4);
        deck.extend(back);
        g.rng.shuffle(&mut deck);
        while hand.len() < n {
            hand.push(deck.pop().unwrap());
        }
        g.p[i].deck = deck;
        for c in hand {
            let h = g.card(c);
            g.p[i].hand.push(h);
        }
    }
    let coin = g.card(t.coin);
    g.p[1].hand.push(coin);
    g
}
