"""Compares a dist directory with a published release's assets.

Every file must be identical, except that the fctl binary in a macOS archive
may differ in LC_UUID and in its code signature (which hashes the page that
holds the UUID): zig derives the UUID from linker output that includes the
LTO object's mtime, so no two macOS builds share it.
"""

import hashlib
import pathlib
import struct
import sys
import tarfile

LC_UUID, LC_CODE_SIGNATURE = 0x1B, 0x1D


def masked(data):
    b = bytearray(data)
    if struct.unpack_from("<I", b, 0)[0] != 0xFEEDFACF:
        raise SystemExit("not a 64-bit Mach-O")
    ncmds = struct.unpack_from("<I", b, 16)[0]
    off, uuid = 32, None
    for _ in range(ncmds):
        cmd, size = struct.unpack_from("<II", b, off)
        if cmd == LC_UUID:
            uuid = bytes(b[off + 8 : off + 24]).hex()
            b[off + 8 : off + 24] = bytes(16)
        elif cmd == LC_CODE_SIGNATURE:
            dataoff, datasize = struct.unpack_from("<II", b, off + 8)
            b[dataoff : dataoff + datasize] = bytes(datasize)
        off += size
    return bytes(b), uuid


def members(path):
    with tarfile.open(path) as t:
        return {m.name: t.extractfile(m).read() for m in t.getmembers()}


def assets(d):
    derived = ("SHA256SUMS", "manifest.json")
    return sorted(p.name for p in d.iterdir() if not p.name.endswith(".sha256") and p.name not in derived)


old, new = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
names = assets(old)
if names != assets(new):
    raise SystemExit(f"different files: {names} and {assets(new)}")
ok = True
for name in names:
    a, b = (old / name).read_bytes(), (new / name).read_bytes()
    if a == b:
        print(f"{name}: identical (sha256 {hashlib.sha256(a).hexdigest()[:16]})")
        continue
    if "apple-darwin" not in name:
        print(f"{name}: DIFFERENT")
        ok = False
        continue
    ma, mb = members(old / name), members(new / name)
    if ma.keys() != mb.keys():
        raise SystemExit(f"{name}: different members")
    for m in sorted(ma):
        if ma[m] == mb[m]:
            print(f"{name} {m}: identical")
        elif m == "fctl":
            (xa, ua), (xb, ub) = masked(ma[m]), masked(mb[m])
            diff = sum(x != y for x, y in zip(ma[m], mb[m]))
            same = len(ma[m]) == len(mb[m]) and xa == xb
            verdict = "identical except" if same else "DIFFERENT beyond"
            print(f"{name} {m}: {verdict} LC_UUID and signature ({diff} bytes; uuid {ua} -> {ub})")
            ok &= same
        else:
            print(f"{name} {m}: DIFFERENT")
            ok = False
sys.exit(0 if ok else 1)
