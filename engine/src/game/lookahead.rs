//! Look-ahead over whole turns, used by `Game::turn` when a side's config enables it. `rollout_pick` plays
//! each of the best few lines of a turn on into the future several times, with the hidden cards re-dealt
//! (`hide`), and keeps the line with the best average `value`. `mcts_pick` does the same as a tree search
//! over both players' turns (UCB per [`Node`]). Both run on copies of the game (`fork`) and never change it.
use super::{Game, KNOWN, Pl, RO_STATS, Rng, search};
use crate::*;
use std::sync::atomic::Ordering;

/// A tree node of `mcts_pick`: visits, value sum for the root player, and per branch the child node,
/// visits and value sum.
struct Node {
    n: f64,
    w: f64,
    kids: Vec<usize>,
    kn: Vec<f64>,
    kw: Vec<f64>,
}

impl Node {
    fn new(k: usize) -> Node {
        Node {
            n: 0.0,
            w: 0.0,
            kids: vec![usize::MAX; k],
            kn: vec![0.0; k],
            kw: vec![0.0; k],
        }
    }

    /// the branch to try: every branch once first, in search order, then the best UCB score (the opponent
    /// maximizes 1 - value)
    fn ucb(&self, maximize: bool) -> usize {
        let mut best = (f64::NEG_INFINITY, 0usize);
        for a in 0..self.kn.len() {
            if self.kn[a] == 0.0 {
                return a;
            }
            let q = self.kw[a] / self.kn[a];
            let q = if maximize { q } else { 1.0 - q };
            let u = q + 0.6 * (self.n.ln() / self.kn[a]).sqrt();
            if u > best.0 {
                best = (u, a);
            }
        }
        best.1
    }

    /// `ucb` over the first `na` branches only, with the visit count taken from them
    fn ucb_first(&self, na: usize, maximize: bool) -> usize {
        let mut tmp = Node {
            n: 0.0,
            w: 0.0,
            kids: vec![],
            kn: self.kn[..na].to_vec(),
            kw: self.kw[..na].to_vec(),
        };
        tmp.n = tmp.kn.iter().sum::<f64>().max(1.0);
        tmp.ucb(maximize)
    }

    /// the child of (node, branch), created on the first visit with `k` branches
    fn child(nodes: &mut Vec<Node>, node: usize, act: usize, k: usize) -> usize {
        let c = nodes[node].kids[act];
        if c != usize::MAX {
            return c;
        }
        nodes.push(Node::new(k));
        let id = nodes.len() - 1;
        nodes[node].kids[act] = id;
        id
    }
}

/// up to `k` of the ranked lines that end in different positions
fn distinct_lines<'r>(
    pln: &mut Planner,
    ranked: &'r [(f64, Vec<Step>, S)],
    k: usize,
) -> Vec<&'r (f64, Vec<Step>, S)> {
    let mut lines = Vec::new();
    let mut seen: FxHashSet<Key> = FxHashSet::default();
    for r in ranked {
        let e = pln.end_of_turn(r.2.clone());
        if seen.insert(pln.key(&e)) {
            lines.push(r);
        }
        if lines.len() >= k {
            break;
        }
    }
    lines
}

impl<'a> Game<'a> {
    /// the line of this turn with the best simulated futures: for the k best (different) lines, play the
    /// next `depth` turns n times from re-dealt hidden cards (the opponent's hand from what I cannot see, my
    /// deck reshuffled), same futures for every line, and score the end by `value`
    pub fn rollout_pick(
        &mut self,
        me: usize,
        pln: &mut Planner,
        ranked: &[(f64, Vec<Step>, S)],
        ro: (usize, usize, usize),
    ) -> Option<(Vec<Step>, S)> {
        let (k, n, depth) = ro;
        let mut lines: Vec<&(f64, Vec<Step>, S)> = Vec::new();
        let mut seen: FxHashSet<Key> = FxHashSet::default();
        for r in ranked {
            let e = pln.end_of_turn(r.2.clone());
            let key = pln.key(&e);
            if seen.insert(key) {
                lines.push(r);
            }
            if lines.len() >= k {
                break;
            }
        }
        if lines.len() < 2 {
            return None;
        }
        let seeds: Vec<u64> = (0..n).map(|_| self.rng.next()).collect();
        let mut best: Option<(f64, usize)> = None;
        for (li, line) in lines.iter().enumerate() {
            let mut v = 0.0;
            for &sd in &seeds {
                let mut g = self.fork(sd);
                g.commit(me, pln, &line.1, line.2.clone()); // the planner that found the line knows its cards
                g.hide(me);
                for d in 0..depth {
                    let who = if d % 2 == 0 { 1 - me } else { me };
                    if g.p[0].hp <= 0 || g.p[1].hp <= 0 {
                        break;
                    }
                    g.turn(who);
                }
                v += g.value(me);
            }
            let v = if self.t.ro_worst { -v } else { v }; // control experiment: pick the worst future
            if best.map(|b| v > b.0).unwrap_or(true) {
                best = Some((v, li));
            }
        }
        let li = best?.1;
        self.last_pick = lines[li].0.to_bits();
        RO_STATS[0].fetch_add(1, Ordering::Relaxed);
        if li != 0 {
            RO_STATS[1].fetch_add(1, Ordering::Relaxed);
        }
        Some((lines[li].1.clone(), lines[li].2.clone()))
    }

    /// start of `who`'s turn: mana and the draw; None if the draw killed it
    pub fn begin_turn(&mut self, who: usize) -> Option<i32> {
        {
            let pl = &mut self.p[who];
            pl.max_mana = (pl.max_mana + 1).min(10);
            pl.turns += 1;
        }
        self.draw(who, 1);
        if self.p[who].hp <= 0 {
            return None;
        }
        let mana = (self.p[who].max_mana - self.p[who].overload).max(0);
        self.p[who].overload = 0;
        Some(mana)
    }

    /// the k best different lines of `who`'s turn from the current position (planner kept for commit)
    pub fn lines_for(
        &mut self,
        who: usize,
        mana: i32,
        k: usize,
    ) -> (Planner, Vec<(f64, Vec<Step>, S)>) {
        let (mut pl, s0) = self.planner(who, mana);
        let (_, path, fin, lethal, ranked, _) = search(&mut pl, &s0, self.cands, self.max_nodes);
        if lethal || ranked.is_empty() {
            return (pl, vec![(0.0, path, fin)]);
        }
        let mut out: Vec<(f64, Vec<Step>, S)> = Vec::new();
        let mut seen: FxHashSet<Key> = FxHashSet::default();
        for r in ranked {
            let e = pl.end_of_turn(r.2.clone());
            if seen.insert(pl.key(&e)) {
                out.push(r);
            }
            if out.len() >= k {
                break;
            }
        }
        (pl, out)
    }

    /// MCTS over whole turns (ISMCTS style): my k best lines at the root, then the opponent's best lines,
    /// then mine... `depth` turns deep. Each iteration re-deals the hidden cards; UCB picks the branch
    /// (the opponent picks what is best for it); the end is scored by `value`. The most visited root line
    /// is played.
    pub fn mcts_pick(
        &mut self,
        me: usize,
        pln: &mut Planner,
        ranked: &[(f64, Vec<Step>, S)],
        iters: usize,
        depth: usize,
        k_my: usize,
        k_opp: usize,
    ) -> Option<(Vec<Step>, S)> {
        let lines = distinct_lines(pln, ranked, k_my);
        if lines.len() < 2 {
            return None;
        }
        let mut nodes = vec![Node::new(lines.len())];
        for _ in 0..iters {
            let (path, v) = self.mcts_descend(me, pln, &lines, &mut nodes, depth, k_my, k_opp);
            for &(nd, a) in &path {
                let x = &mut nodes[nd];
                x.n += 1.0;
                x.w += v;
                if a < x.kn.len() {
                    x.kn[a] += 1.0;
                    x.kw[a] += v;
                }
            }
        }
        let root = &nodes[0];
        let mut best = 0usize;
        for a in 0..root.kn.len() {
            if root.kn[a] > root.kn[best]
                || (root.kn[a] == root.kn[best] && root.kw[a] > root.kw[best])
            {
                best = a;
            }
        }
        RO_STATS[0].fetch_add(1, Ordering::Relaxed);
        if best != 0 {
            RO_STATS[1].fetch_add(1, Ordering::Relaxed);
        }
        self.last_pick = lines[best].0.to_bits();
        Some((lines[best].1.clone(), lines[best].2.clone()))
    }

    /// one MCTS iteration on a re-dealt copy of the game: the visited (node, branch) pairs and the value
    /// for `me` at the end
    fn mcts_descend(
        &mut self,
        me: usize,
        pln: &mut Planner,
        lines: &[&(f64, Vec<Step>, S)],
        nodes: &mut Vec<Node>,
        depth: usize,
        k_my: usize,
        k_opp: usize,
    ) -> (Vec<(usize, usize)>, f64) {
        let sd = self.rng.next();
        let mut g = self.fork(sd);
        g.hide(me);
        let a0 = nodes[0].ucb(true);
        let mut path = vec![(0, a0)];
        g.commit(me, pln, &lines[a0].1, lines[a0].2.clone());
        let (mut node, mut act, mut who) = (0usize, a0, 1 - me);
        for _ in 1..depth {
            if g.p[me].hp <= 0 {
                return (path, 0.0);
            }
            if g.p[1 - me].hp <= 0 {
                return (path, 1.0);
            }
            let mana = match g.begin_turn(who) {
                Some(m) => m,
                None => return (path, if who == me { 0.0 } else { 1.0 }),
            };
            let (mut pl, opts) = g.lines_for(who, mana, if who == me { k_my } else { k_opp });
            let child = Node::child(nodes, node, act, opts.len());
            // a re-dealt position can offer fewer lines than the node knows: only those exist now
            let na = opts.len().min(nodes[child].kn.len()).max(1);
            let pick = nodes[child].ucb_first(na, who == me).min(opts.len() - 1);
            g.commit(who, &mut pl, &opts[pick].1, opts[pick].2.clone());
            path.push((child, pick));
            node = child;
            act = pick;
            who = 1 - who;
        }
        let v = g.value(me);
        (path, v)
    }

    /// a copy of the game for a rollout: fast settings, nothing recorded, its own random stream
    pub fn fork(&self, seed: u64) -> Game<'a> {
        Game {
            t: self.t,
            p: self.p.clone(),
            rng: Rng(seed | 1),
            next_id: self.next_id,
            cardof: self.cardof.clone(),
            max_nodes: self.t.ro_nodes,
            cands: 4,
            depth_left: 1,
            last_pick: 0,
            tno: self.tno,
        }
    }

    /// re-deal what `me` cannot know: the opponent's hand from its hand + deck, my own deck order
    pub fn hide(&mut self, me: usize) {
        let o = 1 - me;
        // cards `me` knows are in that hand (the live game marks them with ids from KNOWN up) stay as they are
        let known: Vec<(i32, usize)> = self.p[o]
            .hand
            .iter()
            .filter(|h| h.0 >= KNOWN)
            .cloned()
            .collect();
        let mut pool: Vec<usize> = self.p[o]
            .hand
            .iter()
            .filter(|h| h.0 < KNOWN)
            .map(|&(_, c)| c)
            .collect();
        let n = pool.len();
        pool.extend(self.p[o].deck.iter().cloned());
        self.rng.shuffle(&mut pool);
        let hand: Vec<usize> = pool.drain(..n.min(pool.len())).collect();
        self.p[o].deck = pool;
        self.p[o].hand = known;
        for c in hand {
            let h = self.card(c);
            self.p[o].hand.push(h);
        }
        let mut d = std::mem::take(&mut self.p[me].deck);
        self.rng.shuffle(&mut d);
        self.p[me].deck = d;
    }

    /// how good the position is for `me`, 0..1: the game result, else health and boards
    pub fn value(&self, me: usize) -> f64 {
        let (a, b) = (&self.p[me], &self.p[1 - me]);
        if a.hp <= 0 {
            return 0.0;
        }
        if b.hp <= 0 {
            return 1.0;
        }
        let bs = |p: &Pl| {
            p.board
                .iter()
                .map(|(u, _)| (u.atk + u.hp) as f64)
                .sum::<f64>()
        };
        let d = (a.hp + a.armor - b.hp - b.armor) as f64 * 0.02 + (bs(a) - bs(b)) * 0.01;
        (0.5 + d).clamp(0.0, 1.0)
    }
}
