import sys, json, subprocess, importlib.util
name, frac = sys.argv[1], float(sys.argv[2])
sys.argv = ["bench.py"]  # no builds: only build the cases
src = open("bench.py", encoding="utf-8").read().split("# ---------------------------------------------------------------- run")[0]
ns = {}
exec(src, ns)
for kind, nm, d, facts in ns["cases"]:
    if nm == name:
        size = len(json.dumps(d, separators=(",", ":")))
        p = subprocess.run(["C:/Users/times/Repos/shrinkfit/target/release/shrinkfit.exe", "-n", str(max(600, int(size * frac))), "-i", "0", "--array-strategy", "condense", "--no-notice", "--no-save-full"], input=json.dumps(d).encode(), capture_output=True)
        out = json.loads(p.stdout.decode())
        print(json.dumps(out, ensure_ascii=False)[:3000] if len(sys.argv) < 3 else out)
        print("facts missing:", [f for f in facts if f not in json.dumps(out, ensure_ascii=False).replace("\n", "\n").replace('\\"', '"')])
