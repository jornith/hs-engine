"""Zipped XML replays (`*.xml.zip` holding `replay.xml`) -> game log lines.

The XML is written back as the `GameState.DebugPrintPower()` lines the game client logs, so `StateTracker`
reads replays and live logs the same way. The main player is the one marked `isMainPlayer`.
"""

import xml.etree.ElementTree as ET
import zipfile

from hearthstone.enums import (
    BlockType,
    CardType,
    GameTag,
    MetaDataType,
    Mulligan,
    PlayState,
    Step,
    Zone,
)

_ENUMS = {
    GameTag.STEP: Step,
    GameTag.NEXT_STEP: Step,
    GameTag.ZONE: Zone,
    GameTag.PLAYSTATE: PlayState,
    GameTag.CARDTYPE: CardType,
    GameTag.MULLIGAN_STATE: Mulligan,
}

P = "D {ts} GameState.DebugPrintPower() - "


def _ts(e, last: str) -> str:
    t = e.attrib.get("ts") or last
    if "." in t:
        hms, frac = t.split(".")
        t = f"{hms}.{(frac + '0000000')[:7]}"
    return t


def _tag(tag: str) -> str:
    try:
        return GameTag(int(tag)).name
    except ValueError:
        return tag


def _val(tag: str, value: str) -> str:
    """Enum values as the game writes them (STEP value=MAIN_ACTION, ZONE value=HAND...)."""
    try:
        enum = _ENUMS.get(GameTag(int(tag)))
        return enum(int(value)).name if enum else value
    except ValueError:
        return value


_ENTITY_KINDS = ("FullEntity", "ShowEntity", "ChangeEntity", "GameEntity", "Player")
_SKIPPED = ("Options", "SendOption", "Choices", "SendChoices", "ChosenEntities")


def _emit(out: list[str], ts: str, text: str, indent: int = 0) -> None:
    """Append one log line with the timestamp prefix and `indent` levels of indentation."""
    out.append(P.format(ts=ts) + "    " * indent + text)


def _emit_tags(out: list[str], e, ts: str, indent: int) -> None:
    """One `tag=... value=...` line for every Tag child of `e`."""
    for t in e.findall("Tag"):
        _emit(out, ts, f"tag={_tag(t.attrib['tag'])} value={_val(t.attrib['tag'], t.attrib['value'])}", indent)


def _emit_entity(out: list[str], e, ts: str, indent: int) -> None:
    """Lines for GameEntity, Player and the entity-creating elements, followed by their tags."""
    k, a = e.tag, e.attrib
    if k == "GameEntity":
        _emit(out, ts, f"GameEntity EntityID={a['id']}", indent)
    elif k == "Player":
        _emit(
            out,
            ts,
            f"Player EntityID={a['id']} PlayerID={a['playerID']} "
            f"GameAccountId=[hi={a.get('accountHi', 0)} lo={a.get('accountLo', 0)}]",
            indent,
        )
    elif k == "FullEntity":
        _emit(out, ts, f"FULL_ENTITY - Creating ID={a['id']} CardID={a.get('cardID', '')}", indent)
    elif k == "ShowEntity":
        _emit(out, ts, f"SHOW_ENTITY - Updating Entity={a['entity']} CardID={a.get('cardID', '')}", indent)
    else:
        _emit(out, ts, f"CHANGE_ENTITY - Updating Entity={a['entity']} CardID={a.get('cardID', '')}", indent)
    _emit_tags(out, e, ts, indent + 1)


def _emit_meta(out: list[str], e, ts: str, indent: int) -> None:
    """The META_DATA line and one Info line per Info child."""
    a = e.attrib
    try:
        meta = MetaDataType(int(a.get("meta", 0))).name
    except ValueError:
        meta = a.get("meta")
    infos = e.findall("Info")
    _emit(out, ts, f"META_DATA - Meta={meta} Data={a.get('data', 0)} InfoCount={len(infos)}", indent)
    for j, inf in enumerate(infos):
        _emit(out, ts, f"Info[{j}] = {inf.attrib.get('entity', 0)}", indent + 1)


def _emit_block(out: list[str], e, ts: str, indent: int, state: dict) -> None:
    """BLOCK_START, the block's children one level deeper, then BLOCK_END."""
    a = e.attrib
    try:
        bt = BlockType(int(a.get("type", 0))).name
    except ValueError:
        bt = a.get("type")
    _emit(
        out,
        ts,
        f"BLOCK_START BlockType={bt} Entity={a.get('entity', 0)} EffectCardId= "
        f"EffectIndex={a.get('effectIndex', -1)} "
        f"Target={a.get('target', 0)} SubOption={a.get('subOption', -1)}",
        indent,
    )
    for c in e:
        _walk(out, c, indent + 1, state)
    _emit(out, ts, "BLOCK_END", indent)


def _emit_simple(out: list[str], e, ts: str, indent: int) -> None:
    """Lines for HideEntity, TagChange, MetaData and ShuffleDeck."""
    k, a = e.tag, e.attrib
    if k == "HideEntity":
        _emit(out, ts, f"HIDE_ENTITY - Entity={a['entity']} tag=ZONE value={a.get('zone', 'DECK')}", indent)
    elif k == "TagChange":
        ent = "GameEntity" if a["entity"] == "1" else a["entity"]
        _emit(out, ts, f"TAG_CHANGE Entity={ent} tag={_tag(a['tag'])} value={_val(a['tag'], a['value'])}", indent)
    elif k == "MetaData":
        _emit_meta(out, e, ts, indent)
    elif k == "ShuffleDeck":
        _emit(out, ts, f"SHUFFLE_DECK PlayerID={a.get('playerID', 0)}", indent)


def _walk(out: list[str], e, indent: int, state: dict) -> None:
    """Write the lines for `e` and, except for blocks and entities, for its children at the same level."""
    ts = state["ts"] = _ts(e, state["ts"])
    k = e.tag
    if k in _ENTITY_KINDS:
        _emit_entity(out, e, ts, indent)
    elif k == "Block":
        _emit_block(out, e, ts, indent, state)
        return
    else:
        _emit_simple(out, e, ts, indent)
    for c in e:
        if c.tag not in ("Tag", "Info") and k not in _ENTITY_KINDS:
            _walk(out, c, indent, state)


def to_lines(path: str) -> tuple[list[str], int | None]:
    """(log lines, PLAYER_ID of the uploader)."""
    root = ET.fromstring(zipfile.ZipFile(path).read("replay.xml"))
    game = root.find("Game")
    out: list[str] = []
    main_id = None
    state = {"ts": "00:00:00.0000000"}
    _emit(out, _ts(game, state["ts"]), "CREATE_GAME")
    for p in game.findall("Player"):
        if p.attrib.get("isMainPlayer") == "true":
            main_id = int(p.attrib["playerID"])
    for e in game:
        if e.tag in _SKIPPED:
            continue
        _walk(out, e, 1, state)
    return out, main_id
