"""Scoring regressions from real model formatting and incomplete workflows."""
import json
from pathlib import Path
import tempfile
import unittest

from run import rescore, review_verdict, score, select_tasks


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
            'review':{'exit':1, 'verdict':'PASS'}}}
        score(row)
        self.assertTrue(row['completed'])
        self.assertFalse(row['flow_completed'])
        row = {'acceptance':[{'exit':0}], 'phases':{
            'plan':{'exit':0}, 'build':{'exit':0},
            'review':{'exit':0, 'verdict':'FAIL'}}}
        score(row)
        self.assertFalse(row['flow_completed'])
        row = {'acceptance':[{'exit':0}], 'phases':{
            'plan':{'exit':0}, 'build':{'exit':0},
            'review':{'exit':0, 'verdict':'PASS'}}}
        score(row)
        self.assertTrue(row['flow_completed'])
        self.assertFalse(row['false_review_pass'])



class Selection(unittest.TestCase):
    def test_tasks_are_picked_by_name_and_level(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name, level in [('b-hard', 'hard'), ('a-plain', None), ('c-hard', 'hard')]:
                (root / name).mkdir()
                (root / name / 'task.toml').write_text(
                    f'title = "{name}"\n' + (f'level = "{level}"\n' if level else ''))
            names = lambda tasks: [path.name for path, _spec in tasks]
            self.assertEqual(names(select_tasks(root)), ['a-plain', 'b-hard', 'c-hard'])
            self.assertEqual(names(select_tasks(root, level='hard')), ['b-hard', 'c-hard'])
            self.assertEqual(names(select_tasks(root, level='standard')), ['a-plain'])
            self.assertEqual(names(select_tasks(root, ['c-hard', 'a-plain'])), ['a-plain', 'c-hard'])
            self.assertEqual(names(select_tasks(root, ['a-plain'], 'hard')), [])
            self.assertEqual(select_tasks(root, ['b-hard'])[0][1]['title'], 'b-hard')


if __name__ == '__main__':
    unittest.main()
