#!/usr/bin/env python3
"""Validate benchmark fixtures, then exercise simulated or explicitly budgeted live hats.

python3 bench/run.py --mode reference
python3 bench/run.py --mode simulated --binary target/debug/ryter
python3 bench/run.py --mode live --model qwen/qwen3-coder-flash --budget 5
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
from simulated_provider import Fixture, Provider, reply


def commands(project, checks, env):
    rows = []
    for command in checks:
        try:
            result = subprocess.run(['sh', '-c', command], cwd=project, env=env,
                                    capture_output=True, text=True, timeout=120)
            rows.append({'command': command, 'exit': result.returncode,
                         'output': (result.stdout + result.stderr)[-8000:]})
        except subprocess.TimeoutExpired:
            rows.append({'command': command, 'exit': 124, 'output': 'fixture timed out'})
    return rows


def passed(rows):
    return bool(rows) and all(row['exit'] == 0 for row in rows)


def overlay(source, target):
    for file in source.rglob('*'):
        if file.is_file():
            out = target / file.relative_to(source)
            out.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(file, out)


def isolated_env(home):
    return {'PATH': os.environ.get('PATH', ''), 'HOME': str(home),
            'LANG': 'C.UTF-8', 'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_NOSYSTEM': '1',
            'RUSTUP_HOME': os.environ.get('RUSTUP_HOME', str(Path.home() / '.rustup')),
            'CARGO_HOME': str(home / 'cargo'), 'CARGO_NET_OFFLINE': 'true'}


def validate(task, spec, directory):
    directory.mkdir(parents=True)
    env = isolated_env(directory)
    needs = commands(task, spec['needs'], env)
    if not passed(needs):
        return {'valid': False, 'needs': needs}
    rows = {}
    for mode in ['baseline', 'reference']:
        project = directory / mode
        shutil.copytree(task / 'repo', project)
        if mode == 'reference':
            overlay(task / 'solution', project)
        visible = commands(project, spec['checks'], env)
        overlay(task / 'hidden', project)
        hidden = commands(project, spec['accept'], env)
        rows[mode] = {'visible': visible, 'hidden': hidden}
    digest = hashlib.sha256()
    for file in sorted(task.rglob('*')):
        if file.is_file() and '__pycache__' not in file.parts:
            digest.update(str(file.relative_to(task)).encode() + b'\0' + file.read_bytes())
    return {'fixture_sha256':digest.hexdigest(), 'valid': passed(rows['reference']['visible']) and passed(rows['reference']['hidden'])
            and not passed(rows['baseline']['hidden']), **rows}


def simulate_responses(phase, task, spec, broken=False):
    if phase == 'plan':
        return [reply('Fixture plan: implement only the declared files and run the checks.')]
    if phase == 'build' and broken:
        return [reply('Intentional negative control: leave the task broken.')]
    if phase == 'build':
        writes = [('write', {'path': str(file.relative_to(task / 'solution')), 'content': file.read_text()})
                  for file in sorted((task / 'solution').rglob('*')) if file.is_file()]
        return [reply('', writes), reply('Reference implementation written by simulated builder.')]
    if phase == 'review':
        return [reply('', [('bash', {'command': ' && '.join(spec['checks'])})]),
                reply('Simulated reviewer verdict; compare against hidden tests.\nVERDICT: PASS')]
    return [reply('', [('run_project', {'action': 'test'})]),
            reply('', [('report_test', {'title':'Simulated visible checks', 'scenarios':[
                {'name':'visible checks', 'result':'pass', 'note':'scripted verdict; use tool results as evidence'}]})]),
            reply('Simulated test complete.')]


def phase_prompt(phase, spec):
    brief = spec['brief'] + '\n' + '\n'.join(t.get('brief', '') for t in spec.get('tasks', []))
    scope = ('Work only in this fixture project. Do not install packages, use external tools/services, '
             'read another project, or change system or user configuration. Do not run git commit. '
             'Use the existing dependencies. This is a CLI fixture, with no server to leave running. ')
    return scope + {
        'plan': 'Plan this task briefly in your reply; do not call present_plan (this run is headless).\n' + brief,
        'build': 'The user approves the fixture task and its edits. Implement it now, inspect the files and run its checks.\n' + brief,
        'review': 'Review the changes and run the visible checks. Do not change project files. '
                  'State what you actually checked, then finish with exactly VERDICT: PASS or VERDICT: FAIL.\n' + brief,
        'test': 'Use run_project action=test with the fixture run file already provided by the harness. '
                'Inspect its real result, then file report_test with pass/fail/not_reached honestly. '
                'Do not propose another run file or use hidden tests.\n' + brief,
    }[phase]


def execute_task(task, spec, directory, args, provider, allocation):
    directory.mkdir(parents=True)
    if provider:
        fixture = Fixture(directory, args.binary, provider)
        project, env = fixture.project, fixture.env
        env.update({key: value for key, value in isolated_env(fixture.home).items()
                    if key in ['CARGO_HOME', 'RUSTUP_HOME', 'CARGO_NET_OFFLINE']})
    else:
        project = directory / 'project'
        project.mkdir()
        check_home = directory / 'check-home'
        check_home.mkdir()
        env = isolated_env(check_home)
    cli_env = env if provider else dict(os.environ, CARGO_NET_OFFLINE='true')
    overlay(task / 'repo', project)
    # Independent repository: never let fixture Git operations reach Ryter itself.
    init = commands(project, ['git init -q -b main', 'git config user.name "Ryter fixture"',
                              'git config user.email fixture@localhost', 'git config commit.gpgsign false',
                              'git add -A && git commit -qm baseline'], env)
    if not passed(init):
        return {'error':'fixture Git initialization failed', 'checks':init, 'usd':0}
    ryter = project / '.ryter'
    ryter.mkdir(exist_ok=True)
    # This is user-provided fixture configuration, not proposed by the model.
    (ryter / 'run.toml').write_text('test = ' + json.dumps(spec['checks']) + '\n')
    if not provider:
        (ryter / 'config.toml').write_text(f'''[spend]
session_budget_usd = {allocation}
review_usd = {allocation}
[context_windows]
"{args.model}" = 32768
[pricing."{args.model}"]
input_per_million = 0.52
output_per_million = 2.6
cached_per_million = 0.104
cache_write_per_million = 0.65
[update]
mode = "off"
''')
        trust = subprocess.run([str(args.binary), 'trust'], cwd=project, env=cli_env,
                               capture_output=True, text=True, timeout=15)
        if trust.returncode:
            return {'error':'fixture trust failed', 'details':trust.stderr, 'usd':0}
    row = {'phases':{}, 'usd':0.0, 'accounting_complete':True, 'mode':args.mode}
    for index, phase in enumerate(['plan', 'build', 'review', 'test']):
        if provider:
            provider.responses.extend(simulate_responses(phase, task, spec, args.simulate_broken_build))
        command = [str(args.binary), '--json', '--hat', phase, '--always-approve',
                   '--connection', 'fixture' if provider else args.connection,
                   '--model', 'fixture-model' if provider else args.model,
                   '--sandbox', 'off' if provider else 'workspace', '-p', phase_prompt(phase, spec)]
        if index:
            command.append('--continue')
        try:
            result = subprocess.run(command, cwd=project, env=cli_env, capture_output=True,
                                    text=True, timeout=240)
        except subprocess.TimeoutExpired:
            row.update(error=f'{phase} timed out; charge may be incomplete', accounting_complete=False)
            break
        (directory / f'{phase}.ndjson').write_text(result.stdout)
        (directory / f'{phase}.stderr').write_text(result.stderr)
        events = [json.loads(line) for line in result.stdout.splitlines()]
        spend = [event for event in events if event['kind'] == 'spend']
        row['usd'] += sum(event['total_usd'] or 0 for event in spend)
        row['accounting_complete'] &= all(event['total_usd'] is not None and not event['incomplete'] for event in spend)
        if not provider and (not spend or any(event['model'] != args.model for event in spend)):
            row['accounting_complete'] = False
        tool_results = [event for event in events if event['kind'] == 'tool_result']
        text = ''.join(event['text'] for event in events if event['kind'] == 'token')
        verdicts = re.findall(r'(?im)^\s*(?:\*\*)?VERDICT\s*:\s*(PASS|FAIL)', text)
        tested = [event['passed'] for event in events if event['kind'] == 'tested']
        row['phases'][phase] = {'exit':result.returncode, 'calls':len(spend),
                                'tool_errors':sum(event['is_error'] for event in tool_results),
                                'verdict':verdicts[-1] if verdicts else None,
                                'test_pass':tested[-1] if tested else None,
                                'text':text[-4000:]}
        if result.returncode or not row['accounting_complete']:
            row['error'] = f'{phase} stopped; see captured events'
            break
    # Models never see the reference or hidden tests. Only now reveal acceptance.
    overlay(task / 'hidden', project)
    row['acceptance'] = commands(project, spec['accept'], env)
    row['completed'] = passed(row['acceptance'])
    review = row['phases'].get('review', {})
    test = row['phases'].get('test', {})
    row['false_review_pass'] = review.get('verdict') == 'PASS' and not row['completed']
    row['false_test_pass'] = test.get('test_pass') is True and not row['completed']
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=['reference','simulated','live'], default='reference')
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug/ryter')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--task', action='append', default=[])
    parser.add_argument('--model', default='qwen/qwen3-coder-flash')
    parser.add_argument('--connection', default='openrouter')
    parser.add_argument('--budget', type=float)
    parser.add_argument('--simulate-broken-build', action='store_true',
                        help='negative control: simulated reviewers/testers pass an unchanged fixture')
    args = parser.parse_args()
    args.binary = args.binary.resolve()
    if args.simulate_broken_build and args.mode != "simulated":
        parser.error("--simulate-broken-build requires simulated mode")
    if args.mode == 'live' and (args.budget is None or not 0 < args.budget <= 5):
        parser.error('live mode requires an explicit --budget above zero and at most $5')
    if args.mode == 'live' and args.model != 'qwen/qwen3-coder-flash':
        parser.error('live cost allowance is calibrated for qwen/qwen3-coder-flash; review pricing before changing it')
    output = (args.output or ROOT / 'target' / f'benchmark-{args.mode}-{time.time_ns()}').resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = {'mode':args.mode, 'model':args.model if args.mode == 'live' else None,
              'budget_usd':args.budget, 'negative_control':args.simulate_broken_build, 'total_usd':0.0, 'tasks':{},
              'cost_kind':'recorded provider usage' if args.mode == 'live' else 'synthetic fixture accounting; no charge'}
    tasks = [(p.parent, tomllib.loads(p.read_text())) for p in sorted((ROOT / 'bench').glob('*/task.toml'))
             if not args.task or p.parent.name in args.task]
    if not tasks:
        parser.error('no matching task')
    # Validate every selected reference before any paid call.
    for task, spec in tasks:
        record = validate(task, spec, output / task.name / 'validation')
        report['tasks'][task.name] = {'validation':record}
        print(f'reference {task.name}: {record["valid"]}', flush=True)
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    if not all(t['validation']['valid'] for t in report['tasks'].values()):
        print(f'Invalid or unavailable reference; no provider calls. See {output / "report.json"}')
        return 1
    if args.mode != 'reference':
        provider = Provider() if args.mode == 'simulated' else None
        try:
            for task, spec in tasks:
                remaining = (args.budget or 0) - report['total_usd']
                # Reserve for the last in-flight request; each session also has a
                # small allocation. No retries after incomplete billing.
                allocation = min(0.35, remaining - 0.10) if not provider else 0
                if not provider and allocation <= 0:
                    report['stopped'] = 'budget reserve reached'
                    break
                record = execute_task(task, spec, output / task.name / 'run', args, provider, allocation)
                report['tasks'][task.name]['run'] = record
                report['total_usd'] += record['usd']
                (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
                print(f'{args.mode} {task.name}: completed={record.get("completed")} cost={record["usd"]:.6f} total={report["total_usd"]:.6f}', flush=True)
                if not record.get('accounting_complete', True):
                    report['stopped'] = 'incomplete accounting; reconcile before more paid calls'
                    break
        finally:
            if provider:
                provider.__exit__(None, None, None)
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(output / 'report.json')
    if args.mode == 'simulated':
        runs = [task.get('run', {}) for task in report['tasks'].values()]
        if args.simulate_broken_build:
            return int(not all(row.get('false_review_pass') and row.get('false_test_pass') for row in runs))
        return int(not all(row.get('completed') and not row.get('error') for row in runs))
    return int('stopped' in report)


if __name__ == '__main__':
    sys.exit(main())
