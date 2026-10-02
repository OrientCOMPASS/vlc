#!/usr/bin/env python3
"""Inject / rewrite spherical video metadata in MP4 and Matroska files.

Used by CI to turn plain encoded test clips into properly tagged 360°/180°
items so that the `Auto` projection mode can be exercised on a device without
having to ship large binary fixtures in the repository.

Supported operations
--------------------
* MP4 (ISO-BMFF): insert `st3d` + `sv3d(svhd, proj(prhd, equi))` inside the
  first video sample entry (`avc1`, `hvc1`, `hev1`, `vp09`, `av01`, `mp4v`),
  following the Spherical Video V2 RFC.
* Matroska/WebM: insert `StereoMode` and `Projection(ProjectionType,
  ProjectionPose(…))` inside the first video track's `Video` element.

Everything else in the file is preserved byte for byte; box/element sizes are
rewritten along the path that is modified.

Usage
-----
    python3 spherical_inject.py input.mp4 output.mp4 \
        --coverage 360 --stereo sbs --eye-order left-first \
        --yaw 0 --pitch 0 --roll 0
"""

from __future__ import annotations

import argparse
import struct
import sys
from typing import List, Optional, Tuple

VIDEO_FOURCCS = {
    b"avc1", b"avc3", b"avc4", b"hvc1", b"hev1", b"hev2", b"vp08", b"vp09",
    b"av01", b"mp4v", b"encv", b"dvh1", b"dvhe",
}

# Spherical Video V1 uuid (used for maximum compatibility with older players)
SPHERICAL_V1_UUID = bytes.fromhex("ffcc8263f8554a938814587a02521fdd")


# --------------------------------------------------------------------------- #
# helpers
# --------------------------------------------------------------------------- #
def be(n: int, width: int) -> bytes:
    return n.to_bytes(width, "big")


def u32(v: float) -> int:
    """0.32 fixed point fraction used by the `equi` projection bounds."""
    return max(0, min(0xFFFFFFFF, int(round(v * float(1 << 32)))))


def fixed_16_16(deg: float) -> bytes:
    return struct.pack(">i", int(round(deg * 65536.0)))


def st3d_box(stereo_mode: int) -> bytes:
    # FullBox(version=0, flags=0) + stereo_mode(u8)
    body = b"\x00\x00\x00\x00" + bytes([stereo_mode & 0xFF])
    return be(len(body) + 8, 4) + b"st3d" + body


def prhd_box(yaw: float, pitch: float, roll: float) -> bytes:
    body = b"\x00\x00\x00\x00" + fixed_16_16(yaw) + fixed_16_16(pitch) + fixed_16_16(roll)
    return be(len(body) + 8, 4) + b"prhd" + body


def equi_box(top: float = 0.0, bottom: float = 0.0, left: float = 0.0, right: float = 0.0) -> bytes:
    body = (
        b"\x00\x00\x00\x00"
        + be(u32(top), 4)
        + be(u32(bottom), 4)
        + be(u32(left), 4)
        + be(u32(right), 4)
    )
    return be(len(body) + 8, 4) + b"equi" + body


def svhd_box(text: str = "libvlcrs test media") -> bytes:
    payload = text.encode("utf-8") + b"\x00"
    body = b"\x00\x00\x00\x00" + payload
    return be(len(body) + 8, 4) + b"svhd" + body


def container_box(fourcc: bytes, children: bytes) -> bytes:
    return be(len(children) + 8, 4) + fourcc + children


def spherical_children(stereo_mode: int, yaw: float, pitch: float, roll: float,
                       crop: Tuple[float, float, float, float]) -> bytes:
    proj = container_box(b"proj", prhd_box(yaw, pitch, roll) + equi_box(*crop))
    sv3d = container_box(b"sv3d", svhd_box() + proj)
    return st3d_box(stereo_mode) + sv3d


def v1_uuid_box(stereo: str, projection: str = "equirectangular",
                yaw: float = 0.0) -> bytes:
    xml = (
        '<?xml version="1.0"?>'
        '<rdf:SphericalVideo xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" '
        'xmlns:GSpherical="http://ns.google.com/videos/1.0/spherical/">'
        "<GSpherical:Spherical>true</GSpherical:Spherical>"
        "<GSpherical:Stitched>true</GSpherical:Stitched>"
        "<GSpherical:StitchingSoftware>libvlcrs</GSpherical:StitchingSoftware>"
        f"<GSpherical:ProjectionType>{projection}</GSpherical:ProjectionType>"
        f"<GSpherical:StereoMode>{stereo}</GSpherical:StereoMode>"
        f"<GSpherical:InitialViewPointDegrees>{yaw}</GSpherical:InitialViewPointDegrees>"
        "</rdf:SphericalVideo>"
    ).encode("utf-8")
    body = SPHERICAL_V1_UUID + xml
    return be(len(body) + 8, 4) + b"uuid" + body


# --------------------------------------------------------------------------- #
# MP4
# --------------------------------------------------------------------------- #
def read_box(data: bytes, pos: int) -> Optional[Tuple[bytes, int, int, int]]:
    """Return (fourcc, header_end, box_end, payload_start) or None at EOF."""
    if pos + 8 > len(data):
        return None
    size = int.from_bytes(data[pos:pos + 4], "big")
    fourcc = data[pos + 4:pos + 8]
    header = 8
    if size == 1:
        if pos + 16 > len(data):
            return None
        size = int.from_bytes(data[pos + 8:pos + 16], "big")
        header = 16
    elif size == 0:
        size = len(data) - pos
    if size < header or pos + size > len(data):
        return None
    return fourcc, pos + header, pos + size, pos + header


MP4_CONTAINERS = {b"moov", b"trak", b"mdia", b"minf", b"stbl", b"edts", b"udta"}


def inject_mp4(data: bytes, payload: bytes, v1: bytes, state: dict) -> bytes:
    """Walk the MP4 and insert `payload` into the first video sample entry."""
    out = bytearray()
    pos = 0
    while True:
        box = read_box(data, pos)
        if box is None:
            break
        fourcc, start, end, _payload = box
        body = data[start:end]
        if state["done"]:
            out += data[pos:end]
            pos = end
            continue
        if fourcc == b"stsd" and not state["done"]:
            # version+flags(4) + entry_count(4) then the entries
            header = body[:8]
            entries = bytearray()
            epos = 8
            injected = False
            while epos < len(body):
                ebox = read_box(body, epos)
                if ebox is None:
                    break
                efcc, estart, eend, _ = ebox
                ebody = body[estart:eend]
                if not injected and efcc in VIDEO_FOURCCS:
                    # VisualSampleEntry fixed part is 78 bytes
                    if len(ebody) >= 78:
                        ebody = ebody[:78] + payload + ebody[78:]
                        injected = True
                        state["done"] = True
                entries += be(len(ebody) + 8, 4) + efcc + ebody
                epos = eend
            new_body = header + bytes(entries)
            out += be(len(new_body) + 8, 4) + fourcc + new_body
            pos = end
            continue
        if fourcc == b"trak" and not state["done"]:
            # inject the V1 uuid at track level too (moov.trak.uuid per the RFC)
            new_body = inject_mp4(body, payload, v1, state)
            if state["done"]:
                new_body = new_body + v1
            out += be(len(new_body) + 8, 4) + fourcc + new_body
            pos = end
            continue
        if fourcc in MP4_CONTAINERS and not state["done"]:
            new_body = inject_mp4(body, payload, v1, state)
            out += be(len(new_body) + 8, 4) + fourcc + new_body
            pos = end
            continue
        out += data[pos:end]
        pos = end
    out += data[pos:]
    return bytes(out)


# --------------------------------------------------------------------------- #
# Matroska
# --------------------------------------------------------------------------- #
def ebml_id_len(first: int) -> int:
    n = 0
    while n < 8 and not (first & (0x80 >> n)):
        n += 1
    return n + 1


def read_vint(data: bytes, pos: int, masked: bool) -> Optional[Tuple[int, int]]:
    if pos >= len(data):
        return None
    first = data[pos]
    length = ebml_id_len(first)
    if pos + length > len(data):
        return None
    value = first
    if masked:
        value &= 0xFF >> length
    for i in range(1, length):
        value = (value << 8) | data[pos + i]
    unknown = masked and all(b == 0xFF for b in data[pos:pos + length]) and length < 8
    return value, length if not unknown else -length


def write_vint_size(size: int) -> bytes:
    for width in range(1, 9):
        marker = 1 << (7 * width)
        if size < marker - 1:
            return be(marker | size, width)
    raise ValueError("element too large")


ID_SEGMENT = 0x18538067
ID_TRACKS = 0x1654AE6B
ID_TRACK_ENTRY = 0xAE
ID_TRACK_TYPE = 0x83
ID_VIDEO = 0xE0
ID_STEREO_MODE = 0x53B8
ID_PROJECTION = 0x7670
ID_PROJECTION_TYPE = 0x7671
ID_PROJECTION_PRIVATE = 0x7672
ID_PROJECTION_POSE = 0x7673
ID_POSE_YAW = 0x7674
ID_POSE_PITCH = 0x7675
ID_POSE_ROLL = 0x7676


def write_id(eid: int) -> bytes:
    for width in (1, 2, 3, 4):
        if eid < (1 << (8 * width)):
            return be(eid, width)
    raise ValueError("bad id")


def ebml_uint(eid: int, value: int) -> bytes:
    body = b""
    for width in range(1, 9):
        if value < (1 << (8 * width)):
            body = be(value, width)
            break
    return write_id(eid) + write_vint_size(len(body)) + body


def ebml_float(eid: int, value: float) -> bytes:
    body = struct.pack(">d", value)
    return write_id(eid) + write_vint_size(len(body)) + body


def ebml_master(eid: int, children: bytes) -> bytes:
    return write_id(eid) + write_vint_size(len(children)) + children


def mkv_projection(stereo_mode: Optional[int], ptype: int, yaw: float, pitch: float,
                   roll: float, crop: Tuple[float, float, float, float],
                   pose_layout: str = "nested") -> bytes:
    """StereoMode + Projection(ProjectionType, ProjectionPrivate, ProjectionPose).

    `ProjectionPrivate` carries the same payload as an ISOBFF `equi` box body
    (FullBox version/flags + four 0.32 bounds), per the Spherical Video V2 RFC.
    """
    out = b""
    if stereo_mode is not None:
        out += ebml_uint(ID_STEREO_MODE, stereo_mode)
    private = (
        be(0, 4)
        + be(u32(crop[0]), 4)
        + be(u32(crop[1]), 4)
        + be(u32(crop[2]), 4)
        + be(u32(crop[3]), 4)
    )
    proj = (
        ebml_uint(ID_PROJECTION_TYPE, ptype)
        + ebml_master(ID_PROJECTION_PRIVATE, private)
    )
    # The official Matroska layout nests the pose floats in a ProjectionPose
    # master (0x7673) — which is what ExoPlayer/Media3 read.  Google's RFC
    # instead uses 0x7673/4/5 as flat floats, and ffmpeg follows the RFC, so it
    # rejects a master there.  Pose is zero in virtually every file, so omit the
    # element entirely in that case: both parsers then agree.  libvlcrs reads
    # either layout.
    if yaw or pitch or roll:
        if pose_layout == "flat":
            # Google RFC layout: 0x7673/4/5 are the yaw/pitch/roll floats
            proj += (
                ebml_float(ID_PROJECTION_POSE, yaw)
                + ebml_float(ID_POSE_YAW, pitch)
                + ebml_float(ID_POSE_PITCH, roll)
            )
        else:
            pose = (
                ebml_float(ID_POSE_YAW, yaw)
                + ebml_float(ID_POSE_PITCH, pitch)
                + ebml_float(ID_POSE_ROLL, roll)
            )
            proj += ebml_master(ID_PROJECTION_POSE, pose)
    out += ebml_master(ID_PROJECTION, proj)
    return out


def inject_mkv(data: bytes, payload: bytes, state: dict) -> bytes:
    """Insert `payload` at the end of the first video track's Video element."""
    out = bytearray()
    pos = 0
    while pos < len(data):
        rv = read_vint(data, pos, masked=False)
        if rv is None:
            break
        eid, id_len = rv
        rv2 = read_vint(data, pos + id_len, masked=True)
        if rv2 is None:
            break
        size, size_len = rv2
        if size_len < 0:  # unknown size
            out += data[pos:]
            break
        header = id_len + size_len
        start = pos + header
        end = start + size
        if end > len(data):
            out += data[pos:]
            break
        body = data[start:end]
        if state["done"]:
            out += data[pos:end]
            pos = end
            continue
        if eid == ID_VIDEO and state.get("is_video_track"):
            new_body = body + payload
            state["done"] = True
            out += write_id(eid) + write_vint_size(len(new_body)) + new_body
            pos = end
            continue
        if eid == ID_TRACK_ENTRY:
            # is this a video track?
            is_video = track_is_video(body)
            prev = state.get("is_video_track")
            state["is_video_track"] = is_video
            new_body = inject_mkv(body, payload, state)
            state["is_video_track"] = prev
            out += write_id(eid) + write_vint_size(len(new_body)) + new_body
            pos = end
            continue
        if eid in (ID_SEGMENT, ID_TRACKS):
            new_body = inject_mkv(body, payload, state)
            out += write_id(eid) + write_vint_size(len(new_body)) + new_body
            pos = end
            continue
        out += data[pos:end]
        pos = end
    out += data[pos:]
    return bytes(out)


def track_is_video(body: bytes) -> bool:
    pos = 0
    while pos < len(body):
        rv = read_vint(body, pos, masked=False)
        if rv is None:
            return False
        eid, id_len = rv
        rv2 = read_vint(body, pos + id_len, masked=True)
        if rv2 is None:
            return False
        size, size_len = rv2
        if size_len < 0:
            return False
        start = pos + id_len + size_len
        if eid == ID_TRACK_TYPE:
            value = int.from_bytes(body[start:start + size], "big")
            return value == 1
        pos = start + size
    return False


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #
STEREO_MODE_MP4 = {"mono": 0, "tb": 1, "sbs": 2, "rl": 4}
STEREO_MODE_MKV = {
    ("sbs", "left-first"): 1,
    ("sbs", "right-first"): 11,
    ("tb", "left-first"): 3,
    ("tb", "right-first"): 2,
    ("mono", "left-first"): 0,
}
STEREO_V1 = {"mono": "mono", "sbs": "left-right", "tb": "top-bottom", "rl": "left-right"}


def detect(data: bytes) -> str:
    if len(data) >= 12 and data[4:8] == b"ftyp":
        return "mp4"
    if data[:4] == bytes([0x1A, 0x45, 0xDF, 0xA3]):
        return "mkv"
    raise SystemExit("unrecognised container (neither MP4 nor Matroska)")


def main(argv: List[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("input")
    ap.add_argument("output")
    ap.add_argument("--coverage", choices=["360", "180"], default="360")
    ap.add_argument("--stereo", choices=["mono", "sbs", "tb", "rl"], default="mono")
    ap.add_argument("--eye-order", choices=["left-first", "right-first"], default="left-first")
    ap.add_argument("--yaw", type=float, default=0.0)
    ap.add_argument("--pitch", type=float, default=0.0)
    ap.add_argument("--roll", type=float, default=0.0)
    ap.add_argument("--no-v1", action="store_true", help="skip the Spherical V1 uuid box (MP4)")
    ap.add_argument("--pose-layout", choices=["nested", "flat"], default="nested",
                    help="Matroska ProjectionPose layout: 'nested' is the official "
                         "Matroska spec (ExoPlayer/Media3), 'flat' is Google's RFC "
                         "layout (ffmpeg). Omitted entirely when the pose is zero.")
    args = ap.parse_args(argv)

    with open(args.input, "rb") as f:
        data = f.read()
    kind = detect(data)

    # 180° is expressed as equirectangular bounds cropping 25% off each side
    crop = (0.0, 0.0, 0.25, 0.25) if args.coverage == "180" else (0.0, 0.0, 0.0, 0.0)

    if kind == "mp4":
        stereo_mode = STEREO_MODE_MP4[args.stereo]
        if args.stereo == "sbs" and args.eye_order == "right-first":
            stereo_mode = 4
        payload = spherical_children(stereo_mode, args.yaw, args.pitch, args.roll, crop)
        v1 = b"" if args.no_v1 else v1_uuid_box(STEREO_V1[args.stereo], "equirectangular", args.yaw)
        out = inject_mp4(data, payload, v1, {"done": False})
    else:
        key = (args.stereo if args.stereo != "rl" else "sbs", args.eye_order)
        stereo_mode = STEREO_MODE_MKV.get(key, 0) if args.stereo != "mono" else 0
        payload = mkv_projection(
            stereo_mode if args.stereo != "mono" else None,
            1,
            args.yaw,
            args.pitch,
            args.roll,
            crop,
            args.pose_layout,
        )
        out = inject_mkv(data, payload, {"done": False})

    if not out or len(out) == len(data):
        print("warning: no video track found, file copied unchanged", file=sys.stderr)
    with open(args.output, "wb") as f:
        f.write(out)
    print(f"{args.input} → {args.output}: {kind} coverage={args.coverage} "
          f"stereo={args.stereo} eye-order={args.eye_order} "
          f"pose=({args.yaw},{args.pitch},{args.roll}) "
          f"({len(data)} → {len(out)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
