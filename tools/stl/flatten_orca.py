#!/usr/bin/env python3
"""Resolve an OrcaSlicer system preset's `inherits` chain into one full JSON
(the CLI doesn't), e.g. flatten_orca.py PROFILES/BBL machine "Bambu Lab X1 Carbon 0.4 nozzle" out.json"""
import json, os, sys
root, kind, name, out = sys.argv[1:5]
def load(n):
    for d in (os.path.join(root, kind), os.path.join(root, '..', 'OrcaFilamentLibrary', kind)):
        for dirpath, _, files in os.walk(d):
            for f in files:
                if f.endswith('.json'):
                    p = os.path.join(dirpath, f)
                    try:
                        j = json.load(open(p))
                    except Exception:
                        continue
                    if j.get('name') == n:
                        return j
    raise SystemExit(f"no {kind} preset named {n!r}")
chain, n = [], name
while n:
    j = load(n); chain.append(j); n = j.get('inherits')
full = {}
for j in reversed(chain):
    full.update(j)
full.pop('inherits', None)
full['name'] = name
full.setdefault('from', 'system')
json.dump(full, open(out, 'w'), indent=1)
print(f"{name}: {len(chain)} levels, {len(full)} keys")
