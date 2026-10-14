import json
import os
import random
import unittest

import shrinkfit
from shrinkfit import ShrinkError, shrink, shrink_with_report

try:
    import yaml
except ImportError:  # pragma: no cover
    yaml = None


def records(n):
    return {
        "status": "ok",
        "total": n,
        "results": [
            {"id": i, "name": f"user {i}", "bio": "Long biography. " * 30, "tags": ["a", "b"]}
            for i in range(n)
        ],
    }


class ShrinkTests(unittest.TestCase):
    def test_small_input_unchanged(self):
        r = shrink_with_report({"a": 1}, 100)
        self.assertEqual(r.text, '{\n  "a": 1\n}')  # indented by default
        self.assertEqual(shrink({"a": 1}, indent=0), '{"a":1}')  # max_chars optional
        self.assertFalse(r.report["pruned"])

    def test_docstring_example(self):
        out = shrink({"items": list(range(1000)), "total": 1000}, max_chars=80)
        self.assertLessEqual(len(out), 80)
        self.assertEqual(json.loads(out)["total"], 1000)
        self.assertIn("items omitted", out)

    def test_objects_strings_and_bytes_inputs_agree(self):
        data = records(200)
        as_text = json.dumps(data)
        a = shrink(data, 2000)
        b = shrink(as_text, 2000)
        c = shrink(as_text.encode(), 2000)
        self.assertEqual(a, b)
        self.assertEqual(b, c)

    def test_budget_respected_and_valid_json(self):
        rng = random.Random(7)
        data = records(500)
        for _ in range(60):
            budget = rng.randint(60, 20000)
            r = shrink_with_report(data, budget)
            self.assertTrue(r.report["fits"])
            self.assertLessEqual(len(r.text), budget)
            parsed = json.loads(r.text)
            self.assertEqual(parsed["status"], "ok")
            self.assertEqual(parsed["total"], 500)

    def test_pretty_indent(self):
        out = shrink(records(50), 3000, indent=2)
        self.assertLessEqual(len(out), 3000)
        self.assertIn('\n  "status": "ok"', out)

    @unittest.skipIf(yaml is None, "PyYAML not installed")
    def test_yaml_output_round_trips(self):
        rng = random.Random(3)
        data = records(300)
        data["notes"] = "\n".join(f"line {i}: something happened" for i in range(400))
        data["weird"] = ["true", "null", "1.5", "- dash", "a: b", "  lead", "tab\there", " ", ""]
        for _ in range(40):
            budget = rng.randint(200, 15000)
            r = shrink_with_report(data, budget, format="yaml")
            self.assertLessEqual(len(r.text), budget, r.report)
            parsed = yaml.safe_load(r.text)
            self.assertEqual(parsed["status"], "ok")
            self.assertEqual(parsed["total"], 300)
            # Ambiguous strings must survive as strings, never become bools/nulls/numbers.
            if "weird" in parsed and isinstance(parsed["weird"], list):
                for v in parsed["weird"]:
                    self.assertIsInstance(v, str)

    @unittest.skipIf(yaml is None, "PyYAML not installed")
    def test_yaml_full_fidelity_when_it_fits(self):
        data = {"a": [1, 2, {"b": "multi\nline\n"}], "c": "x: y", "d": None, "e": "", "f": "\ttab"}
        out = shrink(data, 10_000, format="yaml")
        self.assertEqual(yaml.safe_load(out), data)

    def test_yaml_input(self):
        src = "items:\n" + "".join(f"  - id: {i}\n    name: n{i}\n" for i in range(300))
        out = shrink(src, 500, input_format="yaml", format="yaml")
        self.assertLessEqual(len(out), 500)
        self.assertIn("items omitted", out)

    def test_yaml_input_errors_and_json_output(self):
        src = "a: [1, 2, 3]\nb: hello\n"
        # Auto-detected YAML in -> YAML out; an explicit format overrides.
        self.assertEqual(shrink(src), "a:\n  - 1\n  - 2\n  - 3\nb: hello\n")
        out = shrink(src, 100, input_format="yaml", format="json")
        self.assertEqual(json.loads(out), {"a": [1, 2, 3], "b": "hello"})
        with self.assertRaises(ShrinkError):
            shrink("a: [1", 100, input_format="yaml")
        with self.assertRaises(ShrinkError):
            shrink("{}", 100, input_format="toml")

    def test_condense_strategy_keeps_every_record(self):
        rows = [
            {"id": str(i), "title": f"Row {i}", "avatar_url": "https://x/a.png", "etag": "abc" * 20}
            for i in range(300)
        ]
        r = shrink_with_report(rows, 6000, array_strategy="condense", indent=0)
        out = json.loads(r.text)
        self.assertEqual(len(out), 300)  # every record kept, no gap markers
        self.assertTrue(all(set(x) == set(out[0]) for x in out))  # same fields in each
        self.assertIn("title", out[0])
        self.assertNotIn("avatar_url", out[0])
        self.assertGreater(r.report["fields_dropped"], 0)
        with self.assertRaises(ShrinkError):
            shrink(rows, 500, array_strategy="nope")

    def test_arguments_option_is_accepted(self):
        rows = [{"id": str(i), "title": f"Row {i}", "amount": 1000 + i * 37, "note": "x" * 30} for i in range(120)]
        r = shrink_with_report(rows, 2500, array_strategy="condense", indent=0, arguments={"soql": "SELECT amount FROM deals"})
        self.assertIn("amount", json.loads(r.text)[0])

    def test_protect_keeps_identifiers_exact(self):
        rows = [
            {"ari": "ari:cloud:loom::video/%032x" % (i * 7919), "name": f"Video number {i} about onboarding", "views": i}
            for i in range(80)
        ]
        r = shrink_with_report(rows, 3000, array_strategy="condense", indent=0, protect=["ari"])
        out = json.loads(r.text)
        self.assertTrue(r.report["fits"])
        records = [x for x in out if isinstance(x, dict)]  # the rest are omission markers
        self.assertGreater(len(records), 5)
        for row in records:
            self.assertEqual(len(row["ari"]), len("ari:cloud:loom::video/") + 32)  # never shortened
        with self.assertRaises(ShrinkError):
            shrink(rows, 500, protect=[1])

    def test_field_scores_choose_columns(self):
        rows = [{"id": f"ID-{i}", "alpha": f"alpha text {i} " * 6, "beta": f"beta text {i} " * 6} for i in range(30)]
        keep_beta = {"results/*/beta": 1.0, "results/*/alpha": 0.0}
        r = shrink_with_report({"results": rows}, 1500, array_strategy="condense", indent=0, field_scores=keep_beta, field_score_weight=1)
        first = json.loads(r.text)["results"][0]
        self.assertIn("beta", first)
        self.assertNotIn("alpha", first)
        with self.assertRaises(ShrinkError):
            shrink(rows, 500, field_scores={"a": "high"})

    def test_model_scoring_runs_in_the_core_and_falls_back(self):
        import http.server
        import threading

        seen = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                seen.append((self.path, self.headers.get("Authorization")))
                answers = {
                    name: {"type": "noul", "noul": 1.0 if q["instructions"].split("`")[1].endswith("beta") else 0.0}
                    for name, q in body["questions"].items()
                }
                out = json.dumps({"answers": answers}).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(out)))
                self.end_headers()
                self.wfile.write(out)

            def log_message(self, *args):
                pass

        server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.shutdown)
        rows = [{"id": f"ID-{i}", "alpha": f"alpha text {i} " * 6, "beta": f"beta text {i} " * 6} for i in range(30)]
        data = {"results": rows}
        base = f"http://127.0.0.1:{server.server_port}/api"
        r = shrink_with_report(
            data, 1500, array_strategy="condense", indent=0, field_score_weight=1,
            jev={"api_key": "test-key", "base_url": base, "tool": "search"},
        )
        self.assertEqual(list(json.loads(r.text)["results"][0]), ["beta"])
        self.assertTrue(r.report["model_scoring"].startswith("scored 3 fields"))
        self.assertEqual(seen, [("/api/alpha/decisions", "Bearer test-key")])
        # No key: nothing is sent and the built-in scoring is used.
        os.environ.pop("OPENROUTER_API_KEY", None)
        r = shrink_with_report(data, 1500, array_strategy="condense", indent=0, jev={"base_url": base})
        self.assertIn("no OpenRouter API key", r.report["model_scoring"])
        self.assertEqual(len(seen), 1)

    def test_string_chunking_option(self):
        doc = {"readme": "\n\n".join(f"## Part {i}\n" + "word " * 80 for i in range(20))}
        out = json.loads(shrink(doc, 2500, string_section_chars=40))
        for i in range(20):
            self.assertIn(f"## Part {i}", out["readme"])

    def test_errors(self):
        with self.assertRaises(ShrinkError):
            shrink("{not json", 100)
        with self.assertRaises(ShrinkError):
            shrink({"a": 1}, 100, bogus_option=1)
        with self.assertRaises(ShrinkError):
            shrink({"a": 1}, 100, format="xml")

    def test_unicode(self):
        data = {"t": "héllo wörld 😀 " * 500}
        out = shrink(data, 300)
        self.assertLessEqual(len(out), 300)
        self.assertTrue(json.loads(out)["t"].startswith("héllo"))

    def test_version(self):
        self.assertEqual(shrinkfit.native_version(), shrinkfit.__version__)


if __name__ == "__main__":
    unittest.main()
