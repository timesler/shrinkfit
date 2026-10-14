import json, collections, statistics
anch = set(json.load(open("data/lab_anchors.json"))["anchors"])
lab = collections.defaultdict(list); batch_rate = {}
for k in range(12):
    out = json.load(open(f"data/labels_raw/batch_{k}.json")); n2 = n = 0
    for cid, d in out.items():
        lab[int(cid)].append({int(i): v for i, v in d.items()}); 
        if int(cid) not in anch: n2 += sum(v == 2 for v in d.values()); n += len(d)
    batch_rate[k] = n2 / max(n, 1)
print("share of paths labelled essential (2), per batch:", {k: f"{v:.0%}" for k, v in batch_rate.items()})
# agreement on anchors
pairs = [(lab[c][0], lab[c][1]) for c in anch if len(lab[c]) == 2]
def kappa(a, b):
    n = len(a); po = sum(x == y for x, y in zip(a, b)) / n
    pe = sum((a.count(v) / n) * (b.count(v) / n) for v in set(a) | set(b)); return (po - pe) / (1 - pe) if pe < 1 else 1
A = []; B = []
for x, y in pairs:
    for i in x: A.append(x[i]); B.append(y[i])
print(f"anchors: {len(pairs)} cases, {len(A)} path pairs")
print(f"  exact 3-level agreement {sum(a == b for a, b in zip(A, B))/len(A):.0%}, kappa {kappa(A, B):.2f}")
for name, f in (("essential (2) vs not", lambda v: v == 2), ("keep (>=1) vs drop", lambda v: v >= 1)):
    a = [f(v) for v in A]; b = [f(v) for v in B]; print(f"  {name}: agreement {sum(x == y for x, y in zip(a, b))/len(a):.0%}, kappa {kappa(a, b):.2f}")
cnt = collections.Counter(zip(A, B)); print("  confusion (labeller1, labeller2):", {k: v for k, v in sorted(cnt.items())})
# final labels = mean over labellers
final = {c: {i: statistics.mean(l[i] for l in ls) for i in ls[0]} for c, ls in lab.items()}
json.dump({str(c): {str(i): v for i, v in d.items()} for c, d in final.items()}, open("data/lab_labels.json", "w"))
tot = sum(len(d) for d in final.values()); print(f"final: {len(final)} cases, {tot} paths; essential(mean>=1.5) {sum(v>=1.5 for d in final.values() for v in d.values())/tot:.0%}, keep(mean>=1) {sum(v>=1 for d in final.values() for v in d.values())/tot:.0%}")
