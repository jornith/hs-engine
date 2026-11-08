//! Python bindings: `hsengine._core`. Converts the dict context built by `hsengine.planner` into engine
//! types and exposes `Engine`, `search_once` and `selfplay`. No rules live here. `build` reads the
//! context of one turn, `parse_table` the card table of a self-play run; results go back as plain lists
//! and dicts.
use hs_engine::game::{CardDef, RO_STATS, Rng, Table, new_game};
use hs_engine::*;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use rustc_hash::{FxHashMap, FxHashSet};

/// The value of key `k` in `d`, converted to `T`; an error names a missing key.
pub(crate) fn get<'py, T>(d: &Bound<'py, PyDict>, k: &str) -> PyResult<T>
where
    T: for<'a> FromPyObject<'a, 'py>,
    for<'a> <T as FromPyObject<'a, 'py>>::Error: Into<PyErr>,
{
    match d.get_item(k)? {
        Some(v) => v.extract::<T>().map_err(Into::into),
        None => Err(PyValueError::new_err(format!("missing {k}"))),
    }
}

fn parse_fx(o: &Bound<'_, PyAny>) -> PyResult<Fx> {
    let (op, n, a, h, kw, tgt, cond, scope): (String, i32, i32, i32, u16, String, String, String) =
        o.extract()?;
    Ok(Fx::parse(&op, n, a, h, kw, &tgt, &cond, &scope))
}

fn units(rows: &[Vec<f64>]) -> PyResult<Vec<U>> {
    rows.iter()
        .map(|v| U::from_vec(v).map_err(PyValueError::new_err))
        .collect()
}

fn parse_s(o: &Bound<'_, PyAny>) -> PyResult<S> {
    let d = o.cast::<PyDict>()?;
    let mine: Vec<Vec<f64>> = get(d, "mine")?;
    let theirs: Vec<Vec<f64>> = get(d, "theirs")?;
    Ok(S {
        mana: get(d, "mana")?,
        hand: get(d, "hand")?,
        mine: units(&mine)?,
        theirs: units(&theirs)?,
        my_hp: get(d, "my_hp")?,
        my_armor: get(d, "my_armor")?,
        en_hp: get(d, "en_hp")?,
        hero_atk: get(d, "hero_atk")?,
        hero_att: get(d, "hero_att")?,
        bonus: get(d, "bonus")?,
        watk: get(d, "watk")?,
        wdur: get(d, "wdur")?,
        corpses: get(d, "corpses")?,
        died: get(d, "died")?,
        cannon: get(d, "cannon")?,
        drawn: get(d, "drawn")?,
        played: d
            .get_item("played")?
            .map(|v| v.extract::<i32>())
            .transpose()?
            .unwrap_or(0),
        wid: get(d, "wid")?,
    })
}

fn fx_list(o: &Bound<'_, PyAny>) -> PyResult<Vec<Fx>> {
    o.cast::<PyList>()?.iter().map(|f| parse_fx(&f)).collect()
}

fn item<'py>(d: &Bound<'py, PyDict>, k: &str) -> PyResult<Bound<'py, PyAny>> {
    d.get_item(k)?
        .ok_or_else(|| PyValueError::new_err(format!("missing {k}")))
}

fn opt<'py, T>(d: &Bound<'py, PyDict>, k: &str, default: T) -> PyResult<T>
where
    T: for<'a> FromPyObject<'a, 'py>,
    for<'a> <T as FromPyObject<'a, 'py>>::Error: Into<PyErr>,
{
    match d.get_item(k)? {
        Some(v) if !v.is_none() => v.extract::<T>().map_err(Into::into),
        _ => Ok(default),
    }
}

fn parse_info(d: &Bound<'_, PyDict>, names: &mut FxHashMap<String, i32>) -> PyResult<Info> {
    let name: String = get(d, "name")?;
    let nl = names.len() as i32;
    let nid = *names.entry(name).or_insert(nl);
    let mut modes = Vec::new();
    for m in item(d, "modes")?.cast::<PyList>()?.iter() {
        let (fxl, unk): (Bound<'_, PyAny>, bool) = m.extract()?;
        modes.push((fx_list(&fxl)?, unk));
    }
    let mut trig = Vec::new();
    for t in item(d, "trig")?.cast::<PyList>()?.iter() {
        let (ev, fxl, filt): (u8, Bound<'_, PyAny>, u8) = t.extract()?;
        trig.push((ev, fx_list(&fxl)?, filt));
    }
    let kind: String = get(d, "kind")?;
    Ok(Info {
        name: nid,
        kind: kind_code(&kind),
        cost: get(d, "cost")?,
        atk: get(d, "atk")?,
        hp: get(d, "hp")?,
        kw: get(d, "kw")?,
        sp: get(d, "sp")?,
        modes,
        dr: get(d, "dr")?,
        aura: get(d, "aura")?,
        overload: get(d, "overload")?,
        drs: get(d, "drs")?,
        dr_tok: opt(d, "dr_tok", 0)?,
        eot: fx_list(&item(d, "eot")?)?,
        pv: get(d, "pv")?,
        cannon: get(d, "cannon")?,
        trig,
        wid: get(d, "wid")?,
        adj: opt(d, "adj", 0)?,
        opp_draw: opt(d, "opp_draw", 0)?,
        race: opt(d, "race", 0)?,
        dr_gen: opt(d, "dr_gen", 0)?,
    })
}

fn build(ctx: &Bound<'_, PyDict>) -> PyResult<(Planner, S)> {
    let wd: FxHashMap<String, f64> = get(ctx, "w")?;
    let mut w = Wt::default();
    for (k, v) in &wd {
        w.set(k, *v);
    }
    let mut names: FxHashMap<String, i32> = FxHashMap::default();
    let mut cards = Vec::new();
    for it in item(ctx, "infos")?.cast::<PyList>()?.iter() {
        let d = it.cast::<PyDict>()?;
        cards.push((get::<i32>(d, "uid")?, parse_info(d, &mut names)?));
    }
    let set = |k: &str| -> PyResult<FxHashSet<i32>> {
        Ok(get::<Vec<i32>>(ctx, k)?.into_iter().collect())
    };
    let mut p = Planner::new(w, cards);
    p.hold = get::<Vec<(i32, f64)>>(ctx, "hold")?.into_iter().collect();
    p.corpse_cost = set("corpse_cost")?;
    p.death_discount = set("death_discount")?;
    p.maiev = set("maiev")?;
    p.dormant = set("dormant")?;
    p.towers = set("towers")?;
    p.enemy_towers = set("enemy_towers")?;
    p.enemy_nozdormu = set("enemy_nozdormu")?;
    p.enemy_thoret = get(ctx, "enemy_thoret")?;
    p.wlife = set("wlife")?;
    p.iwa = set("iwa")?;
    p.sp_extra = get(ctx, "sp_extra")?;
    p.enemy_split = opt(ctx, "enemy_split", 0)?;
    for t in item(ctx, "wtrig")?.cast::<PyList>()?.iter() {
        let (code, fxl): (i32, Bound<'_, PyAny>) = t.extract()?;
        p.wtrig.insert(code, fx_list(&fxl)?);
    }
    p.played_cheap = get(ctx, "played_cheap")?;
    p.max_hp = get(ctx, "max_hp")?;
    p.max_mana = get(ctx, "max_mana")?;
    p.enemy_hand = get(ctx, "enemy_hand")?;
    p.role = get(ctx, "role")?;
    p.aoe_threat = get(ctx, "aoe_threat")?;
    p.enemy_swing = get(ctx, "enemy_swing")?;
    p.enemy_direct = get(ctx, "enemy_direct")?;
    p.dk = get(ctx, "dk")?;
    p.power_uid = get(ctx, "power_uid")?;
    p.draw_uid = get(ctx, "draw_uid")?;
    p.uid = get(ctx, "uid_start")?;
    let mut s0 = parse_s(&item(ctx, "s0")?)?;
    p.norm_adj(&mut s0);
    Ok((p, s0))
}

fn t_py(py: Python<'_>, t: &T) -> Py<PyAny> {
    match t {
        T::None => py.None(),
        T::Hero => "hero".into_pyobject(py).unwrap().into_any().unbind(),
        T::Face => "face".into_pyobject(py).unwrap().into_any().unbind(),
        T::Unit(u) => u.into_pyobject(py).unwrap().into_any().unbind(),
    }
}

/// per step: (kind, src, tgt, mode, src stats or None, tgt stats or None)
fn path_py(py: Python<'_>, path: &[Step]) -> PyResult<Py<PyAny>> {
    let out = PyList::empty(py);
    for st in path {
        let (kind, src, tgt, mi) = match &st.act {
            Act::Attack { src, tgt } => ("attack", t_py(py, src), t_py(py, tgt), 0usize),
            Act::Play { uid, t, mi } => (
                "play",
                uid.into_pyobject(py)?.into_any().unbind(),
                t_py(py, t),
                *mi,
            ),
        };
        out.append((kind, src, tgt, mi, st.ss.clone(), st.ts.clone()).into_pyobject(py)?)?;
    }
    Ok(out.into_any().unbind())
}

/// One-shot turn search from a context dict: {"score", "path", "ranked", "lethal", "nodes"}.
#[pyfunction(name = "search")]
#[pyo3(signature = (ctx, budget, candidates, max_nodes))]
fn search_once(
    py: Python<'_>,
    ctx: &Bound<'_, PyDict>,
    budget: f64,
    candidates: usize,
    max_nodes: usize,
) -> PyResult<Py<PyAny>> {
    let (mut p, s0) = build(ctx)?;
    let r = run_search(&mut p, &s0, budget, candidates, max_nodes);
    let result = PyDict::new(py);
    result.set_item("nodes", r.nodes)?;
    result.set_item("score", r.score)?;
    result.set_item("path", path_py(py, &r.path)?)?;
    let rl = PyList::empty(py);
    for (sc, pth, _) in &r.ranked {
        rl.append((*sc, path_py(py, pth)?))?;
    }
    result.set_item("ranked", rl)?;
    result.set_item("lethal", r.lethal)?;
    Ok(result.into_any().unbind())
}

// ------------------------------------------------------------------ the engine object
// Moves from a position, single effects, the end of my turn, the enemy reply, the scores, the turn search
// and its lines with the position after every step.

fn s_py(py: Python<'_>, s: &S) -> PyResult<Py<PyAny>> {
    let d = PyDict::new(py);
    let units = |side: &[U]| -> Vec<Vec<f64>> {
        side.iter()
            .map(|u| {
                let mut v = vec![u.uid as f64];
                v.extend(u.stats());
                v
            })
            .collect()
    };
    d.set_item("mana", s.mana)?;
    d.set_item("hand", s.hand.clone())?;
    d.set_item("mine", units(&s.mine))?;
    d.set_item("theirs", units(&s.theirs))?;
    d.set_item("my_hp", s.my_hp)?;
    d.set_item("my_armor", s.my_armor)?;
    d.set_item("en_hp", s.en_hp)?;
    d.set_item("hero_atk", s.hero_atk)?;
    d.set_item("hero_att", s.hero_att)?;
    d.set_item("bonus", s.bonus)?;
    d.set_item("watk", s.watk)?;
    d.set_item("wdur", s.wdur)?;
    d.set_item("corpses", s.corpses)?;
    d.set_item("died", s.died)?;
    d.set_item("cannon", s.cannon)?;
    d.set_item("drawn", s.drawn)?;
    d.set_item("played", s.played)?;
    d.set_item("wid", s.wid)?;
    Ok(d.into_any().unbind())
}

fn t_of(o: &Bound<'_, PyAny>) -> PyResult<T> {
    if o.is_none() {
        return Ok(T::None);
    }
    if let Ok(s) = o.extract::<String>() {
        return match s.as_str() {
            "hero" => Ok(T::Hero),
            "face" => Ok(T::Face),
            _ => Err(PyValueError::new_err(format!("target {s}"))),
        };
    }
    Ok(T::Unit(o.extract::<i32>()?))
}

fn act_py(py: Python<'_>, a: &Act) -> PyResult<Py<PyAny>> {
    Ok(match a {
        Act::Attack { src, tgt } => ("attack", t_py(py, src), t_py(py, tgt))
            .into_pyobject(py)?
            .into_any()
            .unbind(),
        Act::Play { uid, t, mi } => ("play", *uid, t_py(py, t), *mi)
            .into_pyobject(py)?
            .into_any()
            .unbind(),
    })
}

/// One position and its card models, for tests and analysis: list moves, apply effects, run the search.
#[pyclass(unsendable)]
struct Engine {
    p: Planner,
    s0: S,
    best: Vec<Step>,
    ranked: Vec<(f64, Vec<Step>, S)>,
}

#[pymethods]
impl Engine {
    /// Build the planner and the start position from a context dict (see `hsengine.planner`).
    #[new]
    fn new(ctx: &Bound<'_, PyDict>) -> PyResult<Engine> {
        let (p, s0) = build(ctx)?;
        Ok(Engine {
            p,
            s0,
            best: vec![],
            ranked: vec![],
        })
    }
    /// The start position as a dict.
    fn s0(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        s_py(py, &self.s0)
    }
    /// every move from s: [(action, position after it)]
    fn moves(&mut self, py: Python<'_>, s: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let mut s = parse_s(s)?;
        self.p.norm_adj(&mut s);
        let out = PyList::empty(py);
        for (a, ns) in self.p.moves(&s) {
            out.append((act_py(py, &a)?, s_py(py, &ns)?))?;
        }
        Ok(out.into_any().unbind())
    }
    /// one effect on a position (tests): f = (op, n, a, h, kw bits, tgt, cond, scope)
    #[pyo3(signature = (s, f, t, kind, exclude=None))]
    fn apply(
        &mut self,
        py: Python<'_>,
        s: &Bound<'_, PyAny>,
        f: &Bound<'_, PyAny>,
        t: &Bound<'_, PyAny>,
        kind: &str,
        exclude: Option<i32>,
    ) -> PyResult<Py<PyAny>> {
        let s = parse_s(s)?;
        let f = parse_fx(f)?;
        let ns = self.p.apply(s, &f, t_of(t)?, kind_code(kind), exclude);
        s_py(py, &ns)
    }
    /// The position after my end of turn.
    fn end_of_turn(&mut self, py: Python<'_>, s: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let s = parse_s(s)?;
        let ns = self.p.end_of_turn(s);
        s_py(py, &ns)
    }
    /// The position after the simulated enemy reply.
    fn enemy_turn(&mut self, py: Python<'_>, s: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let s = parse_s(s)?;
        let ns = self.p.enemy_turn(&s);
        s_py(py, &ns)
    }
    /// The static score of a position.
    #[pyo3(signature = (s, threat=true))]
    fn evaluate(&self, s: &Bound<'_, PyAny>, threat: bool) -> PyResult<f64> {
        Ok(self.p.evaluate(&parse_s(s)?, threat))
    }
    /// The score of ending the turn in a position (end of turn, enemy reply, risks, optional look-ahead).
    #[pyo3(signature = (s, ahead=true))]
    fn refined(&mut self, s: &Bound<'_, PyAny>, ahead: bool) -> PyResult<f64> {
        let s = parse_s(s)?;
        Ok(self.p.refined(&s, ahead))
    }
    /// The damage the enemy board could deal next turn.
    fn threat(&self, s: &Bound<'_, PyAny>) -> PyResult<i32> {
        Ok(self.p.threat(&parse_s(s)?))
    }
    /// the minion a reborn copy came back from (None: not a reborn copy)
    fn origin(&self, uid: i32) -> Option<i32> {
        self.p.origin.get(&uid).copied()
    }
    /// the turn search: (score, nodes, lethal, number of ranked lines); line(i) gives the lines
    fn search(
        &mut self,
        py: Python<'_>,
        budget: f64,
        candidates: usize,
        max_nodes: usize,
    ) -> (f64, usize, bool, usize) {
        let s0 = self.s0.clone();
        let p = &mut self.p;
        // without the interpreter lock: other Python threads keep running during the search
        let r = py.detach(|| run_search(p, &s0, budget, candidates, max_nodes));
        self.best = r.path;
        self.ranked = r.ranked;
        (r.score, r.nodes, r.lethal, self.ranked.len())
    }
    /// The refined scores of the ranked lines of the last search.
    fn ranked_scores(&self) -> Vec<f64> {
        self.ranked.iter().map(|r| r.0).collect()
    }
    /// line i of the last search (-1: the best one) as [(action, position after it)]
    fn line(&mut self, py: Python<'_>, i: isize) -> PyResult<Py<PyAny>> {
        let steps = if i < 0 {
            self.best.clone()
        } else {
            self.ranked
                .get(i as usize)
                .ok_or_else(|| PyValueError::new_err("no such line"))?
                .1
                .clone()
        };
        let s0 = self.s0.clone();
        let out = PyList::empty(py);
        for (a, ns) in replay(&mut self.p, &s0, &steps).map_err(PyValueError::new_err)? {
            out.append((act_py(py, &a)?, s_py(py, &ns)?))?;
        }
        Ok(out.into_any().unbind())
    }
}

// ------------------------------------------------------------------ self-play

/// A deck: (card-table indexes, hero power index, a Death Knight deck).
type Deck = (Vec<usize>, usize, bool);

fn parse_table(ctx: &Bound<'_, PyDict>) -> PyResult<(Table, Vec<Deck>)> {
    let (p, _) = build(ctx)?; // weights and the card models the usual way
    let mut cards = Vec::new();
    for it in item(ctx, "cards")?.cast::<PyList>()?.iter() {
        let d = it.cast::<PyDict>()?;
        let wtrig = match d.get_item("wtrig")? {
            Some(w) if !w.is_none() => Some(fx_list(&w)?),
            _ => None,
        };
        let ix: i32 = get(d, "uid")?;
        cards.push(CardDef {
            info_ix: *p
                .info_of
                .get(&ix)
                .ok_or_else(|| PyValueError::new_err("card info"))?,
            kw: get(d, "ekw")?,
            hold: get(d, "hold")?,
            corpse_cost: get(d, "corpse_cost")?,
            death_discount: get(d, "death_discount")?,
            dormant: get(d, "dormant")?,
            tower: get(d, "tower")?,
            maiev: get(d, "maiev")?,
            nozdormu: get(d, "nozdormu")?,
            thoret: get(d, "thoret")?,
            wlife: get(d, "wlife")?,
            wtrig,
            p_direct: get(d, "p_direct")?,
            p_swing: get(d, "p_swing")?,
            cheap: get(d, "cheap")?,
        });
    }
    let decks: Vec<Deck> = get(ctx, "decks")?;
    let mut cfg = vec![(p.w.clone(),)];
    if let Some(wb) = ctx.get_item("w_b")? {
        if !wb.is_none() {
            let wd: FxHashMap<String, f64> = wb.extract()?;
            let mut w2 = p.w.clone();
            for (k, v) in wd {
                w2.set(&k, v);
            }
            cfg.push((w2,));
        }
    }
    let t = Table {
        cards,
        infos: p.infos.clone(),
        cfg,
        ro: opt(ctx, "ro", vec![(0, 0, 0)])?,
        ro_nodes: opt(ctx, "ro_nodes", 60)?,
        mcts: opt(ctx, "mcts", vec![(0, 0, 0, 0)])?,
        ro_worst: opt(ctx, "ro_worst", false)?,
        w: p.w.clone(),
        coin: get(ctx, "coin")?,
        hold_minion: get(ctx, "hold_minion")?,
    };
    Ok((t, decks))
}

/// (deck a, deck b, winner seat or -1, turns, B's seat or -1)
type GameResult = (usize, usize, i32, i32, i32);

/// Self-play: `n_games` games between random pairs of `ctx["decks"]` on `threads` threads, both sides played
/// by the turn search with the hand-tuned weights. With `w_b` in the context, side B uses those weight
/// overrides and alternates seats. Returns {"results": [(deck a, deck b, winner seat or -1, turns, B's seat
/// or -1)], "seconds", "ro_decisions", "ro_changed"}.
#[pyfunction]
#[pyo3(signature = (ctx, n_games, seed, max_nodes, cands, threads))]
fn selfplay(
    py: Python<'_>,
    ctx: &Bound<'_, PyDict>,
    n_games: usize,
    seed: u64,
    max_nodes: usize,
    cands: usize,
    threads: usize,
) -> PyResult<Py<PyAny>> {
    let (t, decks) = parse_table(ctx)?;
    let t = std::sync::Arc::new(t);
    let decks = std::sync::Arc::new(decks);
    let t0 = std::time::Instant::now();
    let res: Vec<Vec<GameResult>> = py.detach(|| {
        let mut hs = Vec::new();
        for th in 0..threads.max(1) {
            let t = t.clone();
            let decks = decks.clone();
            hs.push(std::thread::spawn(move || {
                let mut out = Vec::new();
                let mut rng = Rng((seed ^ ((th as u64 + 1) * 0xD1B54A32D192ED03)) | 1);
                let mut k = th;
                while k < n_games {
                    let (a, b) = (rng.below(decks.len()), rng.below(decks.len()));
                    let mut g = new_game(&t, [&decks[a], &decks[b]], rng.next(), max_nodes, cands);
                    let b_seat = if t.cfg.len() > 1 { (k % 2) as i32 } else { -1 };
                    if b_seat >= 0 {
                        g.p[b_seat as usize].cfg = 1;
                    }
                    let w = g.play(60);
                    let turns = g.p[0].turns + g.p[1].turns;
                    out.push((a, b, w, turns, b_seat));
                    k += threads.max(1);
                }
                out
            }));
        }
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let result = PyDict::new(py);
    result.set_item("results", res.into_iter().flatten().collect::<Vec<_>>())?;
    result.set_item("seconds", t0.elapsed().as_secs_f64())?;
    result.set_item(
        "ro_decisions",
        RO_STATS[0].swap(0, std::sync::atomic::Ordering::Relaxed),
    )?;
    result.set_item(
        "ro_changed",
        RO_STATS[1].swap(0, std::sync::atomic::Ordering::Relaxed),
    )?;
    Ok(result.into_any().unbind())
}

/// The extension module `hsengine._core`.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(search_once, m)?)?;
    m.add_function(wrap_pyfunction!(selfplay, m)?)?;
    m.add_class::<Engine>()?;
    Ok(())
}
