"""The comparison runner's parsing, pricing and summary, on recorded shapes of
each tool's output. No tool is run and nothing is paid."""
import json
import unittest

from compare import (Run, allowance, changed_files, charge, command_for, markdown, merge_into, parse_price,
                     price_tokens, prompt_for, summarize)


def lines(*rows):
    return '\n'.join(json.dumps(r) for r in rows) + '\n'


class Runs(unittest.TestCase):
    def test_a_run_is_a_tool_and_a_model_and_ryter_names_its_connection(self):
        r = Run('ryter=anthropic:claude-sonnet-4-5')
        self.assertEqual((r.tool, r.connection, r.model), ('ryter', 'anthropic', 'claude-sonnet-4-5'))
        self.assertEqual(r.key, 'ryter=anthropic:claude-sonnet-4-5')
        r = Run('opencode=anthropic/claude-sonnet-4-5')
        self.assertEqual((r.tool, r.connection, r.model), ('opencode', None, 'anthropic/claude-sonnet-4-5'))
        for bad in ['ryter=claude-sonnet-4-5', 'cursor=x', 'claude', 'codex=', 'ryter=:m', 'ryter=c:']:
            with self.assertRaises(ValueError, msg=bad):
                Run(bad)

    def test_every_tool_runs_without_asking_and_ryter_is_sandboxed(self):
        prompt = 'do the thing'
        ryter = command_for(Run('ryter=or:m'), '/bin/ryter', prompt, 0.5)
        self.assertEqual(ryter[:3], ['/bin/ryter', '--json', '--hat'])
        self.assertIn('--always-approve', ryter)
        self.assertEqual(ryter[ryter.index('--sandbox') + 1], 'workspace')
        self.assertEqual(ryter[-1], prompt)
        claude = command_for(Run('claude=sonnet'), '/bin/ryter', prompt, 0.5)
        self.assertIn('--dangerously-skip-permissions', claude)
        self.assertEqual(claude[claude.index('--max-budget-usd') + 1], '0.50')
        self.assertEqual(claude[claude.index('--output-format') + 1], 'json')
        codex = command_for(Run('codex=gpt'), '/bin/ryter', prompt, 0.5)
        self.assertEqual(codex[:3], ['codex', 'exec', '--json'])
        self.assertEqual(codex[codex.index('--sandbox') + 1], 'workspace-write')
        self.assertEqual(codex[-1], prompt)
        opencode = command_for(Run('opencode=p/m'), '/bin/ryter', prompt, 0.5)
        self.assertEqual(opencode[:2], ['opencode', 'run'])
        self.assertIn('--auto', opencode)
        self.assertIn('--standalone', opencode)
        self.assertEqual(opencode[-1], prompt)

    def test_the_prompt_is_the_scope_and_the_brief(self):
        p = prompt_for({'brief': '\nFix it.\nDone when tests pass.\n'})
        self.assertTrue(p.startswith('Work only in this project'))
        self.assertTrue(p.endswith('Fix it.\nDone when tests pass.\n'))
        p = prompt_for({'brief': 'Three steps.', 'tasks': [{'brief': 'one'}, {'brief': 'two'}]})
        self.assertIn('Three steps.\none\ntwo\n', p)


class Prices(unittest.TestCase):
    def test_a_price_is_three_rates_per_million(self):
        model, price = parse_price('gpt-5.2=1.75,0.175,14')
        self.assertEqual(model, 'gpt-5.2')
        self.assertEqual(price, {'input': 1.75, 'cached': 0.175, 'output': 14.0})
        self.assertAlmostEqual(price_tokens({'input': 1_000_000, 'cached': 2_000_000, 'output': 100_000}, price),
                               1.75 + 0.35 + 1.4)
        for bad in ['gpt=1,2', 'gpt=a,b,c', '=1,2,3', 'gpt=1,-2,3']:
            with self.assertRaises(ValueError, msg=bad):
                parse_price(bad)

    def test_changed_files_are_the_tools_own(self):
        status = ' M cli.py\n?? __pycache__/\n?? shop/__pycache__/\n?? .ryter/run.toml\n?? shop/errors.py\n'
        self.assertEqual(changed_files(status), ['cli.py', 'shop/errors.py'])
        self.assertEqual(changed_files(''), [])

    def test_the_allowance_keeps_the_reserve(self):
        self.assertAlmostEqual(allowance(5.0, 0.0), 4.9)
        self.assertAlmostEqual(allowance(5.0, 4.85), 0.05)
        self.assertEqual(allowance(5.0, 4.95), 0.0)
        self.assertEqual(allowance(5.0, 6.0), 0.0)


class Charges(unittest.TestCase):
    def test_ryter_is_the_sum_of_its_spend_events_and_stops_on_incomplete(self):
        out = lines({'kind': 'turn_started'},
                    {'kind': 'spend', 'total_usd': 0.01, 'incomplete': False, 'input_tokens': 100,
                     'cached_tokens': 50, 'output_tokens': 10, 'model': 'm'},
                    {'kind': 'spend', 'total_usd': 0.02, 'incomplete': False, 'input_tokens': 200,
                     'cached_tokens': 0, 'output_tokens': 20, 'model': 'm'})
        c = charge(Run('ryter=c:m'), out)
        self.assertEqual((c['usd'], c['priced_by'], c['calls']), (0.03, 'tool', 2))
        self.assertEqual((c['input'], c['cached'], c['output']), (300, 50, 30))
        broken = out + lines({'kind': 'spend', 'total_usd': None, 'incomplete': True,
                              'input_tokens': 0, 'cached_tokens': 0, 'output_tokens': 0, 'model': 'm'})
        c = charge(Run('ryter=c:m'), broken)
        self.assertIsNone(c['usd'])
        self.assertFalse(c['complete'])
        self.assertIsNone(charge(Run('ryter=c:m'), 'not json\n')['usd'])

    def test_claude_code_reports_its_own_dollars(self):
        out = json.dumps({'type': 'result', 'subtype': 'success', 'is_error': False, 'num_turns': 7,
                          'total_cost_usd': 0.1234, 'duration_ms': 90_000, 'result': 'done',
                          'usage': {'input_tokens': 500, 'cache_read_input_tokens': 40_000,
                                    'cache_creation_input_tokens': 2_000, 'output_tokens': 900}})
        c = charge(Run('claude=sonnet'), out)
        self.assertEqual((c['usd'], c['priced_by'], c['calls']), (0.1234, 'tool', 7))
        self.assertEqual((c['input'], c['cached'], c['output']), (500, 40_000, 900))
        self.assertIsNone(charge(Run('claude=sonnet'), 'Error: not logged in\n')['usd'])

    def test_codex_reports_tokens_and_is_priced_from_the_table(self):
        out = lines({'type': 'thread.started', 'thread_id': 't'},
                    {'type': 'turn.completed', 'usage': {'input_tokens': 10_000, 'cached_input_tokens': 8_000,
                                                          'output_tokens': 1_000, 'reasoning_output_tokens': 300}},
                    {'type': 'turn.completed', 'usage': {'input_tokens': 12_000, 'cached_input_tokens': 11_000,
                                                          'output_tokens': 500, 'reasoning_output_tokens': 0}})
        price = {'input': 2.0, 'cached': 0.2, 'output': 10.0}
        c = charge(Run('codex=gpt'), out, price)
        # Codex counts cached tokens inside input_tokens.
        self.assertEqual((c['input'], c['cached'], c['output'], c['calls']), (3_000, 19_000, 1_500, 2))
        self.assertAlmostEqual(c['usd'], (3_000 * 2.0 + 19_000 * 0.2 + 1_500 * 10.0) / 1e6)
        self.assertEqual(c['priced_by'], 'table')
        self.assertIsNone(charge(Run('codex=gpt'), out)['usd'])

    def test_opencode_reports_cost_per_step_or_falls_back_to_the_table(self):
        step = lambda cost, inp, read, out_, reason: {'type': 'step_finish', 'part': {
            'type': 'step-finish', 'cost': cost,
            'tokens': {'input': inp, 'output': out_, 'reasoning': reason, 'cache': {'read': read, 'write': 0}}}}
        out = lines({'type': 'step_start', 'part': {}}, step(0.004, 100, 3_000, 50, 10),
                    {'type': 'text', 'part': {'text': 'done'}}, step(0.006, 200, 3_500, 80, 0))
        c = charge(Run('opencode=p/m'), out)
        self.assertAlmostEqual(c['usd'], 0.010)
        self.assertEqual((c['priced_by'], c['calls']), ('tool', 2))
        self.assertEqual((c['input'], c['cached'], c['output']), (300, 6_500, 140))
        free = lines(step(0, 100, 0, 50, 0))
        self.assertIsNone(charge(Run('opencode=p/m'), free)['usd'])
        c = charge(Run('opencode=p/m'), free, {'input': 1.0, 'cached': 0.1, 'output': 5.0})
        self.assertAlmostEqual(c['usd'], (100 * 1.0 + 50 * 5.0) / 1e6)
        self.assertEqual(c['priced_by'], 'table')


class Summary(unittest.TestCase):
    def report(self):
        def row(solved, usd, seconds=60, by='tool', timed_out=False):
            return {'solved': solved, 'usd': usd, 'seconds': seconds, 'timed_out': timed_out,
                    'charge': {'priced_by': by}, 'acceptance': []}
        return {'date': '2026-10-22', 'ryter': 'abc1234', 'runs': ['ryter=c:m', 'claude=s', 'codex=g'],
                'versions': {'ryter': 'ryter 0.37.0', 'claude': None, 'codex': 'codex-cli 0.1'},
                'budget_usd': 5.0, 'timeout_seconds': 600, 'total_usd': 0.9,
                'prices': {'g': {'input': 2.0, 'cached': 0.2, 'output': 10.0}},
                'tasks': {'a': {'results': {'ryter=c:m': row(True, 0.10), 'claude=s': row(True, 0.30),
                                            'codex=g': row(False, 0.05, 600, 'table', True)}},
                          'b': {'results': {'ryter=c:m': row(True, 0.20, 120), 'claude=s': row(False, 0.25),
                                            'codex=g': {'error': 'fixture trust failed', 'usd': None}}}}}

    def test_solved_per_dollar_is_solved_over_cost(self):
        rows = {s['run']: s for s in summarize(self.report())}
        ryter = rows['ryter=c:m']
        self.assertEqual((ryter['tasks'], ryter['solved']), (2, 2))
        self.assertAlmostEqual(ryter['usd'], 0.30)
        self.assertAlmostEqual(ryter['usd_per_solved'], 0.15)
        self.assertAlmostEqual(ryter['solved_per_usd'], 2 / 0.30)
        self.assertAlmostEqual(ryter['median_minutes'], 1.5)
        claude = rows['claude=s']
        self.assertEqual((claude['solved'], claude['usd']), (1, 0.55))
        codex = rows['codex=g']
        # The errored task is not a result; the timed-out one is a failure.
        self.assertEqual((codex['tasks'], codex['solved'], codex['timed_out']), (1, 0, 1))
        self.assertIsNone(codex['usd_per_solved'])
        self.assertEqual(codex['solved_per_usd'], 0.0, 'nothing solved is zero per dollar, not unknown')
        self.assertTrue(codex['table_priced'])

    def test_a_rerun_drops_the_tools_results_and_spend_and_keeps_the_others(self):
        report = merge_into(self.report(), ['codex=g', 'opencode=o'])
        self.assertEqual(report['runs'], ['ryter=c:m', 'claude=s', 'codex=g', 'opencode=o'])
        self.assertNotIn('codex=g', report['tasks']['a']['results'])
        self.assertIn('ryter=c:m', report['tasks']['a']['results'])
        # a's codex row cost $0.05; b's had no cost
        self.assertAlmostEqual(report['total_usd'], 0.85)
        md = markdown(dict(report, reruns=[{'date': '2026-10-23', 'runs': ['codex=g'], 'budget_usd': 2.0}]))
        self.assertIn('Budget $7.00', md)
        self.assertIn('Run again on 2026-10-23, replacing their earlier results: `codex=g`.', md)
        self.assertIn('| a | ✓ $0.100 | ✓ $0.300 | not run | not run |', md)

    def test_an_unpriced_task_leaves_the_run_without_a_cost(self):
        report = self.report()
        report['tasks']['a']['results']['claude=s']['usd'] = None
        claude = {s['run']: s for s in summarize(report)}['claude=s']
        self.assertIsNone(claude['usd'])
        self.assertEqual(claude['solved'], 1)

    def test_the_markdown_has_the_summary_the_matrix_and_the_rates(self):
        md = markdown(self.report())
        self.assertIn('# Tasks solved per dollar — 2026-10-22', md)
        self.assertIn('| `ryter=c:m` | 2/2 | $0.300 | $0.150 | 6.7 | 1.5 | 0 |', md)
        self.assertIn('| `codex=g` | 0/1 | $0.050 * | — | 0.0 | 10.0 | 1 |', md)
        self.assertIn('`g` $2.0/0.2/10.0 per million', md)
        self.assertIn('| a | ✓ $0.100 | ✓ $0.300 | ✗ $0.050 (timed out) |', md)
        self.assertIn('| b | ✓ $0.200 | ✗ $0.250 | error: fixture trust failed |', md)
        self.assertIn('claude (version unknown)', md)


if __name__ == '__main__':
    unittest.main()
