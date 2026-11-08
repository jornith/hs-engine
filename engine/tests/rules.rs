//! Rule tests on hand-built positions: damage, deaths, triggers, auras, single ops and the turn search.
//! Every card here is synthetic; no card data is needed.
use hs_engine::planner::round3;
use hs_engine::*;

fn pos(mine: Vec<U>, theirs: Vec<U>) -> S {
    S {
        mana: 10,
        hand: vec![],
        mine,
        theirs,
        my_hp: 30,
        my_armor: 0,
        en_hp: 30,
        hero_atk: 0,
        hero_att: 0,
        bonus: 0.0,
        watk: 0,
        wdur: 0,
        corpses: 0,
        died: 0,
        cannon: false,
        drawn: 0,
        played: 0,
        wid: 0,
    }
}

fn unit(uid: i32, a: i32, h: i32) -> U {
    U::new(uid, a, h, h)
}

fn ready(uid: i32, a: i32, h: i32) -> U {
    let mut u = unit(uid, a, h);
    u.att = 1;
    u
}

fn planner(cards: Vec<(i32, Info)>) -> Planner {
    Planner::new(Wt::default(), cards)
}

fn minion(cost: i32, a: i32, h: i32) -> Info {
    Info::new(1, MINION, cost, a, h)
}

fn find(units: &[U], uid: i32) -> Option<U> {
    units.iter().find(|u| u.uid == uid).copied()
}

// ------------------------------------------------------------------ damage

#[test]
fn hit_respects_shields_poison_and_immunity() {
    let mut u = unit(1, 2, 5);
    assert_eq!(hit(&u, 3, false).unwrap().hp, 2);
    assert!(hit(&u, 5, false).is_none());
    assert!(hit(&u, 1, true).is_none(), "poison kills");
    assert_eq!(hit(&u, 0, true), Some(u), "no damage, no poison");
    u.divine = true;
    let shielded = hit(&u, 9, true).unwrap();
    assert!(!shielded.divine && shielded.hp == 5);
    u.shield = 1;
    let layered = hit(&u, 9, false).unwrap();
    assert!(layered.divine && layered.shield == 0);
    u.divine = false;
    u.immune = true;
    assert_eq!(hit(&u, 9, true), Some(u));
}

#[test]
fn spell_damage_adds_to_spells_only() {
    let mut caster = unit(1, 1, 1);
    caster.sp = 1;
    let mut p = planner(vec![]);
    let s = pos(vec![caster], vec![unit(50, 4, 4)]);
    let f = Fx::op("dmg", 2, 0, 0);
    let by_spell = p.apply(s.clone(), &f, T::Unit(50), SPELL, None);
    assert_eq!(find(&by_spell.theirs, 50).unwrap().hp, 1);
    let by_minion = p.apply(s, &f, T::Unit(50), MINION, None);
    assert_eq!(find(&by_minion.theirs, 50).unwrap().hp, 2);
}

#[test]
fn face_damage_and_armor() {
    let mut p = planner(vec![]);
    let s = p.apply(
        pos(vec![], vec![]),
        &Fx::op("dmg", 3, 0, 0),
        T::Face,
        SPELL,
        None,
    );
    assert_eq!(s.en_hp, 27);
    let mut s = pos(vec![], vec![]);
    s.my_armor = 2;
    let s = p.hurt_me(s, 5);
    assert_eq!((s.my_armor, s.my_hp), (0, 27));
}

#[test]
fn area_damage_scopes() {
    let mut p = planner(vec![]);
    let s = pos(vec![unit(1, 1, 3)], vec![unit(50, 1, 3), unit(51, 1, 1)]);
    let enemies = Fx::parse("aoe", 1, 0, 0, 0, "", "", "enemies");
    let e = p.apply(s.clone(), &enemies, T::None, SPELL, None);
    assert_eq!(e.theirs.len(), 1, "the 1/1 died");
    assert_eq!(find(&e.theirs, 50).unwrap().hp, 2);
    assert_eq!(find(&e.mine, 1).unwrap().hp, 3, "my side untouched");
    assert_eq!((e.en_hp, e.my_hp), (29, 30));
    let all = Fx::parse("aoe", 1, 0, 0, 0, "", "", "chars");
    let a = p.apply(s, &all, T::None, SPELL, None);
    assert_eq!(find(&a.mine, 1).unwrap().hp, 2);
    assert_eq!((a.en_hp, a.my_hp), (29, 29));
}

// ------------------------------------------------------------------ deaths

#[test]
fn deathrattle_summons_for_its_owner() {
    let mut dr = minion(3, 2, 2);
    dr.drs = Some((2, 1, 1, 0));
    let mut p = planner(vec![(50, dr)]);
    let s = pos(vec![], vec![unit(50, 2, 2)]);
    let s = p.apply(s, &Fx::op("dmg", 2, 0, 0), T::Unit(50), SPELL, None);
    assert_eq!(s.theirs.len(), 2);
    assert!(s.theirs.iter().all(|u| (u.atk, u.hp) == (1, 1)));
    assert_eq!(s.died, 1);
}

#[test]
fn reborn_returns_with_one_health() {
    let mut p = planner(vec![]);
    let mut u = unit(1, 3, 3);
    u.reborn = true;
    let s = p.dmg_my_unit(pos(vec![u], vec![]), 1, 5);
    assert_eq!(s.mine.len(), 1);
    let back = s.mine[0];
    assert_ne!(back.uid, 1);
    assert_eq!((back.atk, back.hp, back.reborn), (3, 1, false));
    assert_eq!(p.origin.get(&back.uid), Some(&1));
}

#[test]
fn summons_stop_at_seven_minions() {
    let mut p = planner(vec![]);
    let s = pos(vec![unit(1, 1, 1)], vec![]);
    let s = p.apply(s, &Fx::op("summon", 10, 1, 1), T::None, SPELL, None);
    assert_eq!(s.mine.len(), 7);
}

#[test]
fn doomed_minions_die_at_end_of_turn() {
    let mut p = planner(vec![]);
    let mut u = unit(1, 1, 1);
    u.doomed = true;
    let s = p.end_of_turn(pos(vec![u, unit(2, 1, 1)], vec![]));
    assert_eq!(s.mine.len(), 1);
    assert_eq!(s.mine[0].uid, 2);
}

// ------------------------------------------------------------------ triggers

#[test]
fn friendly_death_trigger_fires() {
    let mut watcher = minion(2, 1, 4);
    // "Whenever a friendly minion dies, deal 1 damage to the enemy hero"
    watcher.trig = vec![(9, vec![Fx::op("face", 1, 0, 0)], 0)];
    let mut p = planner(vec![(1, watcher)]);
    let s = pos(vec![unit(1, 1, 4), unit(2, 1, 1)], vec![]);
    let s = p.dmg_my_unit(s, 2, 1);
    assert_eq!(s.en_hp, 29);
    let s = p.dmg_my_unit(s, 1, 1);
    assert_eq!(s.en_hp, 29, "damage that does not kill fires nothing");
}

#[test]
fn end_of_turn_effects_fire() {
    let mut eot = minion(2, 1, 1);
    eot.eot = vec![Fx::op("face", 2, 0, 0)];
    let mut p = planner(vec![(1, eot)]);
    let s = p.end_of_turn(pos(vec![unit(1, 1, 1)], vec![]));
    assert_eq!(s.en_hp, 28);
}

// ------------------------------------------------------------------ auras

#[test]
fn aura_applies_on_play_and_ends_on_death() {
    let mut lord = minion(2, 2, 2);
    lord.aura = Some((1, 1));
    let mut p = planner(vec![(30, lord.clone())]);
    let mut s = pos(vec![unit(1, 2, 2)], vec![]);
    s.hand = vec![30];
    let after = p
        .moves(&s)
        .into_iter()
        .find(|(a, _)| matches!(a, Act::Play { uid: 30, .. }))
        .map(|(_, ns)| ns)
        .expect("the card is playable");
    assert_eq!(after.mana, 8);
    let buffed = find(&after.mine, 1).unwrap();
    assert_eq!((buffed.atk, buffed.hp), (3, 3));
    let lord_on_board = find(&after.mine, 30).unwrap();
    assert_eq!((lord_on_board.atk, lord_on_board.hp), (2, 2));
    let died = p.dmg_my_unit(after, 30, 2);
    let back = find(&died.mine, 1).unwrap();
    assert_eq!((back.atk, back.hp), (2, 2));
}

#[test]
fn adjacency_aura_follows_the_board() {
    let mut flank = minion(1, 1, 1);
    flank.adj = 1;
    let mut p = planner(vec![(1, flank)]);
    // unit 2 shows its base 2 attack plus the aura of its neighbour 1
    let mut s = pos(vec![ready(2, 3, 3), ready(1, 1, 1)], vec![unit(50, 3, 3)]);
    p.norm_adj(&mut s);
    assert_eq!(find(&s.mine, 2).unwrap().atk, 3, "bookkeeping only");
    let (_, ns) = p
        .moves(&s)
        .into_iter()
        .find(|(a, _)| {
            matches!(
                a,
                Act::Attack {
                    src: T::Unit(1),
                    tgt: T::Unit(50)
                }
            )
        })
        .unwrap();
    assert!(
        find(&ns.mine, 1).is_none(),
        "the 1/1 died attacking the 3/3"
    );
    assert_eq!(
        find(&ns.mine, 2).unwrap().atk,
        2,
        "aura gone with its source"
    );
}

// ------------------------------------------------------------------ ops

#[test]
fn unknown_op_does_nothing() {
    let f = Fx::op("no_such_op", 5, 0, 0);
    assert_eq!(f.op, 255);
    let mut p = planner(vec![]);
    let s = pos(vec![unit(1, 1, 1)], vec![]);
    let out = p.apply(s.clone(), &f, T::Unit(1), SPELL, None);
    assert_eq!(p.key(&out), p.key(&s));
}

#[test]
fn buff_and_heal() {
    let mut p = planner(vec![]);
    let mut hurt = unit(1, 2, 2);
    hurt.maxhp = 5;
    let mut s = pos(vec![hurt], vec![]);
    s.my_hp = 25;
    let b = p.apply(s.clone(), &Fx::op("buff", 0, 2, 2), T::Unit(1), SPELL, None);
    let u = find(&b.mine, 1).unwrap();
    assert_eq!((u.atk, u.hp, u.maxhp), (4, 4, 7));
    let h = p.apply(s.clone(), &Fx::op("heal", 9, 0, 0), T::Unit(1), SPELL, None);
    assert_eq!(
        find(&h.mine, 1).unwrap().hp,
        5,
        "healing stops at max health"
    );
    assert_eq!(p.heal_me(s, 9).my_hp, 30);
}

#[test]
fn weapon_loses_durability_on_swing() {
    let mut s = pos(vec![], vec![]);
    s.watk = 3;
    s.hero_atk = 3;
    s.wdur = 2;
    s.hero_att = 1;
    let s = Planner::swing(&s);
    assert_eq!((s.wdur, s.hero_att, s.hero_atk), (1, 0, 3));
    let s = Planner::swing(&s);
    assert_eq!((s.wdur, s.watk, s.hero_atk), (0, 0, 0), "the weapon broke");
}

#[test]
fn rule_codes() {
    assert_ne!(op_code("dmg"), 255);
    assert!(op_is(&Fx::op("summon", 1, 1, 1), "summon"));
    assert_eq!(tgt_code("enemy_minion"), 2);
    assert_eq!(tgt_code("whatever"), 6);
    assert_eq!(scope_code("enemies"), 3);
    assert_eq!(cond_code("atk>=5"), (1, 5));
    assert_eq!(cond_code("race:4"), (7, 4));
    assert_eq!(kind_code("WEAPON"), WEAPON);
    assert_eq!(
        (ceil_div(7, 2), ceil_div(-7, 2), ceil_div(6, 3)),
        (4, -3, 2)
    );
    assert_eq!((round3(0.1234), round3(-2.0)), (123, -2000));
}

fn typed(uid: i32, a: i32, h: i32, race: u32) -> U {
    let mut u = unit(uid, a, h);
    u.race = race;
    u
}

#[test]
fn unit_rows_take_24_or_25_fields() {
    let mut row = vec![0.0; 24];
    row[0] = 7.0;
    row[1] = 2.0;
    row[2] = 3.0;
    row[3] = 3.0;
    assert_eq!(U::from_vec(&row).unwrap().race, 0);
    row.push(5.0);
    let u = U::from_vec(&row).unwrap();
    assert_eq!((u.uid, u.race), (7, 5));
    assert_eq!(
        u.stats().last(),
        Some(&5.0),
        "the row ends with the type bits"
    );
    assert!(U::from_vec(&row[..23]).is_err());
    assert_ne!(typed(1, 2, 2, 1).sig(), typed(1, 2, 2, 2).sig());
}

#[test]
fn buff_with_a_type_condition_skips_other_types() {
    let p = planner(vec![]);
    let f = Fx::parse("buff_all", 0, 1, 1, 0, "", "race:2", "");
    assert_eq!(f.cond, 7);
    let hit = p.buff_unit(&typed(1, 2, 2, 2 | 8), &f);
    assert_eq!((hit.atk, hit.hp), (3, 3));
    for other in [typed(2, 2, 2, 4), unit(3, 2, 2)] {
        assert_eq!(p.buff_unit(&other, &f), other);
    }
}

#[test]
fn same_type_damage_hits_every_minion_of_the_type() {
    let mut p = planner(vec![]);
    let s = pos(
        vec![typed(1, 2, 3, 1), typed(2, 2, 3, 2)],
        vec![
            typed(50, 2, 5, 1),
            typed(51, 2, 3, 1 | 2),
            typed(52, 2, 3, 4),
        ],
    );
    let f = Fx::parse("dmg_same_race", 3, 0, 0, 0, "any_minion", "", "");
    let r = p.apply(s.clone(), &f, T::Unit(50), SPELL, None);
    assert!(find(&r.mine, 1).is_none(), "my minion of the type died");
    assert_eq!(find(&r.mine, 2).unwrap().hp, 3);
    assert_eq!(find(&r.theirs, 50).unwrap().hp, 2);
    assert!(
        find(&r.theirs, 51).is_none(),
        "a minion sharing one of two types died"
    );
    assert_eq!(find(&r.theirs, 52).unwrap().hp, 3);
    let untyped = pos(vec![unit(1, 2, 3)], vec![unit(50, 2, 5), unit(51, 2, 3)]);
    let r = p.apply(untyped, &f, T::Unit(50), SPELL, None);
    assert_eq!(
        find(&r.theirs, 50).unwrap().hp,
        2,
        "the target is always hit"
    );
    assert_eq!(
        find(&r.theirs, 51).unwrap().hp,
        3,
        "untyped minions share no type"
    );
    let r = p.apply(s, &f, T::None, SPELL, None);
    assert_eq!(find(&r.theirs, 50).unwrap().hp, 5, "no target, no damage");
}

#[test]
fn halve_enemy_rounds_down() {
    let mut p = planner(vec![]);
    let mut s = pos(vec![], vec![]);
    s.en_hp = 25;
    let r = p.apply(s, &Fx::op("halve_enemy", 0, 0, 0), T::None, SPELL, None);
    assert_eq!(r.en_hp, 12);
}

#[test]
fn mask_fill_makes_one_minion_1_1_and_fills_the_board() {
    let mut p = planner(vec![]);
    let s = pos(vec![unit(1, 5, 5), unit(2, 2, 3), unit(3, 4, 4)], vec![]);
    let r = p.apply(
        s.clone(),
        &Fx::op("mask_fill", 1, 0, 0),
        T::None,
        SPELL,
        None,
    );
    assert_eq!(r.mine.len(), 7);
    assert_eq!(
        find(&r.mine, 2).map(|u| (u.atk, u.hp, u.maxhp)),
        Some((1, 1, 1))
    );
    assert_eq!(
        find(&r.mine, 1).map(|u| u.atk),
        Some(5),
        "only the weakest changes"
    );
    assert!(
        r.mine
            .iter()
            .filter(|u| u.uid < 0)
            .all(|u| (u.atk, u.hp) == (1, 1))
    );
    let empty = pos(vec![], vec![]);
    let r = p.apply(empty, &Fx::op("mask_fill", 1, 0, 0), T::None, SPELL, None);
    assert!(
        r.mine.is_empty(),
        "without a minion to change nothing is summoned"
    );
}

#[test]
fn a_played_minion_enters_with_its_type() {
    let mut info = minion(1, 2, 2);
    info.race = 8;
    let mut p = planner(vec![(100, info)]);
    let mut s = pos(vec![], vec![]);
    s.hand = vec![100];
    let moves = p.moves(&s);
    assert!(!moves.is_empty());
    for (_, ns) in &moves {
        assert_eq!(find(&ns.mine, 100).unwrap().race, 8);
    }
}

// ------------------------------------------------------------------ moves and search

#[test]
fn taunt_blocks_face_attacks() {
    let mut p = planner(vec![]);
    let mut guard = unit(50, 1, 5);
    guard.taunt = true;
    let s = pos(vec![ready(1, 3, 3)], vec![guard, unit(51, 1, 1)]);
    let moves = p.moves(&s);
    assert!(!moves.is_empty());
    for (a, _) in &moves {
        assert!(
            matches!(
                a,
                Act::Attack {
                    tgt: T::Unit(50),
                    ..
                }
            ),
            "only the Taunt minion can be attacked, got {a:?}"
        );
    }
}

#[test]
fn search_finds_lethal() {
    let mut p = planner(vec![]);
    let mut s = pos(vec![ready(1, 2, 2), ready(2, 2, 2)], vec![unit(50, 1, 8)]);
    s.en_hp = 4;
    let r = run_search(&mut p, &s, 5.0, 6, 10_000);
    assert!(r.lethal);
    assert_eq!(r.path.len(), 2);
    assert!(r.last.en_hp <= 0);
    let replayed = replay(&mut p, &s, &r.path).unwrap();
    assert!(replayed.last().unwrap().1.en_hp <= 0);
}

// ------------------------------------------------------------------ card-drawing ops

#[test]
fn draw_on_weapon_break_needs_the_break() {
    let mut p = planner(vec![]);
    p.w.draw = 1.0;
    let f = Fx::op("draw_brk", 1, 0, 0);
    let mut s = pos(vec![], vec![]);
    s.watk = 2;
    s.hero_atk = 2;
    s.wdur = 2;
    s.hero_att = 1;
    let kept = p.apply(Planner::swing(&s), &f, T::None, WEAPON, None);
    assert_eq!(kept.drawn, 0, "the weapon survived the swing");
    s.wdur = 1;
    let broke = p.apply(Planner::swing(&s), &f, T::None, WEAPON, None);
    assert_eq!(broke.drawn, 1);
    assert!(broke.bonus > 0.0);
}

#[test]
fn draw_on_kill_needs_a_kill() {
    let mut p = planner(vec![]);
    let f = Fx::op("draw_kill", 1, 0, 0);
    let s = pos(vec![], vec![]);
    assert_eq!(p.apply(s.clone(), &f, T::None, WEAPON, None).drawn, 0);
    p.hero_killed = true;
    assert_eq!(p.apply(s.clone(), &f, T::None, WEAPON, None).drawn, 1);
    let mut full = s;
    full.drawn = 10;
    assert_eq!(
        p.apply(full, &f, T::None, WEAPON, None).drawn,
        10,
        "a full hand draws nothing"
    );
}

#[test]
fn hero_swing_that_kills_sets_the_kill_flag_for_the_weapon() {
    let mut weapon = Info::new(2, WEAPON, 3, 2, 2);
    weapon.wid = 1;
    let mut p = planner(vec![(60, weapon)]);
    p.wtrig.insert(1, vec![Fx::op("draw_kill", 1, 0, 0)]);
    let mut s = pos(vec![], vec![unit(50, 1, 2), unit(51, 1, 9)]);
    s.watk = 2;
    s.hero_atk = 2;
    s.wdur = 2;
    s.hero_att = 1;
    s.wid = 1;
    let moves = p.moves(&s);
    let drawn = |tgt: i32| {
        moves
            .iter()
            .find(|(a, _)| matches!(a, Act::Attack { src: T::Hero, tgt: T::Unit(u) } if *u == tgt))
            .map(|(_, r)| r.drawn)
    };
    assert_eq!(drawn(50), Some(1), "the kill draws");
    assert_eq!(drawn(51), Some(0), "a hit that leaves it alive does not");
}

#[test]
fn damage_then_draw_or_summon() {
    let mut p = planner(vec![]);
    let f = Fx::parse("dmg_surv", 1, 2, 2, 0, "any_minion", "", "");
    let s = pos(vec![], vec![unit(50, 1, 3), unit(51, 1, 1)]);
    let alive = p.apply(s.clone(), &f, T::Unit(50), SPELL, None);
    assert_eq!((alive.drawn, alive.mine.len()), (1, 0));
    assert_eq!(find(&alive.theirs, 50).unwrap().hp, 2);
    let dead = p.apply(s.clone(), &f, T::Unit(51), SPELL, None);
    assert_eq!(dead.drawn, 0);
    assert_eq!(dead.mine.len(), 1, "a minion for me");
    assert_eq!((dead.mine[0].atk, dead.mine[0].hp), (2, 2));
    let none = p.apply(s, &f, T::None, SPELL, None);
    assert_eq!((none.drawn, none.mine.len(), none.theirs.len()), (0, 0, 2));
}

#[test]
fn area_damage_draws_for_each_death() {
    let mut p = planner(vec![]);
    let f = Fx::op("aoe_draw", 1, 0, 0);
    let s = pos(
        vec![unit(1, 1, 1), unit(2, 1, 4)],
        vec![unit(50, 1, 1), unit(51, 1, 1), unit(52, 1, 4)],
    );
    let r = p.apply(s, &f, T::None, SPELL, None);
    assert_eq!(r.drawn, 3, "one card per minion that died, on both sides");
    assert_eq!((r.mine.len(), r.theirs.len()), (1, 1));
}

#[test]
fn chain_damage_continues_until_a_minion_survives() {
    let mut p = planner(vec![]);
    let f = Fx::parse("dmg_chain", 2, 0, 0, 0, "enemy_minion", "", "");
    let s = pos(vec![], vec![unit(50, 1, 2), unit(51, 1, 1), unit(52, 1, 5)]);
    let r = p.apply(s.clone(), &f, T::Unit(50), SPELL, None);
    let hp: Vec<i32> = r.theirs.iter().map(|u| u.hp).collect();
    assert_eq!(
        hp,
        vec![3],
        "50 and 51 died, 52 took the last hit and lives"
    );
    let lone = pos(vec![], vec![unit(50, 1, 2)]);
    assert!(
        p.apply(lone, &f, T::Unit(50), SPELL, None)
            .theirs
            .is_empty()
    );
    let none = p.apply(s, &f, T::None, SPELL, None);
    assert_eq!(none.theirs.len(), 3);
}

#[test]
fn split_damage_goes_one_point_at_a_time_and_heals() {
    let mut p = planner(vec![]);
    let f = Fx::parse("dmg_split", 4, 1, 0, 0, "", "", "");
    let mut hurt = pos(vec![], vec![unit(50, 1, 1), unit(51, 1, 2), unit(52, 1, 9)]);
    hurt.my_hp = 20;
    let r = p.apply(hurt, &f, T::None, SPELL, None);
    assert_eq!(
        r.theirs.len(),
        1,
        "1 + 2 points kill two minions, the last point hits the third"
    );
    assert_eq!(r.theirs[0].hp, 8);
    assert_eq!(r.my_hp, 24, "Lifesteal heals every point");
    let empty = pos(vec![], vec![]);
    assert_eq!(p.apply(empty, &f, T::None, SPELL, None).my_hp, 30);
    let plain = Fx::parse("dmg_split", 2, 0, 0, 0, "", "", "");
    let mut s = pos(vec![], vec![unit(50, 1, 9)]);
    s.my_hp = 20;
    assert_eq!(p.apply(s, &plain, T::None, SPELL, None).my_hp, 20);
}

#[test]
fn shuffle_self_removes_the_acting_minion() {
    let mut p = planner(vec![]);
    let f = Fx::op("shuffle_self", 0, 0, 0);
    let s = pos(vec![unit(1, 1, 1), unit(2, 1, 1)], vec![]);
    let r = p.apply(s.clone(), &f, T::None, MINION, Some(2));
    assert_eq!(r.mine.iter().map(|u| u.uid).collect::<Vec<_>>(), vec![1]);
    assert_eq!(p.apply(s, &f, T::None, MINION, None).mine.len(), 2);
}

#[test]
fn deathrattle_cards_go_to_my_hand_only_for_my_minions() {
    let mut coin = minion(2, 1, 1);
    coin.dr_gen = 1;
    let mut p = planner(vec![(1, coin.clone()), (50, coin)]);
    let kill = Fx::op("dmg", 5, 0, 0);
    let s = pos(vec![unit(1, 1, 1)], vec![unit(50, 1, 1)]);
    let mine = p.apply(s.clone(), &kill, T::Unit(1), SPELL, None);
    assert_eq!(mine.drawn, 1);
    let theirs = p.apply(s.clone(), &kill, T::Unit(50), SPELL, None);
    assert_eq!(theirs.drawn, 0);
    p.silenced.insert(1);
    assert_eq!(p.apply(s, &kill, T::Unit(1), SPELL, None).drawn, 0);
}

#[test]
fn elusive_minions_are_battlecry_targets_only() {
    let mut elusive = unit(50, 1, 3);
    elusive.elusive = true;
    let s = pos(vec![], vec![elusive]);
    let dmg = Fx::parse("dmg", 1, 0, 0, 0, "enemy_minion", "", "");
    let mut battlecry = minion(1, 1, 1);
    battlecry.modes = vec![(vec![dmg], false)];
    let mut spell = Info::new(3, SPELL, 1, 0, 0);
    spell.modes = vec![(vec![dmg], false)];
    let mut p = planner(vec![(60, battlecry), (61, spell)]);
    assert_eq!(p.cast(&s, 0, None).len(), 1, "the Battlecry reaches it");
    assert_eq!(p.cast(&s, 1, None).len(), 0, "a spell does not");
}

// ------------------------------------------------------------------ excess damage, combo, move keys

#[test]
fn excess_damage_overflows_to_the_hero() {
    let mut p = planner(vec![]);
    let f = Fx::parse("dmg_excess", 5, 1, 0, 0, "", "", "");
    let s = pos(vec![], vec![unit(50, 1, 2), unit(51, 1, 9)]);
    let r = p.apply(s, &f, T::None, SPELL, None);
    assert_eq!(r.theirs.len(), 1, "the lowest-health minion died");
    assert_eq!(r.en_hp, 27, "5 damage on 2 health: 3 go to the hero");
    let hit_alive = pos(vec![], vec![unit(50, 1, 9)]);
    let r = p.apply(hit_alive, &f, T::None, SPELL, None);
    assert_eq!(
        (r.en_hp, r.theirs[0].hp),
        (30, 4),
        "no overflow without a kill"
    );
    let empty = p.apply(pos(vec![], vec![]), &f, T::None, SPELL, None);
    assert_eq!(empty.en_hp, 25, "no minion: the whole hit goes to the hero");
}

#[test]
fn excess_damage_policy_and_repeat() {
    let mut p = planner(vec![]);
    let board = vec![unit(50, 1, 2), unit(51, 1, 9)];
    let high = Fx::parse("dmg_excess", 4, 1, 1, 0, "", "", "");
    let r = p.apply(pos(vec![], board.clone()), &high, T::None, SPELL, None);
    assert_eq!(
        find(&r.theirs, 51).unwrap().hp,
        5,
        "policy 1 hits the highest health"
    );
    let no_face = Fx::parse("dmg_excess", 5, 1, 10, 0, "", "", "");
    let r = p.apply(pos(vec![], board.clone()), &no_face, T::None, SPELL, None);
    assert_eq!(
        (r.theirs.len(), r.en_hp),
        (1, 30),
        "+10: no overflow to the hero"
    );
    let none = p.apply(pos(vec![], vec![]), &no_face, T::None, SPELL, None);
    assert_eq!(none.en_hp, 30, "+10: no minion, no effect");
    let twice = Fx::parse("dmg_excess", 3, 2, 0, 0, "", "", "");
    let r = p.apply(pos(vec![], board), &twice, T::None, SPELL, None);
    assert_eq!(
        (r.theirs.len(), r.en_hp),
        (1, 29),
        "two hits: kill with 1 over, then 3 on the 9"
    );
    assert_eq!(r.theirs[0].hp, 6);
}

#[test]
fn combo_effects_need_an_earlier_card() {
    let combo = Fx::parse("face", 5, 0, 0, 0, "", "combo", "");
    let mut spell = Info::new(3, SPELL, 1, 0, 0);
    spell.modes = vec![(vec![combo], false)];
    let mut p = planner(vec![(60, spell)]);
    let first = pos(vec![], vec![]);
    assert_eq!(
        p.cast(&first, 0, None)[0].0.en_hp,
        30,
        "first card: no effect"
    );
    let mut later = pos(vec![], vec![]);
    later.played = 1;
    assert_eq!(p.cast(&later, 0, None)[0].0.en_hp, 25);
}

#[test]
fn played_counter_counts_hand_cards_only() {
    let mut spell = Info::new(3, SPELL, 1, 0, 0);
    spell.modes = vec![(vec![Fx::op("face", 1, 0, 0)], false)];
    let mut power = Info::new(4, POWER, 2, 0, 0);
    power.modes = vec![(vec![Fx::op("face", 1, 0, 0)], false)];
    let mut p = planner(vec![(60, spell), (61, power)]);
    let mut s = pos(vec![], vec![]);
    s.hand = vec![60, 61];
    let plays: Vec<(Act, S)> = p
        .moves_raw(&s)
        .into_iter()
        .filter(|(a, _)| matches!(a, Act::Play { .. }))
        .collect();
    for (a, ns) in &plays {
        match a {
            Act::Play { uid: 60, .. } => assert_eq!(ns.played, 1),
            Act::Play { uid: 61, .. } => assert_eq!(ns.played, 0, "hero power does not count"),
            _ => panic!("unexpected move"),
        }
    }
    assert_eq!(plays.len(), 2);
}

#[test]
fn cards_with_different_effects_are_different_moves() {
    let mk = |n: i32| {
        let mut c = Info::new(3, SPELL, 1, 0, 0);
        c.modes = vec![(vec![Fx::op("face", n, 0, 0)], false)];
        c
    };
    let count = |a: Info, b: Info| {
        let mut p = planner(vec![(60, a), (61, b)]);
        let mut s = pos(vec![], vec![]);
        s.hand = vec![60, 61];
        p.moves_raw(&s)
            .iter()
            .filter(|(a, _)| matches!(a, Act::Play { .. }))
            .count()
    };
    assert_eq!(count(mk(2), mk(2)), 1, "same effects: one move");
    assert_eq!(count(mk(2), mk(5)), 2, "same name and cost, other effects");
}

#[test]
fn destroying_my_own_minion_triggers_its_death() {
    let mut p = planner(vec![]);
    p.dk = true;
    let f = Fx::parse("destroy", 0, 0, 0, 0, "any_minion", "", "");
    let s = pos(vec![unit(1, 2, 2), unit(2, 1, 1)], vec![unit(50, 1, 1)]);
    let r = p.apply(s, &f, T::Unit(1), SPELL, None);
    assert_eq!(r.mine.iter().map(|u| u.uid).collect::<Vec<_>>(), vec![2]);
    assert_eq!(r.corpses, 1);
}

// ------------------------------------------------------------------ immune targets

#[test]
fn an_immune_enemy_minion_can_be_picked_as_a_target() {
    let mut p = planner(vec![]);
    let mut s = pos(vec![ready(1, 3, 3)], vec![unit(2, 1, 1)]);
    s.theirs[0].immune = true;
    let attacks_it = p.moves(&s).into_iter().any(|(a, _)| {
        matches!(
            a,
            Act::Attack {
                tgt: T::Unit(2),
                ..
            }
        )
    });
    assert!(attacks_it, "attacks may be aimed at an immune minion");
}
