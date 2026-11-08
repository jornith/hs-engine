//! A one-turn Hearthstone engine: card effects, combat, deaths, triggers, the end of the turn, a simulated
//! enemy reply, a position score and a depth-first search over one turn.
//!
//! The crate has no I/O. Card models ([`Info`]) and the start position ([`S`]) come from the caller; the
//! Python package `hsengine` builds them from a game log and card texts.
#![allow(
    clippy::collapsible_if,
    clippy::needless_range_loop,
    clippy::type_complexity
)]

pub mod effects;
pub mod eval;
pub mod game;
pub mod moves;
pub mod planner;
pub mod reply;
pub mod search;
pub mod state;
pub mod weights;

pub use planner::{Key, Planner};
pub use search::{Entry, Search, Step};
pub use state::*;
pub use weights::Wt;

use rustc_hash::FxHashSet;
use std::time::{Duration, Instant};

/// Result of [`run_search`].
pub struct SearchResult {
    /// Score of the chosen line (refined, or the fast score for a lethal line).
    pub score: f64,
    /// The chosen line.
    pub path: Vec<Step>,
    /// The position at the end of the chosen line.
    pub last: S,
    /// The chosen line kills the enemy hero.
    pub lethal: bool,
    /// Candidate lines, best first: (refined score, line, final position). Empty when lethal.
    pub ranked: Vec<(f64, Vec<Step>, S)>,
    /// Positions visited.
    pub nodes: usize,
}

/// The turn search: a DFS over my moves keeps the `candidates` best end positions by the fast score, then
/// re-ranks them by the score after the enemy reply; the first 12 also get the next-turn look-ahead.
pub fn run_search(
    p: &mut Planner,
    s0: &S,
    budget: f64,
    candidates: usize,
    max_nodes: usize,
) -> SearchResult {
    let deadline = Instant::now() + Duration::from_secs_f64(budget.max(0.0));
    let k0 = p.key(s0);
    let sc0 = p.eval_cached(s0, &k0);
    let mut sr = Search {
        p,
        deadline,
        max_nodes,
        candidates,
        nodes: 0,
        seen: FxHashSet::default(),
        top: vec![Entry {
            sc: sc0,
            n: 0,
            path: vec![],
            st: s0.clone(),
        }],
        best: (sc0, vec![], s0.clone()),
    };
    let mut path = Vec::new();
    sr.dfs(s0, k0, &mut path);
    let nodes = sr.nodes;
    let top = std::mem::take(&mut sr.top);
    let best = sr.best.clone();
    let p = sr.p;
    if best.2.en_hp <= 0 {
        return SearchResult {
            score: best.0,
            path: best.1,
            last: best.2,
            lethal: true,
            ranked: vec![],
            nodes,
        };
    }
    let mut pre: Vec<(f64, f64, Vec<Step>, S)> = Vec::new();
    for e in top {
        let r0 = p.refined(&e.st, false);
        pre.push((r0, e.sc, e.path, e.st));
    }
    pre.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let mut scored: Vec<(f64, i64, f64, Vec<Step>, S)> = Vec::new();
    for (i, (r0, fast, path, st)) in pre.into_iter().enumerate() {
        let r = if i < 12 {
            p.refined(&st, true)
        } else {
            r0 - 1e3
        };
        scored.push((r, -(path.len() as i64), fast, path, st));
    }
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap()
            .then(b.1.cmp(&a.1))
            .then(b.2.partial_cmp(&a.2).unwrap())
    });
    let (score, _, _, path, last) = scored[0].clone();
    let ranked = scored
        .into_iter()
        .map(|(r, _, _, p, s)| (r, p, s))
        .collect();
    SearchResult {
        score,
        path,
        last,
        lethal: false,
        ranked,
        nodes,
    }
}

fn same_t(p: &T, r: &T, stats: &Option<Vec<f64>>, exact: bool, s: &S) -> bool {
    match (p, r) {
        (T::Unit(a), T::Unit(_)) if !exact => match stats {
            Some(st) => s
                .mine
                .iter()
                .chain(s.theirs.iter())
                .any(|u| u.uid == *a && u.stats() == *st),
            None => false,
        },
        _ => p == r,
    }
}

/// Play a line found by the search again from `s0`: [(action, position after it)]. Units created during the
/// search have other ids on the second run (the id counter moved on), so they are matched by their stats.
pub fn replay(p: &mut Planner, s0: &S, steps: &[Step]) -> Result<Vec<(Act, S)>, String> {
    let mut s = s0.clone();
    let mut out = Vec::new();
    for st in steps {
        let moves = p.moves(&s);
        let mut found = None;
        for exact in [true, false] {
            for (act, ns) in &moves {
                let ok = match (act, &st.act) {
                    (Act::Attack { src: a, tgt: b }, Act::Attack { src: c, tgt: d }) => {
                        same_t(a, c, &st.ss, exact, &s) && same_t(b, d, &st.ts, exact, &s)
                    }
                    (
                        Act::Play {
                            uid: u1,
                            t: t1,
                            mi: m1,
                        },
                        Act::Play {
                            uid: u2,
                            t: t2,
                            mi: m2,
                        },
                    ) => u1 == u2 && m1 == m2 && same_t(t1, t2, &st.ts, exact, &s),
                    _ => false,
                };
                if ok {
                    found = Some((act.clone(), ns.clone()));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        match found {
            Some((a, ns)) => {
                s = ns.clone();
                out.push((a, ns));
            }
            None => return Err("the line does not replay".to_string()),
        }
    }
    Ok(out)
}
