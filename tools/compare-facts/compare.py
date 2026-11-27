"""Compare two {path: facts} JSON documents (illuminate's and nonio's); print the files whose facts differ.

python3 compare.py illuminate.json nonio.json
"""
import json
import sys

a = json.load(open(sys.argv[1]))
b = json.load(open(sys.argv[2]))


def norm(f):
    f = dict(f)
    f.setdefault("reads", [])
    f.setdefault("tables", [])
    return f


same = 0
diff = []
for path in sorted(set(a) | set(b)):
    if path not in a or path not in b:
        diff.append((path, "only in " + ("illuminate" if path in a else "nonio"), None))
        continue
    fa, fb = norm(a[path]), norm(b[path])
    if fa == fb:
        same += 1
        continue
    keys = [k for k in sorted(set(fa) | set(fb)) if fa.get(k) != fb.get(k)]
    diff.append((path, ", ".join(keys), (fa, fb)))
print(f"{same} of {len(set(a) | set(b))} files give the same facts")
for path, what, pair in diff:
    print(f"DIFF {path}: {what}")
    if pair and "-v" in sys.argv:
        fa, fb = pair
        for k in what.split(", "):
            print("  illuminate", k, json.dumps(fa.get(k))[:600])
            print("  nonio     ", k, json.dumps(fb.get(k))[:600])
sys.exit(1 if diff else 0)
