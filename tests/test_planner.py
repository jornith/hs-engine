"""Turn-search scenarios on hand-built positions with synthetic cards (no card database needed).

The cards (tests/synth.py) have rules text in the card-database style; the rule parser turns that text into
effects, so these tests cover the parser, the engine and the search together. A test asserts on the line the
search picks or on the position after one move.
"""

import time

import pytest
from synth import DB, card, minion, power, state

from hsengine import check, rules
from hsengine.planner import FACE, HERO, Planner


def search(st):
    p = Planner(st, DB)
    score, path, final = p.search(0.3)
    return p, [a for a, _ in path], final


def lethal(final) -> bool:
    return final.en_hp <= 0


def plays(acts, uid):
    return [a for a in acts if a[0] == "play" and a[1] == uid]


def moves(st):
    p = Planner(st, DB)
    return p, p.moves(p.s0)


# ---------------------------------------------------------------- lethal


def test_lethal_through_board():
    spell = card("T_ARCANE")
    st = state(mana=1, enemy_hp=8, hand=[spell], board=[minion("T_BOAR", 2, 2) for _ in range(3)])
    _, acts, final = search(st)
    assert lethal(final), acts
    assert plays(acts, spell.eid) and plays(acts, spell.eid)[0][2] == FACE, acts


def test_lethal_needs_the_taunt_removed():
    taunt = minion("T_GUARD", 2, 2, kw=("taunt",), mine=False)
    st = state(
        mana=1,
        enemy_hp=4,
        hand=[card("T_ARCANE")],
        enemy_board=[taunt],
        board=[minion("T_BOAR", 2, 3) for _ in range(3)],
    )
    _, acts, final = search(st)
    assert lethal(final), acts
    assert any(a[2] == taunt.eid for a in acts), acts


def test_no_false_lethal_behind_taunt():
    taunt = minion("T_GUARD", 3, 5, kw=("taunt",), mine=False)
    st = state(mana=0, enemy_hp=4, enemy_board=[taunt], board=[minion("T_BOAR", 2, 3) for _ in range(2)])
    _, _, final = search(st)
    assert not lethal(final)


def test_charge_minion_lethal():
    rider = card("T_RIDER", kw=("charge",))
    _, acts, final = search(state(mana=3, enemy_hp=3, hand=[rider]))
    assert lethal(final) and plays(acts, rider.eid), acts


def test_windfury_for_lethal():
    _, acts, final = search(state(mana=2, enemy_hp=8, hand=[card("T_WINDS")], board=[minion("T_YETI", 4, 5)]))
    assert lethal(final), acts


def test_roar_with_hero_attack_for_lethal():
    st = state(mana=3, enemy_hp=10, hand=[card("T_ROAR")], board=[minion("T_BOAR", 2, 2), minion("T_BOAR", 2, 2)])
    _, acts, final = search(st)
    assert lethal(final), acts
    assert any(a[0] == "attack" and a[1] == HERO for a in acts), acts


def test_aura_minion_played_before_the_attacks():
    leader = card("T_LEADER")
    st = state(mana=3, enemy_hp=6, hand=[leader], board=[minion("T_BOAR", 2, 2), minion("T_BOAR", 2, 2)])
    _, acts, final = search(st)
    assert lethal(final), acts
    first_face = next(i for i, a in enumerate(acts) if a[0] == "attack" and a[2] == FACE)
    assert acts.index(plays(acts, leader.eid)[0]) < first_face, acts


def test_silence_then_damage_kills_a_shielded_taunt():
    shock = card("T_SHOCK")
    st = state(
        mana=1,
        enemy_hp=2,
        hand=[shock],
        board=[minion("T_BOAR", 2, 2)],
        enemy_board=[minion("T_GUARD", 1, 1, kw=("divine", "taunt"), mine=False)],
    )
    _, acts, final = search(st)
    assert lethal(final) and plays(acts, shock.eid), acts


def test_rush_minion_cannot_go_face_this_turn():
    st = state(mana=0, enemy_hp=3, board=[minion("T_RAPTOR", 3, 2, kw=("rush",))])
    st.board[0].fresh = True
    _, acts, final = search(st)
    assert not lethal(final)
    assert not any(a[2] == FACE for a in acts), acts


# ---------------------------------------------------------------- choices


def test_heal_goes_to_the_hurt_hero():
    heal = card("T_HEAL")
    _, acts, _ = search(state(mana=2, my_hp=10, hand=[heal]))
    assert plays(acts, heal.eid) and plays(acts, heal.eid)[0][2] == HERO, acts


def test_board_clear_is_played():
    holy = card("T_HOLY")
    enemies = [
        minion("T_RAPTOR", 3, 2, mine=False),
        minion("T_BOAR", 2, 2, mine=False),
        minion("T_BOAR", 2, 1, mine=False),
    ]
    _, acts, final = search(state(mana=3, hand=[holy], enemy_board=enemies))
    assert plays(acts, holy.eid), acts
    assert final.theirs == [] or len(final.theirs) == 0, final.theirs


def test_removal_on_the_big_threat():
    kill = card("T_KILL")
    ogre = minion("T_OGRE", 6, 7, mine=False)
    _, acts, _ = search(state(mana=3, hand=[kill], enemy_board=[ogre]))
    assert plays(acts, kill.eid) and plays(acts, kill.eid)[0][2] == ogre.eid, acts


def test_freeze_the_attacker_when_low():
    frost = card("T_FROST")
    ogre = minion("T_OGRE", 6, 7, mine=False)
    _, acts, _ = search(state(mana=2, my_hp=4, hand=[frost], enemy_board=[ogre]))
    assert plays(acts, frost.eid) and plays(acts, frost.eid)[0][2] == ogre.eid, acts


def test_overload_spell_still_kills_the_threat():
    bolt = card("T_BOLT")
    yeti = minion("T_YETI", 3, 3, mine=False)
    _, acts, final = search(state(mana=1, my_hp=12, hand=[bolt], enemy_board=[yeti]))
    assert plays(acts, bolt.eid) and plays(acts, bolt.eid)[0][2] == yeti.eid, acts
    assert final.theirs == [] or len(final.theirs) == 0


def test_poisonous_trades_up():
    ogre = minion("T_OGRE", 6, 7, mine=False)
    _, acts, final = search(state(mana=0, board=[minion("T_BOAR", 2, 3, kw=("poisonous",))], enemy_board=[ogre]))
    assert any(a[0] == "attack" and a[2] == ogre.eid for a in acts), acts
    assert len(final.theirs) == 0


def test_taunt_kills_what_would_not_attack_into_it():
    enemy = minion("T_RAPTOR", 2, 3, mine=False)
    st = state(mana=0, board=[minion("T_GUARD", 5, 5, kw=("taunt",))], enemy_board=[enemy])
    _, acts, _ = search(st)
    assert any(a[0] == "attack" and a[2] == enemy.eid for a in acts), acts


def test_weapon_then_hero_attack():
    axe = card("T_AXE")
    _, acts, _ = search(state(mana=2, hand=[axe], enemy_board=[minion("T_RAPTOR", 3, 2, mine=False)]))
    assert plays(acts, axe.eid), acts
    assert any(a[0] == "attack" and a[1] == HERO for a in acts), acts


def test_doomed_minion_trades_since_it_dies_anyway():
    whelp = minion("T_BOAR", 1, 1, mine=False)
    st = state(mana=0, board=[minion("T_GHOUL", 1, 1, kw=("charge",))], enemy_board=[whelp])
    _, acts, _ = search(st)
    assert any(a[0] == "attack" and a[2] == whelp.eid for a in acts), acts


def test_hero_powers():
    _, acts, final = search(state(mana=2, hero_power=power("T_BP_FACE")))
    assert final.en_hp == 28, acts
    _, acts, final = search(state(mana=2, hero_power=power("T_BP_ARMOR")))
    assert final.my_armor == 2, acts
    enemy = minion("T_RAPTOR", 3, 1, mine=False)
    _, acts, final = search(state(mana=2, hero_power=power("T_BP_PING"), enemy_board=[enemy]))
    assert any(a[0] == "play" and a[2] == enemy.eid for a in acts), acts


def test_stealthed_enemy_is_not_a_target():
    spy = minion("T_BOAR", 2, 1, kw=("stealth",), mine=False)
    p, ms = moves(state(mana=1, hand=[card("T_ARCANE")], enemy_board=[spy]))
    assert ms and not any(a[2] == spy.eid for a, _ in ms), ms


# ---------------------------------------------------------------- single moves and the enemy reply


def test_deathrattle_summon_is_on_the_board():
    golem = minion("T_GOLEM", 2, 3, kw=("deathrattle",), mine=False)
    _, ms = moves(state(mana=0, board=[minion("T_YETI", 4, 5)], enemy_board=[golem]))
    kill = next(ns for act, ns in ms if act[0] == "attack" and act[2] == golem.eid)
    assert [(u.atk, u.hp) for u in kill.theirs] == [(2, 1)], kill.theirs


def test_reborn_comes_back():
    _, ms = moves(
        state(
            mana=0, board=[minion("T_BOAR", 1, 1, kw=("reborn",))], enemy_board=[minion("T_RAPTOR", 2, 3, mine=False)]
        )
    )
    trade = next(ns for act, ns in ms if act[0] == "attack" and act[2] != FACE)
    assert [(u.atk, u.hp, u.reborn) for u in trade.mine] == [(1, 1, False)], trade.mine


def test_aura_reaches_later_minions_and_ends_with_its_source():
    boar = card("T_RAPTOR")
    leader = minion("T_LEADER", 2, 3)
    p, ms = moves(state(mana=3, hand=[boar], board=[leader]))
    after = next(ns for a, ns in ms if a[0] == "play")
    new = next(u for u in after.mine if u.uid != leader.eid)
    assert new.atk == 4, new
    from hsengine.rules import Fx

    gone = p._apply(after, Fx("dmg", 3, tgt="any_minion"), leader.eid, "SPELL")
    assert [u.atk for u in gone.mine] == [3], gone.mine


def test_full_hand_burns_draws():
    from hsengine.rules import Fx

    p = Planner(state(mana=0, hand=[card("T_BOAR") for _ in range(9)]), DB)
    s = p._apply(p.s0, Fx("draw", 3), None, "SPELL", None)
    assert s.drawn == 1, s.drawn


def test_frozen_enemy_is_no_threat():
    st = state(mana=0, my_hp=4, enemy_board=[minion("T_YETI", 5, 5, kw=("frozen",), mine=False)])
    p = Planner(st, DB)
    assert p.threat(p.s0) == 0


def test_enemy_lifesteal_heals_them():
    st = state(mana=0, my_hp=10, enemy_hp=20, enemy_board=[minion("T_YETI", 4, 4, kw=("lifesteal",), mine=False)])
    p = Planner(st, DB)
    after = p.enemy_turn(p.s0)
    assert (after.en_hp, after.my_hp) == (24, 6)


def test_big_board_stays_within_budget():
    hand = [card("T_YETI") for _ in range(3)] + [card("T_ARCANE") for _ in range(3)] + [card("T_HOLY")]
    board = [minion("T_BOAR", 2 + i, 3) for i in range(5)]
    enemies = [minion("T_YETI", 3, 3 + i, kw=("taunt",) if i == 0 else (), mine=False) for i in range(6)]
    st = state(
        mana=10, my_hp=20, enemy_hp=22, hand=hand, board=board, enemy_board=enemies, hero_power=power("T_BP_FACE")
    )
    t = time.perf_counter()
    _, acts, _ = search(st)
    assert time.perf_counter() - t < 2.0
    assert len(acts) >= 3, acts


# ---------------------------------------------------------------- minion types

STORM = {"play": [{"op": "dmg_same_race", "n": 3, "tgt": "any_minion"}]}
CHANT = {"play": [{"op": "buff_all", "a": 1, "h": 1, "cond": "race:MURLOC"}]}
MASK = {"play": [{"op": "mask_fill", "n": 1}]}


@pytest.fixture
def hand_written(monkeypatch):
    """Install hand-written rules for the synthetic spells, as a card_fx file would."""

    def install(**specs):
        for cid, spec in specs.items():
            monkeypatch.setitem(rules.CARD_FX, cid, spec)

    return install


def drawn(states):
    return sorted(s.drawn for s in states)


def stats(side):
    return sorted((u[1], u[2]) for u in side)


def test_same_type_damage_hits_both_boards(hand_written):
    hand_written(T_STORM=STORM)
    target = minion("T_FISH", 2, 5, mine=False)
    st = state(
        mana=2,
        hand=[card("T_STORM")],
        board=[minion("T_CHIMERA", 2, 5), minion("T_WOLF", 2, 5)],
        enemy_board=[target, minion("T_WOLF", 2, 5, mine=False), minion("T_FISH", 2, 3, mine=False)],
    )
    p, mv = moves(st)
    hit = [s for a, s in mv if a[0] == "play" and a[2] == target.eid]
    assert hit, "the spell can target a minion of a type"
    after = hit[0]
    assert stats(after.mine) == [(2, 2), (2, 5)], "my Murloc-Beast took 3, my plain Beast did not"
    assert stats(after.theirs) == [(2, 2), (2, 5)], "their Murlocs took 3 (one died), their Beast did not"


def test_buff_with_a_type_condition_reaches_only_that_type(hand_written):
    hand_written(T_CHANT=CHANT)
    st = state(
        mana=2,
        hand=[card("T_CHANT")],
        board=[minion("T_FISH", 2, 3), minion("T_WOLF", 2, 3), minion("T_CHIMERA", 2, 3)],
    )
    p, mv = moves(st)
    played = [s for a, s in mv if a[0] == "play"]
    assert played
    assert stats(played[0].mine) == [(2, 3), (3, 4), (3, 4)]


def test_mask_fill_fills_the_board_with_one_one_minions(hand_written):
    hand_written(T_MASK=MASK)
    st = state(mana=2, hand=[card("T_MASK")], board=[minion("T_OGRE", 6, 7), minion("T_BOAR", 3, 3)])
    p, mv = moves(st)
    played = [s for a, s in mv if a[0] == "play"]
    assert played
    assert stats(played[0].mine) == [(1, 1)] * 6 + [(6, 7)]


def test_minion_played_from_hand_keeps_its_type(hand_written):
    hand_written(T_STORM=STORM)
    fish = card("T_FISH")
    st = state(mana=3, hand=[fish, card("T_STORM")], enemy_board=[minion("T_FISH", 2, 3, mine=False)])
    p, mv = moves(st)
    after_fish = [s for a, s in mv if a[0] == "play" and a[1] == fish.eid][0]
    assert after_fish.mine[0].race == p.races[fish.eid] != 0


# ---------------------------------------------------------------- card-drawing rules


def swings(st):
    """The positions after my hero attacks the enemy minion."""
    p, mv = moves(st)
    return [s for a, s in mv if a[0] == "attack" and a[1] == HERO and a[2] != FACE]


def test_weapon_that_breaks_with_the_swing_draws(hand_written):
    hand_written(T_AXE={"wtrig": [{"op": "draw_brk", "n": 1}]})
    target = minion("T_BOAR", 1, 9, mine=False)
    last = state(
        my_weapon_id="T_AXE", my_weapon_atk=3, my_weapon_dur=1, hero_atk=3, hero_can_attack=True, enemy_board=[target]
    )
    assert drawn(swings(last)) == [1]
    more = state(
        my_weapon_id="T_AXE", my_weapon_atk=3, my_weapon_dur=2, hero_atk=3, hero_can_attack=True, enemy_board=[target]
    )
    assert drawn(swings(more)) == [0]


def test_weapon_that_kills_draws(hand_written):
    hand_written(T_AXE={"wtrig": [{"op": "draw_kill", "n": 1}]})
    weak, strong = minion("T_BOAR", 1, 2, mine=False), minion("T_OGRE", 1, 9, mine=False)
    st = state(
        my_weapon_id="T_AXE",
        my_weapon_atk=3,
        my_weapon_dur=2,
        hero_atk=3,
        hero_can_attack=True,
        enemy_board=[weak, strong],
    )
    p, mv = moves(st)
    after = {a[2]: s.drawn for a, s in mv if a[0] == "attack" and a[1] == HERO and a[2] != FACE}
    assert after == {weak.eid: 1, strong.eid: 0}


def test_damage_that_draws_if_the_minion_lives_and_summons_if_it_dies(hand_written):
    hand_written(T_BLAST={"play": [{"op": "dmg_surv", "n": 2, "a": 2, "h": 2, "tgt": "any_minion"}]})
    tough, frail = minion("T_YETI", 4, 5, mine=False), minion("T_BOAR", 1, 2, mine=False)
    p, mv = moves(state(mana=3, hand=[card("T_BLAST")], enemy_board=[tough, frail]))
    by = {a[2]: s for a, s in mv if a[0] == "play"}
    assert (by[tough.eid].drawn, len(by[tough.eid].mine)) == (1, 0)
    assert (by[frail.eid].drawn, stats(by[frail.eid].mine)) == (0, [(2, 2)])


def test_area_damage_draws_for_each_minion_that_died(hand_written):
    hand_written(T_SPARK={"play": [{"op": "aoe_draw", "n": 1}]})
    st = state(
        mana=1,
        hand=[card("T_SPARK")],
        board=[minion("T_BOAR", 1, 1), minion("T_YETI", 4, 5)],
        enemy_board=[minion("T_BOAR", 1, 1, mine=False), minion("T_BOAR", 1, 1, mine=False)],
    )
    p, mv = moves(st)
    played = [s for a, s in mv if a[0] == "play"]
    assert drawn(played) == [3]


def test_deathrattle_that_puts_cards_in_my_hand(hand_written):
    hand_written(T_COINER={"dr": [{"op": "value", "n": 12}], "dr_gen": 1})
    mine = minion("T_COINER", 1, 1)
    enemy = minion("T_RAPTOR", 3, 3, mine=False)
    p, mv = moves(state(board=[mine], enemy_board=[enemy]))
    traded = [s for a, s in mv if a[0] == "attack" and a[1] == mine.eid and a[2] == enemy.eid]
    assert drawn(traded) == [1], "it died in the trade and left a card"


def test_battlecry_reaches_an_elusive_minion_but_a_spell_does_not(hand_written):
    ghost = minion("T_BOAR", 1, 3, kw=("elusive",), mine=False)
    p, mv = moves(state(mana=2, hand=[card("T_PINGER")], enemy_board=[ghost]))
    assert any(a[0] == "play" and a[2] == ghost.eid for a, _ in mv)
    p, mv = moves(state(mana=1, hand=[card("T_ARCANE")], enemy_board=[ghost]))
    assert not any(a[0] == "play" and a[2] == ghost.eid for a, _ in mv)


def test_upgrading_draw_card_uses_its_live_number():
    c = card("FIR_911")
    c.script = (4, 0)
    p, mv = moves(state(mana=2, hand=[c]))
    assert drawn([s for a, s in mv if a[0] == "play"]) == [4]


def test_held_back_cards_fill_free_hand_slots():
    # held-back cards count only while the hand starts full, and only up to the free slots
    st = state(mana=0, held_back=3)
    assert Planner(st, DB).held_back == 0
    st.hand = [card("T_ARCANE") for _ in range(10)]
    assert Planner(st, DB).held_back == 3
    d = {"hand": [], "drawn": 8, "mine": [], "theirs": [], "my_hp": 30, "my_armor": 0, "en_hp": 30}
    assert check.outcome(d, set(), 3)[4] == 10
    assert check.outcome(d, set(), 0)[4] == 8


def test_chest_deathrattle_fills_their_hand():
    c = card("T_CHEST")
    p, _ = moves(state(mana=2, hand=[c]))
    assert p.info[c.eid].opp_draw == 10


def test_ley_bolt_spills_the_excess_to_the_hero():
    bolt = card("MEND_500")
    bolt.script = (6, 1)
    weak = minion("T_BOAR", 1, 2, mine=False)
    st = state(mana=2, hand=[bolt], enemy_board=[weak])
    _, mv = moves(st)
    played = [s for a, s in mv if a[0] == "play" and a[1] == bolt.eid]
    assert played and all(s.en_hp == 26 and not s.theirs for s in played), "6 on a 2-health minion: 4 spill"


def test_combo_damage_needs_a_card_played_first():
    blast = card("CATA_785")
    foe = minion("T_YETI", 4, 5, mine=False)

    def results(played):
        st = state(mana=2, hand=[blast], enemy_board=[foe], cards_played=played)
        return [s for a, s in moves(st)[1] if a[0] == "play" and a[1] == blast.eid]

    assert {s.en_hp for s in results(0)} == {30}, "first card of the turn: no damage"
    assert 27 in {s.en_hp for s in results(1)}, "after another card: 3 to any character"


def test_spill_card_hits_a_minion_then_one_more_without_overflow():
    spill = card("CATA_485")
    foe = minion("T_BOAR", 1, 1, mine=False)
    st = state(mana=2, hand=[spill], enemy_board=[foe])
    played = [s for a, s in moves(st)[1] if a[0] == "play" and a[1] == spill.eid]
    assert played and all(s.en_hp >= 28 for s in played), "the 1-damage part never reaches the hero"


def test_ley_line_cards_read_their_script_numbers():
    bolt, summon, draw = card("MEND_500"), card("MEND_502"), card("MEND_504")
    bolt.script, summon.script, draw.script = (6, 2), (3, 2), (0, 3)
    p, _ = moves(state(mana=6, hand=[bolt, summon, draw]))
    fx = lambda c: p.info[c.eid].modes[0][0]  # noqa: E731
    assert [(f.op, f.n, f.a) for f in fx(bolt)] == [("dmg_excess", 6, 2)]
    assert [(f.op, f.n, f.a, f.h) for f in fx(summon)] == [("summon", 2, 3, 3)]
    assert [(f.op, f.n) for f in fx(draw) if f.op == "draw"] == [("draw", 3)]
    one = card("MEND_504")
    one.script = (4,)
    p, _ = moves(state(mana=2, hand=[one]))
    assert [f.n for f in p.info[one.eid].modes[0][0] if f.op == "draw"] == [1]


def test_atiesh_doubles_spell_damage_and_healing():
    shot, heal, golem = card("T_ARCANE"), card("T_HEAL"), card("T_GOLEM")
    st = state(mana=6, hand=[shot, heal, golem], my_weapon_id="TIME_890t", my_weapon_atk=1, my_weapon_dur=2)
    p, _ = moves(st)
    assert [f.n for f in p.info[shot.eid].modes[0][0]] == [4]
    assert [f.n for f in p.info[heal.eid].modes[0][0]] == [16]
    assert p.info[golem.eid].modes == moves(state(mana=6, hand=[golem]))[0].info[golem.eid].modes
    plain, _ = moves(state(mana=6, hand=[shot]))
    assert [f.n for f in plain.info[shot.eid].modes[0][0]] == [2]


# ---------------------------------------------------------------- hand-dependent card texts


def summons_of(p, e):
    return sum(f.n for f in p.info[e.eid].modes[0][0] if f.op == "summon")


def test_holding_a_dragon_adds_a_summon():
    nest, wyrm, boar = card("T_NEST"), card("T_WYRM"), card("T_BOAR")
    with_dragon, _ = moves(state(mana=2, hand=[nest, wyrm]))
    without, _ = moves(state(mana=2, hand=[nest, boar]))
    assert summons_of(with_dragon, nest) == summons_of(without, nest) + 1


def test_outcast_draws_only_from_the_edge_of_the_hand():
    seer, a, b = card("T_SEER"), card("T_BOAR"), card("T_BOAR")
    draws = lambda p: sum(f.n for f in p.info[seer.eid].modes[0][0] if f.op == "draw")  # noqa: E731
    edge, _ = moves(state(mana=1, hand=[seer, a, b]))
    middle, _ = moves(state(mana=1, hand=[a, seer, b]))
    assert draws(edge) == draws(middle) + 1


def test_destroy_a_minion_may_pick_my_own_minion():
    kill = card("T_KILL")
    own = minion("T_BOAR", 1, 1)
    p, ms = moves(state(mana=3, hand=[kill], board=[own]))
    assert any(a[0] == "play" and a[2] == own.eid for a, _ in ms)


def test_the_rafaams_share_a_key_whatever_their_names():
    green, other = minion("TIME_005t2", 3, 3), minion("TIME_005t3", 3, 3)
    p = Planner(state(board=[green, other]), DB)
    names = {p._rust_info(e.eid, p.info[e.eid])["name"] for e in (green, other)}
    assert len(names) == 1
