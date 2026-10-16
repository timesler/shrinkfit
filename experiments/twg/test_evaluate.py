import unittest
from evaluate import metrics, retained, alternatives


class MetricsTests(unittest.TestCase):
    def test_either_id_or_key_is_valid_but_missing_identity_is_not(self):
        source = {'data': [{'key': 'DEMO-1', 'id': '1', 'name': 'Title', 'state': {'label': 'On track', 'value': 'on_track'}}]}
        contract = ('data', 'key', ['name', 'state/label'])
        output = {'data': [{'id': '1', 'name': 'Title', 'state': {'value': 'on_track'}}]}
        result = metrics(source, output, contract)
        self.assertEqual(result['task_value_recall'], 1)
        self.assertEqual(result['usable_record_recall'], 1)
        self.assertEqual(metrics(source, {'data': [{'name': 'Title'}]}, contract)['task_value_recall'], 0)

    def test_fabricated_values_do_not_count(self):
        source = {'data': [{'key': 'DEMO-1', 'name': 'Original'}]}
        output = {'data': [{'key': 'DEMO-1', 'name': 'Invented'}]}
        self.assertEqual(metrics(source, output, ('data', 'key', ['name']))['task_value_recall'], 0)

    def test_partial_content_gets_only_fractional_credit(self):
        self.assertEqual(retained('abcdefghij', 'abc…[+7 chars]'), 0.3)
        self.assertEqual(retained('abcdefghij', 'xyz…'), 0)
        self.assertEqual(retained('abcdefghij', '…'), 0)
        self.assertIsNone(retained(None, None))
        self.assertEqual(retained(False, False), 1)

    def test_markers_and_absent_arrays_are_safe(self):
        source = {'data': [{'key': 'DEMO-1', 'name': 'Original'}]}
        for output in [{}, {'data': ['…[1 items omitted]']}, {'data': '…'}]:
            self.assertEqual(metrics(source, output, ('data', 'key', ['name']))['identity_recall'], 0)

    def test_equivalent_paths(self):
        self.assertEqual(alternatives('state/label'), ['state/label', 'state/value'])
        self.assertEqual(alternatives('title'), ['title', 'content/title'])


if __name__ == '__main__':
    unittest.main()
