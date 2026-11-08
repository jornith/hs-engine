//! Triggered effects. `fire` runs my weapon's and my minions' triggers for one event (hero attack,
//! spell, minion played, hero power, card drawn, an ally grew, health restored, an ally died) in board
//! order; `fire_unit` runs the triggers of one minion (for example "after this takes damage"). Triggers call
//! back into `effects::Planner::apply`.
use crate::planner::*;
use crate::state::*;

impl Planner {
    /// an event (1 hero attack, 2 spell, 3 minion played, 4 hero power, 5 card drawn, 7 a friendly minion grew, 8 health restored, 9 a friendly minion died, 10 this minion took damage): my weapon's and my
    /// minions' triggers for it, in board order
    pub fn fire(&mut self, mut s: S, event: u8, played: Option<i32>, battlecry: bool) -> S {
        if event == 1 {
            if let Some(fx) = self.wtrig.get(&s.wid).cloned() {
                for f in fx {
                    s = self.apply(s, &f, T::None, WEAPON, None);
                }
            }
        }
        let snapshot: Vec<i32> = s.mine.iter().map(|u| u.uid).collect();
        for uid in snapshot {
            if Some(uid) == played || self.silenced.contains(&uid) {
                continue;
            }
            let infos = self.infos.clone();
            let ix = match self.info_of.get(&uid) {
                Some(&ix) if !infos[ix].trig.is_empty() => ix,
                _ => continue,
            };
            for (ev, fx, filt) in &infos[ix].trig {
                if *ev != event || (*filt == 1 && !battlecry) {
                    continue;
                }
                for f in fx {
                    let t = if f.tgt == 5 {
                        played.map(T::Unit).unwrap_or(T::None)
                    } else {
                        T::None
                    };
                    s = self.apply(s, f, t, MINION, Some(uid));
                }
            }
        }
        s
    }

    /// The "after this attacks" triggers of minion `uid`.
    pub fn fire_self(&mut self, s: S, uid: i32) -> S {
        self.fire_unit(s, uid, 6)
    }

    /// only this minion's own triggers for the event (6 it attacked, 10 it took damage)
    pub fn fire_unit(&mut self, mut s: S, uid: i32, event: u8) -> S {
        if !s.mine.iter().any(|x| x.uid == uid) || self.silenced.contains(&uid) {
            return s;
        }
        let trig = match self.info(uid) {
            Some(i) if !i.trig.is_empty() => i.trig.clone(),
            _ => return s,
        };
        for (ev, fx, _) in trig {
            if ev == event {
                for f in fx {
                    s = self.apply(s, &f, T::None, MINION, Some(uid));
                }
            }
        }
        s
    }
}
