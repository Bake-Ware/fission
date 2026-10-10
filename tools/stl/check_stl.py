#!/usr/bin/env python3
"""Check a binary STL is printable: closed, consistently oriented, outward."""
import struct, sys
from collections import Counter

def check(path, tol=1e-4):
    data = open(path, 'rb').read()
    n = struct.unpack_from('<I', data, 80)[0]
    assert len(data) == 84 + 50 * n, f"size {len(data)} != {84 + 50*n}"
    key = lambda v: tuple(round(c / tol) for c in v)
    ids, edges, vol, degen = {}, Counter(), 0.0, 0
    lo, hi = [1e30]*3, [-1e30]*3
    bad_normal = 0
    for t in range(n):
        f = struct.unpack_from('<12f', data, 84 + 50*t)
        nrm, tri = f[0:3], [f[3:6], f[6:9], f[9:12]]
        for v in tri:
            for i in range(3):
                lo[i], hi[i] = min(lo[i], v[i]), max(hi[i], v[i])
        k = [ids.setdefault(key(v), len(ids)) for v in tri]
        if len(set(k)) < 3:
            degen += 1
            continue
        for a, b in ((k[0], k[1]), (k[1], k[2]), (k[2], k[0])):
            edges[(a, b)] += 1
        a, b, c = tri
        vol += (a[0]*(b[1]*c[2]-b[2]*c[1]) - a[1]*(b[0]*c[2]-b[2]*c[0]) + a[2]*(b[0]*c[1]-b[1]*c[0])) / 6
        u = [b[i]-a[i] for i in range(3)]; w = [c[i]-a[i] for i in range(3)]
        cr = [u[1]*w[2]-u[2]*w[1], u[2]*w[0]-u[0]*w[2], u[0]*w[1]-u[1]*w[0]]
        if sum(cr[i]*nrm[i] for i in range(3)) < 0:
            bad_normal += 1
    open_edges = sum(1 for (a, b), c in edges.items() if edges.get((b, a), 0) != c)
    dup = sum(1 for e, c in edges.items() if c > 1)
    ok = open_edges == 0 and dup == 0 and vol > 0 and bad_normal == 0
    print(f"{path}: {n} triangles, {len(ids)} vertices, volume {vol:.1f} mm³, "
          f"bbox {[round(hi[i]-lo[i], 2) for i in range(3)]} mm, open/flipped edges {open_edges}, "
          f"non-manifold {dup}, degenerate {degen}, normals disagreeing {bad_normal} -> {'OK' if ok else 'PROBLEM'}")
    return ok

if __name__ == '__main__':
    sys.exit(0 if all([check(p) for p in sys.argv[1:]]) else 1)
