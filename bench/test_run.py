"""Scoring regressions from real model formatting and incomplete workflows."""
import json
from pathlib import Path
import tempfile
import unittest

from run import rescore, review_verdict, score


class Scoring(unittest.TestCase):
    def test_markdown_and_last_verdict_match_the_application(self):
        for text, expected in [
            ('## VERDICT: PASS', 'PASS'),
            ('**VERDICT: FAIL**', 'FAIL'),
            ('> `VERDICT: PASS`', 'PASS'),
            ('VERDICT: FAIL\nAfter checking:\n## VERDICT: PASS', 'PASS'),
            ('VERDICT: PASS\nVERDICT: UNVERIFIED', 'FAIL'),
            ('Looks fine; no explicit verdict.', None),
        ]:
            self.assertEqual(review_verdict(text), expected)

    def test_saved_events_correct_false_pass_without_changing_cost_or_original(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            events = root / 'fixture/run'
            events.mkdir(parents=True)
            (events / 'review.ndjson').write_text(json.dumps({'kind':'token', 'text':'## VERDICT: PASS'}) + '\n')
            original = json.dumps({'total_usd':0.02, 'tasks':{'fixture':{'run':{
                'usd':0.02, 'phases':{'review':{'exit':0, 'verdict':None}},
                'acceptance':[{'exit':1}], 'false_review_pass':False}}}})
            path = root / 'report.json'
            path.write_text(original)
            rescore(path)
            corrected = json.loads((root / 'report-rescored.json').read_text())
            self.assertTrue(corrected['tasks']['fixture']['run']['false_review_pass'])
            self.assertEqual(corrected['total_usd'], 0.02)
            self.assertEqual(path.read_text(), original)

    def test_passing_hidden_tests_does_not_mean_all_hats_succeeded(self):
        row = {'acceptance':[{'exit':0}], 'phases':{
            'plan':{'exit':0}, 'build':{'exit':0},
            'review':{'exit':0, 'verdict':'PASS'}, 'test':{'exit':0, 'test_pass':False}}}
        score(row)
        self.assertTrue(row['completed'])
        self.assertFalse(row['flow_completed'])


if __name__ == '__main__':
    unittest.main()
