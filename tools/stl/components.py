#!/usr/bin/env python3
"""Per connected component of an STL: triangles, bbox, open edges."""
import struct, sys
from collections import defaultdict
data = open(sys.argv[1], 'rb').read(); n = struct.unpack_from('<I', data, 80)[0]
tol = float(sys.argv[2]) if len(sys.argv) > 2 else 1e-4
ids, pos, tris = {}, [], []
for t in range(n):
    f = struct.unpack_from('<12f', data, 84 + 50*t)
    k = []
    for v in (f[3:6], f[6:9], f[9:12]):
        q = tuple(round(c / tol) for c in v)
        if q not in ids: ids[q] = len(ids); pos.append(v)
        k.append(ids[q])
    if len(set(k)) == 3: tris.append(k)
parent = list(range(len(ids)))
def find(x):
    while parent[x] != x: parent[x] = parent[parent[x]]; x = parent[x]
    return x
for a, b, c in tris: parent[find(a)] = find(b); parent[find(b)] = find(c)
edges = defaultdict(int)
for a, b, c in tris:
    for e in ((a, b), (b, c), (c, a)): edges[e] += 1
comp = defaultdict(lambda: [0, 0, [1e9]*3, [-1e9]*3])
for a, b, c in tris:
    r = comp[find(a)]; r[0] += 1
    for v in (a, b, c):
        for i in range(3): r[2][i] = min(r[2][i], pos[v][i]); r[3][i] = max(r[3][i], pos[v][i])
for (a, b), cnt in edges.items():
    if edges.get((b, a), 0) != cnt: comp[find(a)][1] += 1
for r in sorted(comp.values(), key=lambda r: -r[1]):
    print(f"tris {r[0]:6} open {r[1]:4} min {[round(x,1) for x in r[2]]} max {[round(x,1) for x in r[3]]}")
