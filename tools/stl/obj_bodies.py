#!/usr/bin/env python3
"""Split an OBJ by object and check each one is a closed, oriented mesh."""
import sys
from collections import Counter
verts, objs, cur = [], {}, None
for line in open(sys.argv[1]):
    p = line.split()
    if not p: continue
    if p[0] == 'v': verts.append(tuple(map(float, p[1:4])))
    elif p[0] in ('o', 'g'): cur = ' '.join(p[1:]); objs.setdefault(cur, [])
    elif p[0] == 'f': objs.setdefault(cur, []).append([int(x.split('/')[0]) - 1 for x in p[1:]])
for name, faces in objs.items():
    key = lambda i: tuple(round(c * 1e4) for c in verts[i])
    edges, vol = Counter(), 0.0
    for f in faces:
        k = [key(i) for i in f]
        if len(set(k)) < 3: continue
        for a, b in zip(k, k[1:] + k[:1]): edges[(a, b)] += 1
        a, b, c = (verts[i] for i in f[:3])
        vol += (a[0]*(b[1]*c[2]-b[2]*c[1]) - a[1]*(b[0]*c[2]-b[2]*c[0]) + a[2]*(b[0]*c[1]-b[1]*c[0])) / 6
    bad = sum(1 for (a, b), n in edges.items() if n != 1 or edges.get((b, a)) != 1)
    print(f"{name}: {len(faces)} faces, volume {vol:.1f}, bad edges {bad}")
