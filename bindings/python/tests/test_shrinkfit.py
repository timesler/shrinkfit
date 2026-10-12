import json
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
