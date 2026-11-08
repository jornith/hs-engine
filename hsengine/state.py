"""Game log lines -> GameState: the position the engine needs, from the side of the local player."""

import re
from dataclasses import dataclass, field

from hearthstone.enums import CardType, GameTag, Step, Zone
from hslog import LogParser
from hslog.export import EntityTreeExporter

# entity tags exposed to the engine as keywords
KEYWORDS = {
    GameTag.TAUNT: "taunt",
    GameTag.DIVINE_SHIELD: "divine",
    GameTag.STEALTH: "stealth",
    GameTag.CHARGE: "charge",
    GameTag.RUSH: "rush",
    GameTag.WINDFURY: "windfury",
    GameTag.POISONOUS: "poisonous",
    GameTag.LIFESTEAL: "lifesteal",
    GameTag.DEATHRATTLE: "deathrattle",
    GameTag.FROZEN: "frozen",
    GameTag.ELUSIVE: "elusive",
    GameTag.CANT_BE_TARGETED_BY_SPELLS: "elusive",
    GameTag.IMMUNE: "immune",
    GameTag.CANT_ATTACK: "cant_attack",
    GameTag.REBORN: "reborn",
    GameTag.SILENCED: "silenced",
    GameTag.IMMUNE_WHILE_ATTACKING: "iwa",
    GameTag.DORMANT: "dormant",
    GameTag.SECRET: "secret",
}


@dataclass(slots=True)
class Ent:
    eid: int
    card_id: str
    cost: int = 0
    atk: int = 0
    health: int = 0
    taunt: bool = False
    can_attack: bool = False
    divine: bool = False
    stealth: bool = False
    attacks: int = 0
    ready: bool = True  # hero power: not yet used this turn
    maxhp: int = 0
    kw: frozenset = frozenset()
    sp: int = 0  # Spell Damage +N
    fresh: bool = False  # entered play this turn (asleep unless Charge/Rush)
    script: tuple = ()  # TAG_SCRIPT_DATA_NUM_1/2: the live numbers behind "{0}/{1}" in the card text


@dataclass(slots=True)
class GameState:
    turn: int = 0
    my_turn: bool = False
    mana: int = 0
    hand: list[Ent] = field(default_factory=list)
    board: list[Ent] = field(default_factory=list)
    enemy_board: list[Ent] = field(default_factory=list)
    my_hp: int = 0  # health + armor
    my_armor: int = 0
    my_max_hp: int = 30
    enemy_hp: int = 0  # health + armor
    enemy_hand_size: int = 0
    hero_atk: int = 0
    hero_can_attack: bool = False
    hero_power: Ent | None = None
    enemy_power: Ent | None = None
    enemy_weapon_atk: int = 0
    my_weapon_atk: int = 0
    my_weapon_dur: int = 0
    max_mana: int = 0
    mulligan: bool = False  # choosing which starting cards to replace
    first: bool = False  # I go first
    result: str | None = None  # WON / LOST / TIED once the game ends
    my_player_id: int | None = None
    main_action: bool = False  # the game is in the action phase (mana/draws for the turn are settled)
    enemy_secrets: int = 0
    enemy_pending: list = field(default_factory=list)  # their revealed "next turn" effects (card ids)
    enemy_hero: str = ""  # hero card id (its class narrows the deck guess)
    my_hero: str = ""
    enemy_seen: list = field(default_factory=list)  # card ids the opponent has shown (played, in play, in graveyard)
    enemy_max_mana: int = 0
    corpses: int = 0  # Death Knight corpses
    cards_played: int = 0  # cards I have played this turn so far (player tag NUM_CARDS_PLAYED_THIS_TURN): Combo
    jade: int = 0  # Jade Golems I summoned this game
    herald: int = 0  # times I Heralded this game
    spell_power: int = 0  # my total Spell Damage as the game counts it
    role: int = 0  # +1 beatdown, -1 control (from the decks' average cost), 0 unknown
    my_weapon_id: str = ""
    hero_id: int = 0  # my hero entity id
    my_played: list = field(default_factory=list)  # my deck cards already played (in play or graveyard)
    my_seen: list = field(default_factory=list)  # my own deck cards already out of the deck (hand, play, graveyard)
    locations: list = field(default_factory=list)  # my locations on the board that can be activated now
    held_back: int = 0  # my cards held aside by an enchantment (JAIL_509e) until my hand has room


def _ent(e) -> Ent:
    t = e.tags
    fresh = t.get(GameTag.NUM_TURNS_IN_PLAY, 0) == 0
    # Rush given this turn to a minion already played (Soulrest Ceremony): the game keeps EXHAUSTED=1 and
    # sets ATTACKABLE_BY_RUSH instead; it may attack minions only
    rush_now = bool(t.get(GameTag.ATTACKABLE_BY_RUSH, 0)) and bool(t.get(GameTag.RUSH, 0))
    if rush_now:
        fresh = True
    exhausted = (t.get(GameTag.EXHAUSTED, 0) and not rush_now) or t.get(GameTag.FROZEN, 0) or not t.get(GameTag.ATK, 0)
    attacks_left = (2 if t.get(GameTag.WINDFURY, 0) else 1) - t.get(GameTag.NUM_ATTACKS_THIS_TURN, 0)
    can = (
        not exhausted
        and attacks_left > 0
        and not t.get(GameTag.CANT_ATTACK, 0)
        and (not fresh or t.get(GameTag.CHARGE, 0) or t.get(GameTag.RUSH, 0))
    )
    return Ent(
        e.id,
        e.card_id or "",
        t.get(GameTag.COST, 0),
        t.get(GameTag.ATK, 0),
        t.get(GameTag.HEALTH, 0) - t.get(GameTag.DAMAGE, 0),
        bool(t.get(GameTag.TAUNT, 0)),
        bool(can),
        bool(t.get(GameTag.DIVINE_SHIELD, 0)),
        bool(t.get(GameTag.STEALTH, 0)),
        max(attacks_left, 0) if can else 0,
        not t.get(GameTag.EXHAUSTED, 0),
        t.get(GameTag.HEALTH, 0),
        frozenset(name for tag, name in KEYWORDS.items() if t.get(tag, 0)),
        max(
            t.get(GameTag.SPELLPOWER, 0), t.get(GameTag.CURRENT_SPELLPOWER, 0) if t.get(GameTag.ZONE, 0) == 3 else 0
        ),  # a spell in hand can carry its own
        bool(fresh),
        tuple(t[g] for g in (GameTag.TAG_SCRIPT_DATA_NUM_1, GameTag.TAG_SCRIPT_DATA_NUM_2) if g in t),
    )


_OPT_PLAYER = re.compile(r"mainEntity=\[.*? player=(\d+)\]")
_PLAYER_ENT = re.compile(r"Player EntityID=(\d+) PlayerID=(\d+)")
_FULL_NAMED = re.compile(r"FULL_ENTITY - Updating ([^\[\d][^\[]*?) CardID=")  # names may have spaces
_NAMED = re.compile(r"Entity=([^\[\]=\d][^\[\]=]*?) (?:tag=|EffectCardId=)")
_PLAYER_NAME = re.compile(r"PlayerID=(\d+), PlayerName=(.+)$")


class StateTracker:
    """Feed log lines one by one; snapshot() returns the current GameState (None before the game exists).

    `own_names`: the local player's names, for logs that hide the opponent's name (optional)."""

    def __init__(self, own_names=()):
        self.own_names = set(own_names)
        self.parser = LogParser()
        self._pent: dict = {}  # player id -> player entity id (CREATE_GAME)
        self._hidden: int | None = None  # entity id of a player shown as "UNKNOWN HUMAN PLAYER"
        self._names: dict = {}  # "Entity=<player name> " -> "Entity=<entity id> "
        self._pname: dict = {}  # player id -> name announced at the game start
        self.dirty = False
        self.turn = 0  # last TURN of the game entity
        self.options_turn = -1  # TURN when the game last offered me options (only on my own turn)
        self.local_id: int | None = None  # my PLAYER_ID, from the cards the game offers me to play

    def feed(self, line: str) -> None:
        if not line:
            return
        if "GameState.DebugPrintPower() - CREATE_GAME" in line:
            # fresh parser per game: hslog keeps player ids across games and raises
            # InconsistentPlayerIdError when you are player 1 in one game and player 2 in the next
            self.parser = LogParser()
            self._pent, self._hidden, self._names, self._pname = {}, None, {}, {}
            self.turn, self.options_turn, self.local_id = 0, -1, None
        elif "GameState.DebugPrintPower()" in line and "Entity=GameEntity tag=TURN value=" in line:
            try:
                self.turn = int(line.rsplit("value=", 1)[1].split()[0])
            except ValueError:
                pass
        elif "GameState.DebugPrintOptions() - id=" in line:
            self.options_turn = self.turn
        if (
            self.local_id is None
            and "GameState.DebugPrintOptions()" in line
            and "mainEntity=[" in line
            and "error=NONE" in line
        ):
            m = _OPT_PLAYER.search(line)
            if m:
                self.local_id = int(m.group(1))
                name = self._pname.get(self.local_id, "")
                if "#" in name:
                    self.own_names.add(name)
        # a hidden opponent name ("UNKNOWN HUMAN PLAYER"): hslog cannot tie it to a player and every later
        # export fails ("Entity ID not available"). Map it to the player's entity id ourselves.
        if "Player EntityID=" in line and (m := _PLAYER_ENT.search(line)):
            self._pent[int(m.group(2))] = int(m.group(1))
        elif "DebugPrintGame() - PlayerID=" in line and (m := _PLAYER_NAME.search(line)):
            self._pname[int(m.group(1))] = m.group(2).strip()
            if int(m.group(1)) in self._pent:
                self._names["Entity=" + m.group(2).strip() + " "] = f"Entity={self._pent[int(m.group(1))]} "
                if m.group(2).strip() == "UNKNOWN HUMAN PLAYER":
                    self._hidden = self._pent[int(m.group(1))]
        # player names -> their entity ids, for BOTH players: with one name hidden hslog guessed and gave
        # my mana tags to the opponent (5 mana shown instead of 3)
        if self._hidden and "Entity=" in line and "DebugPrintGame()" not in line and (m := _NAMED.search(line)):
            # the game may name me differently later (a localised "You" at the start, "name#1234" in tag
            # changes): with the opponent hidden, any other player name is me
            key = "Entity=" + m.group(1) + " "
            if key not in self._names and m.group(1) != "GameEntity":
                # a name not announced at the start: one of `own_names` is me, anything else is the hidden
                # opponent showing their real name
                other = [e for e in self._pent.values() if e != self._hidden]
                mine = m.group(1) in self.own_names or (
                    m.group(1) not in self.own_names
                    and "#" not in m.group(1)
                    and not any(v == f"Entity={other[0]} " for v in self._names.values())
                )
                if other:
                    self._names[key] = f"Entity={other[0] if mine else self._hidden} "
        if self._names and "Entity=" in line and "DebugPrintGame()" not in line:
            for k, v in self._names.items():
                if k in line:
                    line = line.replace(k, v)
        # a full resend of the game (Rewind / reconnect) names the players ("FULL_ENTITY - Updating <name>
        # CardID="), which the log parser cannot resolve: the name becomes the player's entity id
        if "FULL_ENTITY - Updating " in line and (m := _FULL_NAMED.search(line)):
            name = m.group(1)
            eid = next(
                (
                    self._pent[pid]
                    for pid, n in self._pname.items()
                    if pid in self._pent and (n == name or n.split("#")[0] == name.split("#")[0])
                ),
                None,
            )
            if eid is None and len(self._pent) == 2:  # unknown name: the hidden opponent
                eid = self._hidden
            if eid is not None:
                line = line[: m.start(1)] + str(eid) + line[m.end(1) :]
        try:
            self.parser.read_line(line)
        except Exception:
            self.errors = getattr(self, "errors", 0) + 1  # one odd line must never kill the tracker
        self.dirty = True

    def snapshot(self) -> GameState | None:
        if not self.dirty or not self.parser.games:
            return None
        self.dirty = False
        self.parser.flush()
        try:
            game = EntityTreeExporter(self.parser.games[-1], tolerate_missing_entities=True).export().game
        except Exception:
            return None
        if game is None:  # the game has not been created yet in what was read so far
            return None
        st = self._build(game, self.local_id)
        if st is not None and self.options_turn == self.turn and self.turn and not st.result:
            st.my_turn = True  # the game is offering me moves: it is my turn whatever the parser thinks
        return st

    @staticmethod
    def _build(game, local_id: int | None = None) -> GameState | None:
        players = list(game.players)
        if len(players) != 2:
            return None

        # "me" = the player whose hand cards have known ids
        def known(p):
            return sum(1 for e in game.in_zone(Zone.HAND) if e.controller == p and e.card_id)

        # my PLAYER_ID is known once the game has offered me something to play; until then guess by the
        # hand whose cards I can see (fails when my hand is empty and theirs holds revealed cards)
        me = next((p for p in players if local_id is not None and p.tags.get(GameTag.PLAYER_ID) == local_id), None)
        if me is None:
            me = max(players, key=known)

        st = GameState(turn=game.tags.get(GameTag.TURN, 0))
        _read_player(st, game, me)
        enemy = next((p for p in players if p is not me), None)
        if enemy is not None:
            st.enemy_max_mana = enemy.tags.get(GameTag.RESOURCES, 0)
        _read_seen(st, game, me, enemy)
        _read_held_back(st, game, me)
        _read_hands(st, game, me)
        _read_play(st, game, me)
        return st


def _read_player(st: GameState, game, me) -> None:
    """My player tags: turn flags, mana, Corpses, Herald and Jade counts, Spell Damage."""
    st.my_turn = game.current_player is me
    st.main_action = game.tags.get(GameTag.STEP) == Step.MAIN_ACTION
    st.mulligan = me.tags.get(GameTag.MULLIGAN_STATE, 0) == 1  # Mulligan.INPUT
    st.first = bool(me.tags.get(GameTag.FIRST_PLAYER, 0))
    st.my_player_id = me.tags.get(GameTag.PLAYER_ID)
    st.result = {4: "WON", 5: "LOST", 6: "TIED"}.get(int(me.tags.get(GameTag.PLAYSTATE, 0)))
    mt = me.tags
    st.max_mana = mt.get(GameTag.RESOURCES, 0)
    # CORPSES counts every corpse gained this game; the spent ones are counted separately
    st.corpses = max(0, mt.get(GameTag.CORPSES, 0) - mt.get(GameTag.CORPSES_SPENT_THIS_GAME, 0))
    st.cards_played = mt.get(GameTag.NUM_CARDS_PLAYED_THIS_TURN, 0)
    st.herald = mt.get(GameTag.HERALD_COLOSSAL_AMOUNT, 0)  # Herald count: the Soldier grows with it
    st.spell_power = mt.get(GameTag.CURRENT_SPELLPOWER, 0)  # all of my Spell Damage (minions + the rest)
    st.jade = mt.get(GameTag.JADE_GOLEM, 0)  # Jade Golems summoned so far: the next one is (jade+1)/(jade+1)
    st.mana = (
        mt.get(GameTag.RESOURCES, 0)
        + mt.get(GameTag.TEMP_RESOURCES, 0)
        - mt.get(GameTag.RESOURCES_USED, 0)
        - mt.get(GameTag.OVERLOAD_LOCKED, 0)
    )


def _read_held_back(st: GameState, game, me) -> None:
    """Cards the enchantment JAIL_509e copied aside when my hand was full: they join the hand once it has room."""
    ench = {e.id for e in game.entities if getattr(e, "card_id", None) == "JAIL_509e" and e.controller is me}
    if not ench:
        return
    st.held_back = sum(
        1
        for e in game.entities
        if e.zone == Zone.SETASIDE
        and e.controller is me
        and getattr(e, "card_id", None)
        and e.tags.get(GameTag.CREATOR) in ench
        and e.tags.get(GameTag.CARDTYPE) != CardType.ENCHANTMENT
    )


def _read_seen(st: GameState, game, me, enemy) -> None:
    """Cards already out of the decks: mine (seen and played) and the enemy's (shown, and their hero)."""
    for e in game.entities:  # cards already out of the decks
        cid = getattr(e, "card_id", None)
        if (
            cid
            and e.controller is me
            and e.zone in (Zone.HAND, Zone.PLAY, Zone.GRAVEYARD, Zone.SECRET)
            and not e.tags.get(GameTag.CREATOR)
            and e.tags.get(GameTag.CARDTYPE) not in (CardType.HERO, CardType.HERO_POWER)
        ):
            st.my_seen.append(cid)
            if e.zone in (Zone.PLAY, Zone.GRAVEYARD):
                st.my_played.append(cid)
        if (
            cid
            and e.controller is enemy
            and e.zone in (Zone.PLAY, Zone.GRAVEYARD, Zone.SECRET)
            and not e.tags.get(GameTag.CREATOR)
            and e.tags.get(GameTag.CARDTYPE) != CardType.HERO_POWER
        ):
            if e.tags.get(GameTag.CARDTYPE) == CardType.HERO:
                if e.zone == Zone.PLAY:
                    st.enemy_hero = cid
            else:
                st.enemy_seen.append(cid)


def _read_hands(st: GameState, game, me) -> None:
    """My hand, their hand size, their Secrets and revealed delayed effects."""
    for e in game.in_zone(Zone.HAND):
        if e.controller == me:
            st.hand.append(_ent(e))
        else:
            st.enemy_hand_size += 1
    st.enemy_secrets = sum(1 for e in game.in_zone(Zone.SECRET) if e.controller != me and e.tags.get(GameTag.SECRET, 0))
    # their revealed delayed effects ("At the start of your next turn, deal 6 damage...": Seal of Embers)
    st.enemy_pending = [
        e.card_id
        for e in game.in_zone(Zone.SECRET)
        if e.controller != me and e.card_id and not e.tags.get(GameTag.SECRET, 0)
    ]


def _read_play(st: GameState, game, me) -> None:
    """Everything in play: minions left to right, heroes, my locations, hero powers and weapons."""
    pos: dict = {}
    for e in game.in_zone(Zone.PLAY):
        ctype = e.tags.get(GameTag.CARDTYPE)
        mine = e.controller == me
        if ctype == CardType.MINION:
            pos[e.id] = e.tags.get(GameTag.ZONE_POSITION, 0)
            (st.board if mine else st.enemy_board).append(_ent(e))
        elif ctype == CardType.HERO:
            t = e.tags
            armor = t.get(GameTag.ARMOR, 0)
            hp = t.get(GameTag.HEALTH, 30) - t.get(GameTag.DAMAGE, 0) + armor
            if mine:
                st.hero_id = e.id
                st.my_hero = e.card_id or ""
                st.my_hp, st.my_armor, st.my_max_hp = hp, armor, t.get(GameTag.HEALTH, 30)
                st.hero_atk = t.get(GameTag.ATK, 0)
                st.hero_can_attack = bool(
                    st.hero_atk and not t.get(GameTag.FROZEN, 0) and not t.get(GameTag.NUM_ATTACKS_THIS_TURN, 0)
                )
            else:
                st.enemy_hp = hp
        elif ctype == CardType.LOCATION and mine:
            t = e.tags
            loc = _ent(e)
            loc.ready = (
                not t.get(GameTag.LOCATION_ACTION_COOLDOWN, 0) and not t.get(GameTag.EXHAUSTED, 0) and loc.health > 0
            )
            st.locations.append(loc)
        elif ctype == CardType.HERO_POWER:
            if mine:
                st.hero_power = _ent(e)
            else:
                st.enemy_power = _ent(e)
        elif ctype == CardType.WEAPON:
            if mine:
                st.my_weapon_atk = e.tags.get(GameTag.ATK, 0)
                st.my_weapon_id = e.card_id or ""
                dur = e.tags.get(GameTag.DURABILITY, 0) or e.tags.get(GameTag.HEALTH, 0)  # newer logs: HEALTH
                st.my_weapon_dur = max(1, dur - e.tags.get(GameTag.DAMAGE, 0))  # it is equipped, so >= 1
            else:
                st.enemy_weapon_atk = e.tags.get(GameTag.ATK, 0)
    # left to right as on the board
    st.board.sort(key=lambda e: pos.get(e.eid, 0))
    st.enemy_board.sort(key=lambda e: pos.get(e.eid, 0))
