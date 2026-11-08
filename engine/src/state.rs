//! Game state types and the codes shared by every rule. [`S`] is a position during my turn and [`U`] a
//! minion; both are plain values the search copies freely. [`Info`] is the compiled model of one card and
//! [`Fx`] one effect of it, with op, target, condition and scope as small integer codes (`*_code`
//! functions map the rule-file strings to them). Keyword bits (`K_*`) and card kinds also live here.

/// Keyword bit: Taunt. The `K_*` bits combine in `Info::kw` and `Fx::kw`.
pub const K_TAUNT: u16 = 1;
/// Keyword bit: Divine Shield.
pub const K_DIVINE: u16 = 2;
/// Keyword bit: Windfury.
pub const K_WF: u16 = 4;
/// Keyword bit: Charge.
pub const K_CHARGE: u16 = 8;
/// Keyword bit: Rush.
pub const K_RUSH: u16 = 16;
/// Keyword bit: Stealth.
pub const K_STEALTH: u16 = 32;
/// Keyword bit: Lifesteal.
pub const K_LIFE: u16 = 64;
/// Keyword bit: Poisonous.
pub const K_POISON: u16 = 128;
/// Keyword bit: Reborn.
pub const K_REBORN: u16 = 256;
/// Keyword bit: dies at the end of the turn.
pub const K_DOOMED: u16 = 512;

/// Card kind: minion. Kind codes are `Info::kind`; see [`kind_code`].
pub const MINION: u8 = 0;
/// Card kind: spell.
pub const SPELL: u8 = 1;
/// Card kind: weapon.
pub const WEAPON: u8 = 2;
/// Card kind: hero power.
pub const POWER: u8 = 3;
/// Card kind: hero card.
pub const HERO_K: u8 = 4;
/// Card kind: location (played to the board).
pub const LOCATION: u8 = 5;
/// Card kind: activating a location already on the board.
pub const LOCACT: u8 = 6;

/// The kind code of a card type name; 7 for anything else.
pub fn kind_code(s: &str) -> u8 {
    match s {
        "MINION" => MINION,
        "SPELL" => SPELL,
        "WEAPON" => WEAPON,
        "POWER" => POWER,
        "HERO" => HERO_K,
        "LOCATION" => LOCATION,
        "LOCACT" => LOCACT,
        _ => 7,
    }
}

// ------------------------------------------------------------------ state
/// A minion on the board. `att` is attacks left this turn, `fresh` means summoned this turn, `tmp` is an
/// attack change that ends with the turn, `dr` and `pv` are the evaluation's deathrattle and extra value,
/// `shield` counts extra Divine Shield layers and `swung` marks a minion that attacked.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct U {
    pub uid: i32,
    pub atk: i32,
    pub hp: i32,
    pub maxhp: i32,
    pub taunt: bool,
    pub divine: bool,
    pub att: i32,
    pub stealth: bool,
    pub rush: bool,
    pub poison: bool,
    pub life: bool,
    pub wf: bool,
    pub frozen: bool,
    pub fresh: bool,
    pub dr: f64,
    pub sp: i32,
    pub elusive: bool,
    pub immune: bool,
    pub tmp: i32,
    pub pv: f64,
    pub reborn: bool,
    pub doomed: bool,
    pub swung: bool,
    pub shield: i32,
    /// Minion type bits (one bit per type, ALL sets every bit); 0 when the type is unknown.
    pub race: u32,
    pub adjb: i32, // attack bonus from adjacent auras currently applied (see adj_side)
}

/// The bits of a float with -0.0 folded into 0.0, for hashing and equality.
pub fn fbits(x: f64) -> i64 {
    (x + 0.0).to_bits() as i64
}

impl U {
    /// A minion with these stats and no keywords.
    pub fn new(uid: i32, atk: i32, hp: i32, maxhp: i32) -> U {
        U {
            uid,
            atk,
            hp,
            maxhp,
            taunt: false,
            divine: false,
            att: 0,
            stealth: false,
            rush: false,
            poison: false,
            life: false,
            wf: false,
            frozen: false,
            fresh: false,
            dr: 0.0,
            sp: 0,
            elusive: false,
            immune: false,
            tmp: 0,
            pv: 0.0,
            reborn: false,
            doomed: false,
            swung: false,
            shield: 0,
            race: 0,
            adjb: 0,
        }
    }
    /// Everything but the id: identical units are interchangeable.
    pub fn sig(&self) -> [i64; 24] {
        [
            self.atk as i64,
            self.hp as i64,
            self.maxhp as i64,
            self.taunt as i64,
            self.divine as i64,
            self.att as i64,
            self.stealth as i64,
            self.rush as i64,
            self.poison as i64,
            self.life as i64,
            self.wf as i64,
            self.frozen as i64,
            self.fresh as i64,
            fbits(self.dr),
            self.sp as i64,
            self.elusive as i64,
            self.immune as i64,
            self.tmp as i64,
            fbits(self.pv),
            self.reborn as i64,
            self.doomed as i64,
            self.swung as i64,
            self.shield as i64,
            self.race as i64,
        ]
    }
    /// The unit as its row without the id: the inverse of [`U::from_vec`] minus `uid`.
    pub fn stats(&self) -> Vec<f64> {
        vec![
            self.atk as f64,
            self.hp as f64,
            self.maxhp as f64,
            self.taunt as i32 as f64,
            self.divine as i32 as f64,
            self.att as f64,
            self.stealth as i32 as f64,
            self.rush as i32 as f64,
            self.poison as i32 as f64,
            self.life as i32 as f64,
            self.wf as i32 as f64,
            self.frozen as i32 as f64,
            self.fresh as i32 as f64,
            self.dr,
            self.sp as f64,
            self.elusive as i32 as f64,
            self.immune as i32 as f64,
            self.tmp as f64,
            self.pv,
            self.reborn as i32 as f64,
            self.doomed as i32 as f64,
            self.swung as i32 as f64,
            self.shield as f64,
            self.race as f64,
        ]
    }
    /// A unit from its row of 24 numbers `[uid, atk, hp, maxhp, taunt, divine, att, stealth, rush, poison,
    /// life, wf, frozen, fresh, dr, sp, elusive, immune, tmp, pv, reborn, doomed, swung, shield]`, or 25 with
    /// the minion type bits (`race`) last.
    pub fn from_vec(v: &[f64]) -> Result<U, String> {
        if v.len() != 24 && v.len() != 25 {
            return Err(format!("unit needs 24 or 25 fields, got {}", v.len()));
        }
        let b = |x: f64| x != 0.0;
        Ok(U {
            uid: v[0] as i32,
            atk: v[1] as i32,
            hp: v[2] as i32,
            maxhp: v[3] as i32,
            taunt: b(v[4]),
            divine: b(v[5]),
            att: v[6] as i32,
            stealth: b(v[7]),
            rush: b(v[8]),
            poison: b(v[9]),
            life: b(v[10]),
            wf: b(v[11]),
            frozen: b(v[12]),
            fresh: b(v[13]),
            dr: v[14],
            sp: v[15] as i32,
            elusive: b(v[16]),
            immune: b(v[17]),
            tmp: v[18] as i32,
            pv: v[19],
            reborn: b(v[20]),
            doomed: b(v[21]),
            swung: b(v[22]),
            shield: v[23] as i32,
            race: v.get(24).map_or(0, |r| *r as u32),
            adjb: 0,
        })
    }
}

/// A position during my turn: mana, hand (card ids), both boards, both heroes, my weapon, and counters
/// the rules read (`corpses`, `died`, `drawn`, `played`). `bonus` is the score the search adds for effects that do
/// not show on the board.
#[derive(Clone, Debug)]
pub struct S {
    pub mana: i32,
    pub hand: Vec<i32>,
    pub mine: Vec<U>,
    pub theirs: Vec<U>,
    pub my_hp: i32,
    pub my_armor: i32,
    pub en_hp: i32,
    pub hero_atk: i32,
    pub hero_att: i32,
    pub bonus: f64,
    pub watk: i32,
    pub wdur: i32,
    pub corpses: i32,
    pub died: i32,
    pub cannon: bool,
    pub drawn: i32,
    pub played: i32, // cards played this turn so far (Combo)
    pub wid: i32,    // equipped weapon code (0 = none): its "After your hero attacks" trigger
}

/// The target of an action or effect: none, my hero, the enemy hero (`Face`) or a unit by id.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum T {
    None,
    Hero,
    Face,
    Unit(i32),
}

/// One move: an attack, or playing a card (`uid`) at a target in mode `mi`.
#[derive(Clone, Debug)]
pub enum Act {
    Attack { src: T, tgt: T },
    Play { uid: i32, t: T, mi: usize },
}

/// `u` after taking `d` damage: a shield or immunity absorbs it, `None` means it died.
pub fn hit(u: &U, d: i32, poison: bool) -> Option<U> {
    if d <= 0 || u.immune {
        return Some(*u);
    }
    if u.divine {
        let mut n = *u;
        if u.shield > 0 {
            n.shield -= 1;
        } else {
            n.divine = false;
        }
        return Some(n);
    }
    if poison || u.hp - d <= 0 {
        return None;
    }
    let mut n = *u;
    n.hp = u.hp - d;
    Some(n)
}

/// `units` with the unit `uid` replaced by `new`, or removed when `new` is `None`.
pub fn swap(units: &[U], uid: i32, new: Option<U>) -> Vec<U> {
    let mut out = Vec::with_capacity(units.len());
    for u in units {
        if u.uid == uid {
            if let Some(n) = new {
                out.push(n);
            }
        } else {
            out.push(*u);
        }
    }
    out
}

// ------------------------------------------------------------------ effects
/// One effect of a card rule: the op ([`OPS`]), its numbers (`n`, attack `a`, health `h`), keyword bits,
/// target scope, condition and area scope as codes. See docs/RULES_FORMAT.md.
#[derive(Clone, Copy, Debug)]
pub struct Fx {
    pub op: u8,
    pub n: i32,
    pub a: i32,
    pub h: i32,
    pub kw: u16,
    pub tgt: u8,  // see tgt_code
    pub cond: u8, // 0 "" | 1 atk>= | 2 atk<= | 3 damaged | 4 undamaged | 5 temp | 6 other | 7 race:BITS | 9 combo
    pub cond_n: i32,
    pub scope: u8, // 0 "" | 1 other_enemies | 2 enemy_minions | 3 enemies | 4 minions | 5 chars | 6 other_chars | 7 other_minions | 8 others | 9 other
}

/// The names of all effect ops; an op code is an index into this list.
pub const OPS: &[&str] = &[
    "dmg",
    "face",
    "self_dmg",
    "aoe",
    "self_hp_enemy_hand",
    "self_buff_allies",
    "dmg_random",
    "dmg_random_minions",
    "heal",
    "heal_hero",
    "heal_all",
    "buff",
    "buff_all",
    "enemy_atk_temp",
    "hand_buff",
    "opp_cards",
    "hero_atk",
    "armor",
    "weapon",
    "summon",
    "draw",
    "corpse",
    "self_hp_hand",
    "destroy_weapon",
    "summon_corpse",
    "dr_all",
    "buff_all_corpse",
    "draw_to",
    "summon_played",
    "mana",
    "ramp",
    "destroy",
    "destroy_highest",
    "weapon_buff",
    "self_buff_hand",
    "summon_copies",
    "gen",
    "value",
    "discard",
    "lifesteal_heal",
    "destroy_random",
    "destroy_all",
    "transform",
    "silence",
    "freeze",
    "freeze_all",
    "set_hp",
    "set_atk",
    "all_hp",
    "bounce",
    "transform_all",
    "doom_all",
    "bounce_all",
    "enemy_summon",
    "mana_marker",
    "self_atk",
    "discover",
    "steal_temp",
    "self_stats",
    "drain_lowest",
    "ping",
    "heal_damaged",
    "attack_random",
    "grow_if_full",
    "eat_right",
    "summon_adj",
    "bitter",
    "aoe_ls",
    "buff_name",
    "halve_enemy",
    "mask_fill",
    "dmg_same_race",
    "draw_brk",
    "draw_kill",
    "dmg_surv",
    "aoe_draw",
    "shuffle_self",
    "dmg_chain",
    "dmg_split",
    "dmg_excess",
];

impl Fx {
    /// An effect from its rule-file fields: op name, n, a, h, keyword bits, target, condition, scope.
    #[allow(clippy::too_many_arguments)]
    pub fn parse(
        op: &str,
        n: i32,
        a: i32,
        h: i32,
        kw: u16,
        tgt: &str,
        cond: &str,
        scope: &str,
    ) -> Fx {
        let (c, cond_n) = cond_code(cond);
        Fx {
            op: op_code(op),
            n,
            a,
            h,
            kw,
            tgt: tgt_code(tgt),
            cond: c,
            cond_n,
            scope: scope_code(scope),
        }
    }

    /// Shorthand for an effect with only an op and numbers.
    pub fn op(op: &str, n: i32, a: i32, h: i32) -> Fx {
        Fx::parse(op, n, a, h, 0, "", "", "")
    }
}

/// The code of an op name; 255 for an unknown op (the effect then does nothing).
pub fn op_code(s: &str) -> u8 {
    OPS.iter()
        .position(|x| *x == s)
        .map(|i| i as u8)
        .unwrap_or(255)
}

/// Whether `f` is the op `name`.
pub fn op_is(f: &Fx, name: &str) -> bool {
    f.op != 255 && OPS[f.op as usize] == name
}

/// The target scope code of a rule's `tgt` string; unknown scopes are 6 ("other").
pub fn tgt_code(s: &str) -> u8 {
    match s {
        "" => 0,
        "enemy_char" => 1,
        "enemy_minion" => 2,
        "friendly_char" => 3,
        "friendly_minion" => 4,
        "same" => 5,
        "any_minion" => 7,
        "any_char" => 8,
        _ => 6,
    }
}

/// The condition code and its number for a rule's `cond` string (for example `atk>=5`).
pub fn cond_code(s: &str) -> (u8, i32) {
    if let Some(r) = s.strip_prefix("atk>=") {
        return (1, r.parse().unwrap_or(0));
    }
    if let Some(r) = s.strip_prefix("atk<=") {
        return (2, r.parse().unwrap_or(0));
    }
    if let Some(r) = s.strip_prefix("race:") {
        return (7, r.parse().unwrap_or(0));
    }
    match s {
        "" => (0, 0),
        "damaged" => (3, 0),
        "undamaged" => (4, 0),
        "temp" => (5, 0),
        "combo" => (9, 0),
        _ => (6, 0),
    }
}

/// The area scope code of a rule's `scope` string.
pub fn scope_code(s: &str) -> u8 {
    match s {
        "" => 0,
        "other_enemies" => 1,
        "enemy_minions" => 2,
        "enemies" => 3,
        "minions" => 4,
        "chars" => 5,
        "other_chars" => 6,
        "other_minions" => 7,
        "others" => 8,
        _ => 9,
    }
}

/// The compiled model of one card: kind, cost, stats, keywords, spell damage, the effect list of each
/// play mode (with a flag for modes that hide unknown effects), deathrattle, aura, triggers and the
/// evaluation's extra values.
#[derive(Clone, Debug)]
pub struct Info {
    pub name: i32,
    pub kind: u8,
    pub cost: i32,
    pub atk: i32,
    pub hp: i32,
    pub kw: u16,
    pub sp: i32,
    pub modes: Vec<(Vec<Fx>, bool)>,
    pub dr: f64,
    pub aura: Option<(i32, i32)>,
    pub overload: i32,
    pub drs: Option<(i32, i32, i32, u16)>,
    /// number of the card model the summoned Deathrattle minion takes (0 for a plain token)
    pub dr_tok: i32,
    pub eot: Vec<Fx>,
    pub pv: f64,
    pub cannon: bool,
    pub trig: Vec<(u8, Vec<Fx>, u8)>, // (event, effects, filter): 1 hero_attack 2 spell 3 minion_play 4 power 5 draw 6 self_attack
    pub wid: i32,
    pub adj: i32,      // "adjacent minions have +adj Attack"
    pub opp_draw: i32, // deathrattle: the opponent (me, when it is their minion) draws this many cards
    pub race: u32,     // minion type bits of the card (see `U::race`)
    pub dr_gen: i32,   // deathrattle: cards (a Coin) put in my hand when it dies
}

impl Info {
    /// A card model with no effects.
    pub fn new(name: i32, kind: u8, cost: i32, atk: i32, hp: i32) -> Info {
        Info {
            name,
            kind,
            cost,
            atk,
            hp,
            kw: 0,
            sp: 0,
            modes: Vec::new(),
            dr: 0.0,
            aura: None,
            overload: 0,
            drs: None,
            dr_tok: 0,
            eot: Vec::new(),
            pv: 0.0,
            cannon: false,
            trig: Vec::new(),
            wid: 0,
            adj: 0,
            opp_draw: 0,
            race: 0,
            dr_gen: 0,
        }
    }
}

/// A freshly summoned token with keyword bits `kw`.
pub fn token(uid: i32, a: i32, h: i32, kw: u16) -> U {
    let mut t = U::new(uid, a, h, h);
    t.taunt = kw & K_TAUNT != 0;
    t.divine = kw & K_DIVINE != 0;
    t.stealth = kw & K_STEALTH != 0;
    t.poison = kw & K_POISON != 0;
    t.life = kw & K_LIFE != 0;
    t.wf = kw & K_WF != 0;
    t.fresh = true;
    t
}

/// Integer division rounded up, as Python's `-(-a // b)`.
pub fn ceil_div(a: i32, b: i32) -> i32 {
    -((-a).div_euclid(b)
        + if (-a).rem_euclid(b) != 0 && b < 0 {
            1
        } else {
            0
        })
}
