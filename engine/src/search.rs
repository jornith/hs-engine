//! The depth-first turn search and its candidate heap. [`Search::dfs`] walks every line of my turn
//! (moves from [`Planner::moves`], best-scored child first), skips positions it has seen by key, and
//! stops at a node or time budget. Every position goes into a fixed-size min-heap of the best end
//! positions by the fast score; [`crate::run_search`] in lib.rs re-ranks that heap with the full
//! evaluation. A line is a list of [`Step`]s, which [`crate::replay`] can play again.
use crate::planner::*;
use crate::state::*;
use rustc_hash::FxHashSet;
use std::time::Instant;

/// One step of a line. Units created during the search have no stable id outside it, so the step keeps the
/// stats of its source and target to find the same unit again.
#[derive(Clone)]
pub struct Step {
    pub act: Act,
    pub ss: Option<Vec<f64>>,
    pub ts: Option<Vec<f64>>,
}

/// The stats row of the unit `t`, if `t` is a unit in `s`.
pub fn stats_of(s: &S, t: &T) -> Option<Vec<f64>> {
    if let T::Unit(uid) = t {
        s.mine
            .iter()
            .chain(s.theirs.iter())
            .find(|u| u.uid == *uid)
            .map(|u| u.stats())
    } else {
        None
    }
}

/// The step for playing `act` in `s`.
pub fn step_of(s: &S, act: &Act) -> Step {
    let (ss, ts) = match act {
        Act::Attack { src, tgt } => (stats_of(s, src), stats_of(s, tgt)),
        Act::Play { t, .. } => (None, stats_of(s, t)),
    };
    Step {
        act: act.clone(),
        ss,
        ts,
    }
}

/// A candidate end position in the heap: fast score, visit order (ties go to the earlier one), line and
/// position.
#[derive(Clone)]
pub struct Entry {
    pub sc: f64,
    pub n: usize,
    pub path: Vec<Step>,
    pub st: S,
}

/// The heap order: lower score first, then the earlier visit.
pub fn lt(a: &Entry, b: &Entry) -> bool {
    a.sc < b.sc || (a.sc == b.sc && a.n < b.n)
}

/// The sift-down of a binary min-heap, in the same order as Python's `heapq`.
pub fn siftdown(h: &mut [Entry], start: usize, mut pos: usize) {
    let item = h[pos].clone();
    while pos > start {
        let parent = (pos - 1) >> 1;
        if lt(&item, &h[parent]) {
            h[pos] = h[parent].clone();
            pos = parent;
            continue;
        }
        break;
    }
    h[pos] = item;
}

/// The sift-up of a binary min-heap, in the same order as Python's `heapq`.
pub fn siftup(h: &mut [Entry], mut pos: usize) {
    let end = h.len();
    let start = pos;
    let item = h[pos].clone();
    let mut child = 2 * pos + 1;
    while child < end {
        let right = child + 1;
        if right < end && !lt(&h[child], &h[right]) {
            child = right;
        }
        h[pos] = h[child].clone();
        pos = child;
        child = 2 * pos + 1;
    }
    h[pos] = item;
    siftdown(h, start, pos);
}

/// The state of one turn search: budget, positions seen, the candidate heap and the best line so far
/// (a lethal line beats any other, a shorter lethal line beats a longer one).
pub struct Search<'a> {
    pub p: &'a mut Planner,
    pub deadline: Instant,
    pub max_nodes: usize,
    pub candidates: usize,
    pub nodes: usize,
    pub seen: FxHashSet<Key>,
    pub top: Vec<Entry>,
    pub best: (f64, Vec<Step>, S),
}

impl<'a> Search<'a> {
    /// Whether the node or time budget is spent.
    pub fn out_of_budget(&self) -> bool {
        self.nodes > self.max_nodes || Instant::now() > self.deadline
    }

    /// Visit `s` (key `k`, reached by `path`): record it, then search its children best first.
    pub fn dfs(&mut self, s: &S, k: Key, path: &mut Vec<Step>) {
        self.nodes += 1;
        if self.seen.contains(&k) || self.out_of_budget() {
            return;
        }
        self.seen.insert(k.clone());
        let sc = self.p.eval_cached(s, &k);
        if s.en_hp <= 0 {
            if self.best.2.en_hp > 0 || path.len() < self.best.1.len() {
                self.best = (sc, path.clone(), s.clone());
            }
        } else if sc > self.best.0 && self.best.2.en_hp > 0 {
            self.best = (sc, path.clone(), s.clone());
        }
        let e = Entry {
            sc,
            n: self.nodes,
            path: path.clone(),
            st: s.clone(),
        };
        if self.top.len() < self.candidates {
            self.top.push(e);
            let last = self.top.len() - 1;
            siftdown(&mut self.top, 0, last);
        } else if sc > self.top[0].sc {
            self.top[0] = e;
            siftup(&mut self.top, 0);
        }
        if s.en_hp <= 0 {
            return;
        }
        let moves = self.p.moves(s);
        let mut kids: Vec<(f64, Act, S, Key)> = Vec::with_capacity(moves.len());
        for (act, ns) in moves {
            let nk = self.p.key(&ns);
            if !self.seen.contains(&nk) {
                let v = self.p.eval_cached(&ns, &nk);
                kids.push((v, act, ns, nk));
            }
        }
        kids.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        for (_, act, ns, nk) in kids {
            path.push(step_of(s, &act));
            self.dfs(&ns, nk, path);
            path.pop();
            if self.out_of_budget() {
                return;
            }
        }
    }
}
