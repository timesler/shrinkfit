"""Retention benchmark for long strings that are not prose: logs, command output, HTML, XML, JSON and rich text stored as
text, diffs, blobs. Each case is a small JSON document whose long string carries facts a reader would need (an error
line, a link, a value deep in a list); the score is the share of facts still present after shrinking to a budget.

usage: python bench.py NAME=PATH_TO_CLI [NAME=PATH ...]     (run from experiments/strings)
The cases are synthetic and seeded, written to resemble real outputs. They show whether the facts survive, not how
often such strings occur (see experiments/path-selection for that).
"""
import base64
import json
import random
import subprocess
import sys

R = random.Random(7)
WORDS = "service request cache queue latency invoice billing export search login webhook retry sync upload storage index".split()


def sentence(n=9):
    return " ".join(R.choice(WORDS) for _ in range(n)).capitalize() + "."


def doc(payload, **extra):
    d = {"title": "Result for job 4471", "status": "failed", "meta": {"site": "demo", "count": 1}}
    d.update(extra)
    d["payload"] = payload
    return d


cases = []  # (kind, name, document, facts)

# ---- logs and command output
lines = [f"   Compiling crate-{i} v0.{i}.{i % 9}" for i in range(300)]
lines[120:120] = ["warning: unused variable: `tmp`", "  --> src/util.rs:17:9"]
lines += ["error[E0308]: mismatched types", "  --> src/main.rs:42:5", "   |", '42 |     let n: u32 = "seven";', "   |            ---   ^^^^^^^ expected `u32`, found `&str`",
          'error: could not compile `app` (bin "app") due to 1 previous error', "test result: FAILED. 41 passed; 2 failed; 0 ignored"]
cases.append(("log", "cargo build", doc("\n".join(lines)), ["error[E0308]: mismatched types", "src/main.rs:42:5", "2 failed", "could not compile `app`"]))

lines = ["============================= test session starts ============================="] + [f"tests/test_api.py::test_case_{i} PASSED" for i in range(220)]
lines += ["tests/test_api.py::test_login FAILED", "=================================== FAILURES ===================================", "_________________________________ test_login __________________________________",
          "    def test_login(client):", ">       assert client.post('/login').status_code == 200", "E       AssertionError: assert 500 == 200", "tests/test_api.py:88: AssertionError",
          "=========================== short test summary info ============================", "FAILED tests/test_api.py::test_login - AssertionError: assert 500 == 200", "======================= 1 failed, 220 passed in 12.31s ========================"]
cases.append(("log", "pytest", doc("\n".join(lines)), ["AssertionError: assert 500 == 200", "tests/test_api.py:88", "1 failed, 220 passed", "FAILED tests/test_api.py::test_login"]))

lines = [f"2026-10-16T05:{i // 60:02d}:{i % 60:02d}Z INFO  worker-{i % 7} processed batch {i} in {10 + i % 13}ms" for i in range(400)]
lines[250] = "2026-10-16T05:04:10Z FATAL worker-3 connection pool exhausted: 0/32 connections available (db=orders-primary)"
lines[251:251] = ["Traceback (most recent call last):", '  File "app/db.py", line 88, in acquire', "    raise PoolExhausted(name)", "PoolExhausted: orders-primary"]
cases.append(("log", "app log + traceback", doc("\n".join(lines)), ["FATAL worker-3", "pool exhausted", "app/db.py", "PoolExhausted: orders-primary"]))

lines = ["\x1b[1m\x1b[32m   Downloading\x1b[0m deps"] + [f"  % Total    % Received\r{i}%  {i * 1000}  {i}M\r" for i in range(1, 100)]
lines += [f"\x1b[33mwarn\x1b[0m: package-{i} is deprecated" for i in range(30)]
lines += ["scp: /srv/data/export.csv: Permission denied", "Connection closed by 10.4.7.2 port 22", "Exit code: 1"]
cases.append(("shell", "ANSI + progress + exit", doc("\n".join(lines)), ["Permission denied", "10.4.7.2 port 22", "Exit code: 1"]))

lines = [f"npm WARN deprecated pkg-{i}@1.{i}.0: no longer maintained" for i in range(120)]
lines += ["npm ERR! code ERESOLVE", "npm ERR! ERESOLVE unable to resolve dependency tree", "npm ERR! Found: react@18.2.0", 'npm ERR! peer react@"^17.0.0" from old-widget@2.1.0']
cases.append(("shell", "npm error", doc("\n".join(lines)), ["ERESOLVE unable to resolve", "react@18.2.0", "old-widget@2.1.0"]))

# ---- diff
d = []
files = [f"src/module_{i}.rs" for i in range(14)]
for k, f in enumerate(files):
    d += [f"diff --git a/{f} b/{f}", "index 83db48f..bf2a3c1 100644", f"--- a/{f}", f"+++ b/{f}"]
    for h in range(3):
        d.append(f"@@ -{h * 40 + 1},9 +{h * 40 + 1},9 @@ impl Thing{k}")
        d += [f" // context {sentence(5)}" for _ in range(3)]
        d += [f"-    let value_{k}_{h} = old_call({h});", f"+    let value_{k}_{h} = new_call({h}, true);"]
        d += [f" // context {sentence(5)}" for _ in range(3)]
cases.append(("diff", "14-file diff", doc("\n".join(d)), ["src/module_9.rs", "src/module_13.rs", "+    let value_11_1 = new_call(1, true);", "@@ -41,9 +41,9 @@ impl Thing12"]))

# ---- HTML
paras = [f'<p class="c-text"><span>{sentence(14)}</span></p>' for _ in range(60)]
paras[14] = '<p>The <b>Zeta-9 protocol</b> was adopted on <a href="https://example.org/zeta9" class="link">the standards page</a> in March.</p>'
paras[33] = '<p>Rollback requires the <code>--force-epoch</code> flag; see <a href="https://example.org/rollback">rollback guide</a>.</p>'
paras[50] = '<p>Contact <a href="mailto:ops@example.org">ops@example.org</a> for the quarantine window: 72 hours.</p>'
html = '<html><head><title>Release notes</title><style>.c-text{color:#333}</style><script>var a=1;</script></head><body><div class="wrap"><h1>Release notes</h1>' + "".join(paras) + "</div></body></html>"
cases.append(("html", "article", doc(html), ["Zeta-9 protocol", "https://example.org/zeta9", "--force-epoch", "https://example.org/rollback", "quarantine window: 72 hours"]))

rows_html = "".join(f'<tr class="r"><td>SKU-{i:04d}</td><td>{sentence(3)}</td><td>${R.randint(5, 900)}.00</td><td>{R.choice(["in stock", "backorder", "discontinued"])}</td></tr>' for i in range(80))
rows_html = rows_html.replace("SKU-0042", "SKU-0042 (NEW)", 1)
cases.append(("html", "product table", doc("<table><thead><tr><th>SKU</th><th>Name</th><th>Price</th><th>Stock</th></tr></thead><tbody>" + rows_html + "</tbody></table>"), ["SKU-0042 (NEW)", "SKU-0070", "SKU-0001"]))

cases.append(("html", "search snippet", doc("".join(f'<span class="searchmatch">{w}</span> {sentence(6)} ' for w in ["Rust", "borrow", "checker", "lifetime", "trait", "macro", "async", "cargo", "crate", "unsafe"] * 4) + "and the Hindley–Milner tail"), ["Hindley–Milner tail", "checker", "lifetime"]))

# ---- XML
items = "".join(f'<Item id="{i}"><Sku>SKU-{i:04d}</Sku><Name>{sentence(3)}</Name><Qty>{R.randint(1, 50)}</Qty></Item>' for i in range(90))
items = items.replace("<Sku>SKU-0063</Sku>", "<Sku>SKU-0063</Sku><Flag>RECALLED</Flag>")
soap = f'<?xml version="1.0"?><soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"><soap:Body><GetStockResponse><Warehouse>Rotterdam-2</Warehouse><Items>{items}</Items></GetStockResponse></soap:Body></soap:Envelope>'
cases.append(("xml", "SOAP stock", doc(soap), ["Rotterdam-2", "RECALLED", "SKU-0063", "SKU-0089"]))

rss = '<?xml version="1.0"?><rss version="2.0"><channel><title>Ops feed</title>' + "".join(f"<item><title>Update {i}: {sentence(4)}</title><link>https://feed.example/p/{i}</link><pubDate>2026-10-{1 + i % 28:02d}</pubDate></item>" for i in range(70)) + "</channel></rss>"
cases.append(("xml", "RSS feed", doc(rss), ["Ops feed", "https://feed.example/p/35", "https://feed.example/p/69"]))

# ---- JSON stored as text
recs = [{"id": 1000 + i, "name": f"{R.choice(WORDS)}-{i}", "owner": {"login": f"user{i}", "url": f"https://api.example/users/{i}"}, "labels": [R.choice(WORDS)], "created": f"2026-01-{1 + i % 28:02d}T10:00:00Z"} for i in range(90)]
recs[61]["name"] = "payments-gateway"
body = {"results": recs, "error": None, "next_cursor": "cur_8F3kQ"}
cases.append(("json-text", "compact JSON string", doc(json.dumps(body, separators=(",", ":"))), ["payments-gateway", "cur_8F3kQ", "user89", "1000"]))
cases.append(("json-text", "pretty JSON string", doc(json.dumps(body, indent=2)), ["payments-gateway", "cur_8F3kQ", "user89", "1000"]))


def adf(n=40):
    c = [{"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Incident review"}]}]
    for i in range(n):
        c.append({"type": "paragraph", "content": [{"type": "text", "text": sentence(16)}]})
    c[13] = {"type": "paragraph", "content": [{"type": "text", "text": "Root cause was the "}, {"type": "text", "text": "stale TLS certificate", "marks": [{"type": "strong"}]}, {"type": "text", "text": " on the edge proxy."}]}
    c[27] = {"type": "paragraph", "content": [{"type": "mention", "attrs": {"id": "5b1", "text": "@Priya Nair"}}, {"type": "text", "text": " owns the follow-up, tracked in "}, {"type": "text", "text": "the runbook", "marks": [{"type": "link", "attrs": {"href": "https://wiki.example/runbook/tls"}}]}]}
    c[34] = {"type": "bulletList", "content": [{"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Rotate certificates every 60 days"}]}]}]}
    return {"type": "doc", "version": 1, "content": c}


facts_adf = ["Incident review", "stale TLS certificate", "@Priya Nair", "https://wiki.example/runbook/tls", "Rotate certificates every 60 days"]
cases.append(("adf", "ADF object", doc(adf(), description=None), facts_adf))
cases.append(("adf", "ADF as JSON text", doc(json.dumps(adf(), separators=(",", ":"))), facts_adf))

# ---- blob
img = base64.b64encode(bytes(R.getrandbits(8) for _ in range(30000))).decode()
d = {"title": "Search results", "items": [{"name": "Rust logo", "caption": "Official logo", "thumbnail": img}, {"name": "Ferris", "caption": "The mascot", "thumbnail": img[:9000]}, {"name": "Cargo", "caption": "Package manager", "thumbnail": img[:5000]}]}
cases.append(("blob", "inline images", d, ["Rust logo", "Official logo", "Ferris", "The mascot", "Cargo", "Package manager"]))

# ---------------------------------------------------------------- run
builds = dict(a.split("=", 1) for a in sys.argv[1:])
FRACS = (0.08, 0.15, 0.30)


def texts(o):
    if isinstance(o, dict):
        for k, v in o.items():
            yield k
            yield from texts(v)
    elif isinstance(o, list):
        for v in o:
            yield from texts(v)
    elif isinstance(o, str):
        yield o


def run(cli, d, n):
    p = subprocess.run([cli, "-n", str(n), "-i", "0", "--array-strategy", "condense", "--no-notice", "--no-save-full"], input=json.dumps(d).encode(), capture_output=True)
    return json.loads(p.stdout.decode("utf-8", "replace"))


rows = []
for kind, name, d, facts in cases:
    size = len(json.dumps(d, separators=(",", ":")))
    row = [kind, name, size]
    for b, cli in builds.items():
        for f in FRACS:
            out = run(cli, d, max(600, int(size * f)))
            blob = "\n".join(texts(out))
            row.append(sum(x in blob for x in facts) / len(facts))
    rows.append(row)
names = list(builds)
print(f"{'kind':10s}{'case':24s}{'chars':>8s}  " + "  ".join(f"{b[:6]:>6s}@{int(f * 100):>2d}%" for b in names for f in FRACS))
for r in rows:
    print(f"{r[0]:10s}{r[1]:24s}{r[2]:8d}  " + "  ".join(f"{x * 100:9.0f}%" for x in r[3:]))
print()
for i, b in enumerate(names):
    for j, f in enumerate(FRACS):
        col = [r[3 + i * len(FRACS) + j] for r in rows]
        print(f"{b:10s} budget {int(f * 100):2d}%: mean facts kept {sum(col) / len(col) * 100:5.1f}%")
