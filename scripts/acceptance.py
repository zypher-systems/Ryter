#!/usr/bin/env python3
"""Repeatable CLI acceptance: python3 scripts/acceptance.py [--binary PATH]."""
import argparse
import json
from pathlib import Path
import tempfile
import unittest

from simulated_provider import Fixture, Provider, reply


class Acceptance(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='ryter-cli-')
        self.addCleanup(self.temp.cleanup)
        self.provider = Provider()
        self.addCleanup(self.provider.__exit__, None, None, None)
        self.fixture = Fixture(self.temp.name, BINARY, self.provider)

    def successful(self, *args):
        result, events = self.fixture.events(*args)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(events)
        self.assertFalse(self.provider.errors)
        return events

    def test_reply_resume_and_spend(self):
        self.provider.responses.extend([reply(), reply('continued')])
        first = self.successful('-p', 'first fixture message')
        self.successful('-c', '-p', 'second fixture message')
        self.assertTrue(any('first fixture message' in str(m['content'])
                            for m in self.provider.requests[-1]['messages']))
        spend = next(e for e in first if e['kind'] == 'spend')
        self.assertAlmostEqual(spend['total_usd'], 0.00101)
        self.assertFalse(spend['incomplete'])

    def test_explicit_model_beats_saved_hat_route(self):
        hats = self.fixture.config.parent / 'hats.toml'
        hats.write_text('[review]\nconnection="fixture"\nmodel="wrong-expensive-model"\n')
        self.provider.responses.append(reply())
        self.successful('--hat', 'review', '--connection', 'fixture', '--model', 'fixture-model', '-p', 'review')
        self.assertEqual(self.provider.requests[-1]['model'], 'fixture-model')
        self.assertIn('wrong-expensive-model', hats.read_text(), 'do not rewrite user defaults')

    def test_each_hat_is_sent_and_read_only_hats_refuse_writes(self):
        for hat in ['plan', 'review']:
            self.provider.responses.extend([reply('', [('write', {'path':'source.txt', 'content':'bad'})]), reply()])
            events = self.successful('--hat', hat, '--always-approve', '-p', 'try fixture edit')
            result = next(e for e in events if e['kind'] == 'tool_result')
            self.assertTrue(result['is_error'], hat)
            self.assertFalse((self.fixture.project / 'source.txt').exists())
            self.assertTrue(any(f'[hat: {hat}' in str(m['content'])
                                for m in self.provider.requests[-1]['messages']))

    def test_budget_stops_and_resume_sends_nothing(self):
        self.fixture.configure('\n[spend]\nsession_budget_usd = 0.0001\n')
        self.provider.responses.append(reply())
        result, _ = self.fixture.events('-p', 'hit budget')
        self.assertEqual(result.returncode, 3, result.stderr)
        before = len(self.provider.requests)
        result, _ = self.fixture.events('-c', '-p', 'must not send')
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertEqual(len(self.provider.requests), before)

    def test_interrupted_usage_is_kept_and_budget_remains_closed(self):
        self.fixture.configure('\n[spend]\nsession_budget_usd = 1.0\n')
        self.provider.responses.append(reply(complete=False))
        result, events = self.fixture.events('-p', 'interrupted request')
        spend = next(e for e in events if e['kind'] == 'spend')
        self.assertTrue(spend['incomplete'])
        self.assertAlmostEqual(spend['total_usd'], 0.00101)
        before = len(self.provider.requests)
        resumed, _ = self.fixture.events('-c', '-p', 'closed budget')
        self.assertEqual(resumed.returncode, 3, (result.stderr, resumed.stderr))
        self.assertEqual(len(self.provider.requests), before)

    def test_version_ignores_invalid_config_and_missing_connection_fails(self):
        self.fixture.config.write_text('invalid TOML')
        result = self.fixture.run('--version')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.startswith('ryter '))
        self.fixture.configure()
        result = self.fixture.run('--connection', 'missing', '-p', 'no request')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.provider.requests, [])

    def test_torn_transcript_resumes_with_recovery_notice(self):
        self.provider.responses.extend([reply(), reply()])
        self.successful('-p', 'preserve me')
        transcript, = self.fixture.config.parent.glob('sessions/*/*/transcript.jsonl')
        with transcript.open('ab') as stream:
            stream.write(b'{"role":"assistant","content":"torn')
        events = self.successful('-c', '-p', 'resume after tear')
        self.assertTrue(list(transcript.parent.glob('transcript.jsonl.recovery-*.bak')))
        self.assertTrue(any(e['kind'] == 'notice' and 'recover' in e['message'].lower() for e in events))
        self.assertIn('preserve me', transcript.read_text())


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/ryter'))
    args, rest = parser.parse_known_args()
    BINARY = args.binary.resolve()
    unittest.main(argv=['acceptance', *rest], verbosity=2)
