#!/usr/bin/env python3
"""Tasks solved per dollar: the bench tasks given to Ryter and to other coding
agents on this machine (Claude Code, Codex, OpenCode, Kilo), each with the same
brief in a fresh copy of the same project, scored by the hidden tests, costed
by what each tool reports.

python3 bench/compare.py --run ryter=anthropic:claude-sonnet-4-5 \\
    --run claude=claude-sonnet-4-5 --run opencode=anthropic/claude-sonnet-4-5 \\
    --budget 10 --dry-run
python3 bench/compare.py --run codex=gpt-5.2 --price gpt-5.2=1.75,0.175,14 --budget 5
python3 bench/compare.py --run opencode=anthropic/claude-sonnet-4-5 --budget 3 \
    --into target/compare-20261022-120000   # rerun one tool into an existing report

Every tool runs as the user has it set up: its own login, its own settings.
Nothing is copied between tools. Live calls are opt-in, under one budget for
the whole run, and no CI job runs this.
"""
import argparse
import json
import math
import os
from pathlib import Path
import re
import shutil
import signal
import statistics
import subprocess
import sys
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'bench'))
from run import LEVELS, commands, isolated_env, overlay, passed, select_tasks, validate  # noqa: E402

TOOLS = ['ryter', 'claude', 'codex', 'opencode', 'kilo']
RESERVE_USD = 0.10
# A tool's cap is written with two decimals, and a cap of 0 means no cap to
# Ryter; so a task starts only with a cent or more to spend.
MIN_CAP_USD = 0.01
SCOPE = ('Work only in this project, the current directory. Do not install packages, use other '
         'projects, external services or the network, or change system or user configuration. '
         'Do not commit. Use the existing dependencies. This is a command-line project with no '
         'server to leave running. Implement the task below, run its checks, and stop when they '
         'pass or when you cannot make them pass.\n\n')


class Run:
    """One tool on one model. `spec` is `tool=model`; for ryter the model is
    `connection:model`."""

    def __init__(self, spec):
        tool, sep, model = spec.partition('=')
        if tool not in TOOLS or not sep or not model:
            raise ValueError(f'--run takes tool=model with tool one of {", ".join(TOOLS)}: {spec!r}')
        self.tool, self.connection = tool, None
        if tool == 'ryter':
            self.connection, sep, model = model.partition(':')
            if not sep or not model or not self.connection:
                raise ValueError(f'--run ryter takes connection:model: {spec!r}')
        self.model = model

    @property
    def key(self):
        return f'{self.tool}={self.connection + ":" if self.connection else ""}{self.model}'

    @property
    def directory(self):
        """The run's directory name under a task: the key with its
        punctuation flattened, so two models of one tool never share one."""
        return re.sub(r'[^A-Za-z0-9._-]+', '-', self.key)

    def __repr__(self):
        return self.key


def parse_price(spec):
    """`model=input,cached,output`, dollars per million tokens."""
    model, sep, rates = spec.partition('=')
    parts = rates.split(',')
    if not sep or not model or len(parts) != 3:
        raise ValueError(f'--price takes model=input,cached,output per million: {spec!r}')
    try:
        rates = [float(p) for p in parts]
    except ValueError:
        raise ValueError(f'--price rates must be numbers: {spec!r}') from None
    if any(r < 0 for r in rates):
        raise ValueError(f'--price rates must not be negative: {spec!r}')
    return model, dict(zip(['input', 'cached', 'output'], rates))


def price_tokens(tokens, price):
    """Dollars for `tokens` ({input, cached, output}: input is the uncached
    prompt) at `price` (per million)."""
    return sum(tokens[k] * price[k] for k in ['input', 'cached', 'output']) / 1_000_000


def prompt_for(spec):
    brief = spec['brief'].strip() + '\n' + '\n'.join(t.get('brief', '').strip() for t in spec.get('tasks', []))
    return SCOPE + brief.strip() + '\n'


def cap_text(cap_usd):
    """A task's cap as the tools take it, two decimals. Never `0.00`: to
    Ryter a cap of 0 is no cap at all."""
    text = f'{cap_usd:.2f}'
    if float(text) <= 0:
        raise ValueError(f'a cap of ${cap_usd} would be written as 0, which turns the cap off')
    return text


def command_for(run, binary, prompt, cap_usd):
    """The tool's command line for one task in the current directory, with
    no question asked of a person. `cap_usd` is this task's cap where the
    tool has one."""
    if run.tool == 'ryter':
        return [str(binary), '--json', '--hat', 'build', '--always-approve', '--sandbox', 'workspace',
                '--connection', run.connection, '--model', run.model, '-p', prompt]
    if run.tool == 'claude':
        return ['claude', '-p', prompt, '--output-format', 'json', '--dangerously-skip-permissions',
                '--no-session-persistence', '--model', run.model, '--max-budget-usd', cap_text(cap_usd)]
    if run.tool == 'codex':
        return ['codex', 'exec', '--json', '--sandbox', 'workspace-write', '--ephemeral',
                '--color', 'never', '-m', run.model, prompt]
    if run.tool == 'kilo':
        # Kilo's run is OpenCode's older one: `--dir` names the directory,
        # `--auto` answers what is not denied, the events are the same shape.
        return ['kilo', 'run', '--format', 'json', '--auto', '--dir', '.', '-m', run.model, prompt]
    return ['opencode', 'run', '--format', 'json', '--standalone', '--auto', '-m', run.model, prompt]


def json_lines(text):
    rows = []
    for line in text.splitlines():
        line = line.strip()
        if line.startswith('{'):
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return rows


def charge(run, stdout, price=None):
    """What the tool says the task cost: {usd, priced_by, input, cached,
    output, calls, complete}. `usd` is None when nothing priced it;
    `complete` is False when the tool said its own accounting was not."""
    out = {'usd': None, 'priced_by': None, 'input': 0, 'cached': 0, 'output': 0, 'calls': 0, 'complete': True}
    if run.tool == 'ryter':
        spend = [e for e in json_lines(stdout) if e.get('kind') == 'spend']
        for e in spend:
            out['input'] += e.get('input_tokens', 0)
            out['cached'] += e.get('cached_tokens', 0)
            out['output'] += e.get('output_tokens', 0)
            if e.get('total_usd') is None or e.get('incomplete'):
                out['complete'] = False
        out['calls'] = len(spend)
        if spend and out['complete']:
            out['usd'], out['priced_by'] = sum(e['total_usd'] for e in spend), 'tool'
        return out
    if run.tool == 'claude':
        results = [e for e in json_lines(stdout) if e.get('type') == 'result']
        if not results:
            return out
        r = results[-1]
        usage = r.get('usage') or {}
        out['input'] = usage.get('input_tokens', 0)
        out['cached'] = usage.get('cache_read_input_tokens', 0)
        out['output'] = usage.get('output_tokens', 0)
        out['calls'] = r.get('num_turns', 0)
        if isinstance(r.get('total_cost_usd'), (int, float)):
            # Claude Code's figure is its own estimate at list prices, whatever
            # the login; a subscription is billed nothing per token.
            out['usd'], out['priced_by'] = float(r['total_cost_usd']), 'tool estimate'
        return out
    if run.tool == 'codex':
        turns = [e for e in json_lines(stdout) if e.get('type') == 'turn.completed']
        for e in turns:
            u = e.get('usage') or {}
            cached = u.get('cached_input_tokens', 0)
            out['input'] += max(u.get('input_tokens', 0) - cached, 0)
            out['cached'] += cached
            out['output'] += u.get('output_tokens', 0)
        out['calls'] = len(turns)
        if turns and price:
            out['usd'], out['priced_by'] = price_tokens(out, price), 'table'
        return out
    # OpenCode and Kilo: a `step_finish` part per model call, with its cost.
    steps = [e.get('part') or {} for e in json_lines(stdout) if e.get('type') == 'step_finish']
    usd = 0.0
    for p in steps:
        t = p.get('tokens') or {}
        out['input'] += t.get('input', 0)
        out['cached'] += (t.get('cache') or {}).get('read', 0)
        out['output'] += t.get('output', 0) + t.get('reasoning', 0)
        usd += p.get('cost') or 0
    out['calls'] = len(steps)
    if steps and usd > 0:
        out['usd'], out['priced_by'] = usd, 'tool'
    elif steps and price:
        out['usd'], out['priced_by'] = price_tokens(out, price), 'table'
    return out


def run_tool(command, cwd, env, timeout, log):
    """Run the tool in its own process group; on timeout kill the group.
    Returns (exit, stdout, seconds, timed_out)."""
    started = time.monotonic()
    with (log / 'stdout.ndjson').open('w') as out, (log / 'stderr.txt').open('w') as err:
        proc = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                stdout=out, stderr=err, text=True, start_new_session=True)
        timed_out = False
        try:
            proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(proc.pid, signal.SIGTERM)
            try:
                proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
    return proc.returncode, (log / 'stdout.ndjson').read_text(), time.monotonic() - started, timed_out


def allowance(budget, spent, reserve=RESERVE_USD):
    """What one task may cost now, in whole cents, or 0 when the next task
    must not start. Whole cents, since a cap is written with two decimals
    and `0.11 - 0.10` is a hair under a cent in floating point."""
    cents = round((budget - spent - reserve) * 100, 6)
    return max(math.floor(cents) / 100, 0.0)


def changed_files(status):
    """The files a tool left changed, from `git status --porcelain`: not the
    bytecode its test runs made, not Ryter's own folder."""
    return sorted(line[3:] for line in status.splitlines()
                  if line and not line[3:].startswith('.ryter/') and '__pycache__' not in line[3:])


def stale_directories(report, runs):
    """The directories of the results `merge_into` is about to drop, relative
    to the output directory. A result from before directories were recorded
    lived under the tool's name."""
    stale = []
    for name, task in report['tasks'].items():
        for key in runs:
            old = task.get('results', {}).get(key)
            if old:
                stale.append(old.get('directory') or f'{name}/{key.partition("=")[0]}')
    return stale


def merge_into(report, runs):
    """Prepare an existing report for `runs` to be run again into it: their
    earlier results go, with their spend; the run keys stay in order."""
    for key in runs:
        if key not in report['runs']:
            report['runs'].append(key)
        for task in report['tasks'].values():
            old = task.get('results', {}).pop(key, None)
            if old and old.get('usd') is not None:
                report['total_usd'] -= old['usd']
    report['total_usd'] = max(round(report['total_usd'], 9), 0.0)
    return report


def execute(run, task, spec, directory, binary, env, timeout, cap_usd, price):
    """One tool on one task. The tool runs with `env`, the user's own, since
    its login lives there; the fixture's git and the scoring run under the
    isolated environment `validate` proved the baseline fails in, so what
    is in the user's shell cannot pass or fail a tree on the tool's behalf."""
    directory.mkdir(parents=True)
    project = directory / 'project'
    project.mkdir()
    score_home = directory / 'score-home'
    score_home.mkdir()
    score_env = isolated_env(score_home)
    overlay(task / 'repo', project)
    init = commands(project, ['git init -q -b main', 'git config user.name "Ryter bench"',
                              'git config user.email bench@localhost', 'git config commit.gpgsign false',
                              'git add -A && git commit -qm baseline'], score_env)
    if not passed(init):
        return {'error': 'fixture git initialization failed', 'checks': init, 'usd': None}
    if run.tool == 'ryter':
        ryter = project / '.ryter'
        ryter.mkdir()
        # The run file is the project's own: its test and its fast check.
        (ryter / 'run.toml').write_text('test = ' + json.dumps(spec['checks']) + '\n'
                                        + 'check = ' + json.dumps(spec['checks'][0]) + '\n')
        (ryter / 'config.toml').write_text(f'[spend]\nsession_budget_usd = {cap_text(cap_usd)}\n[update]\nmode = "off"\n')
        trust = subprocess.run([str(binary), 'trust'], cwd=project, env=env, capture_output=True,
                               text=True, timeout=15)
        if trust.returncode:
            return {'error': 'fixture trust failed', 'details': trust.stderr, 'usd': None}
    prompt = prompt_for(spec)
    (directory / 'prompt.txt').write_text(prompt)
    command = command_for(run, binary, prompt, cap_usd)
    # Every tool runs in the copy: one of them reads `PWD` before asking the
    # kernel, so the copy is `PWD` too.
    env = dict(env, PWD=str(project))
    code, stdout, seconds, timed_out = run_tool(command, project, env, timeout, directory)
    row = {'run': run.key, 'directory': f'{task.name}/{run.directory}', 'exit': code,
           'seconds': round(seconds, 1), 'timed_out': timed_out, 'charge': charge(run, stdout, price)}
    row['usd'] = row['charge']['usd']
    status = subprocess.run(['git', 'status', '--porcelain'], cwd=project, env=score_env,
                            capture_output=True, text=True)
    row['changed'] = changed_files(status.stdout)
    # The hidden tests are copied in only now.
    overlay(task / 'hidden', project)
    row['acceptance'] = commands(project, spec['accept'], score_env)
    row['solved'] = passed(row['acceptance'])
    return row


def tool_version(run, binary):
    exe = str(binary) if run.tool == 'ryter' else run.tool
    try:
        done = subprocess.run([exe, '--version'], capture_output=True, text=True, timeout=20)
        return (done.stdout or done.stderr).strip().splitlines()[0] if done.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired, IndexError):
        return None


def summarize(report):
    """Per run: tasks given, solved, cost, cost per solved task, solved per
    dollar, median minutes. Costs are None when a task could not be priced."""
    rows = []
    for key in report['runs']:
        results = [t['results'][key] for t in report['tasks'].values() if key in t.get('results', {})]
        done = [r for r in results if 'solved' in r]
        solved = sum(r['solved'] for r in done)
        priced = all(r.get('usd') is not None for r in done) and bool(done)
        usd = sum(r['usd'] for r in done) if priced else None
        rows.append({'run': key, 'tasks': len(done), 'solved': solved, 'usd': usd,
                     'usd_per_solved': (usd / solved) if priced and solved else None,
                     'solved_per_usd': (solved / usd) if priced and usd else None,
                     'timed_out': sum(r.get('timed_out', False) for r in done),
                     'median_minutes': statistics.median(r['seconds'] for r in done) / 60 if done else None,
                     'table_priced': any(r.get('charge', {}).get('priced_by') == 'table' for r in done),
                     'estimated': key.startswith('claude=')
                     or any(r.get('charge', {}).get('priced_by') == 'tool estimate' for r in done)})
    return rows


def usd(v):
    return '—' if v is None else f'${v:.3f}'


def markdown(report):
    budget = report['budget_usd'] + sum(r['budget_usd'] for r in report.get('reruns', []))
    lines = [f'# Tasks solved per dollar — {report["date"]}', '',
             f'Ryter `{report["ryter"]}`; tools as installed: '
             + ', '.join(f'{k} ({v or "version unknown"})' for k, v in report['versions'].items()) + '.',
             f'Budget ${budget:.2f}, spent {usd(report["total_usd"])}; '
             f'{report["timeout_seconds"]} s per task; one attempt per task'
             + (f'; the {report["level"]} tasks only' if report.get('level') else '') + '.', '',
             '| Run | Solved | Cost | Cost per solved | Solved per $ | Median minutes | Timed out |',
             '|---|---|---|---|---|---|---|']
    summary = summarize(report)
    for s in summary:
        flag = (' *' if s['table_priced'] else '') + (' †' if s['estimated'] else '')
        lines.append(f'| `{s["run"]}` | {s["solved"]}/{s["tasks"]} | {usd(s["usd"])}{flag} | '
                     f'{usd(s["usd_per_solved"])} | '
                     f'{"—" if s["solved_per_usd"] is None else f"{s["solved_per_usd"]:.1f}"} | '
                     f'{"—" if s["median_minutes"] is None else f"{s["median_minutes"]:.1f}"} | {s["timed_out"]} |')
    if any(s['table_priced'] for s in summary):
        lines += ['', '\\* priced from the token counts at the rates given: '
                  + ', '.join(f'`{m}` ${p["input"]}/{p["cached"]}/{p["output"]} per million input/cached/output'
                              for m, p in report['prices'].items()) + '.']
    if any(s['estimated'] for s in summary):
        lines += ['', "† the tool's own estimate of the cost at list prices, not a provider's charge: "
                  "Claude Code prices its usage itself, and on a subscription login nothing is billed per token."]
    lines += ['', '| Task | ' + ' | '.join(f'`{k}`' for k in report['runs']) + ' |',
              '|---|' + '---|' * len(report['runs'])]
    for name, task in report['tasks'].items():
        cells = []
        for key in report['runs']:
            r = task.get('results', {}).get(key)
            if r is None or 'solved' not in r:
                cells.append('not run' if r is None else f'error: {r.get("error")}')
            else:
                cells.append(('✓' if r['solved'] else '✗') + ' ' + usd(r['usd'])
                             + (' (timed out)' if r.get('timed_out') else ''))
        lines.append(f'| {name} | ' + ' | '.join(cells) + ' |')
    for rerun in report.get('reruns', []):
        lines += ['', f'Run again on {rerun["date"]}, replacing their earlier results: '
                  + ', '.join(f'`{k}`' for k in rerun['runs']) + '.']
    if report.get('stopped'):
        lines += ['', f'Stopped: {report["stopped"]}.']
    return '\n'.join(lines) + '\n'


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--run', action='append', default=[], help='tool=model; ryter=connection:model')
    parser.add_argument('--price', action='append', default=[],
                        help='model=input,cached,output dollars per million, for a tool that reports tokens only')
    parser.add_argument('--task', action='append', default=[])
    parser.add_argument('--level', choices=LEVELS, help='only the tasks at this level; every level when absent')
    parser.add_argument('--budget', type=float, help='dollars for the whole run, all tools; required to run')
    parser.add_argument('--timeout', type=int, default=600, help='seconds per task per tool')
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug/ryter')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--dry-run', action='store_true', help='validate, print the commands, call nothing')
    parser.add_argument('--render', type=Path, help='an output directory: rewrite its RESULTS.md from report.json, run nothing')
    parser.add_argument('--into', type=Path,
                        help='an earlier output directory: run the given tools again into its report, '
                             'replacing their results; --budget is for this invocation')
    args = parser.parse_args(argv)
    if args.render:
        report = json.loads((args.render / 'report.json').read_text())
        (args.render / 'RESULTS.md').write_text(markdown(report))
        print(args.render / 'RESULTS.md')
        return 0
    try:
        runs = [Run(s) for s in args.run]
        prices = dict(parse_price(s) for s in args.price)
    except ValueError as e:
        parser.error(str(e))
    if not runs:
        parser.error('at least one --run')
    if args.budget is None:
        parser.error('--budget is required')
    if len({r.key for r in runs}) != len(runs):
        parser.error('each --run once')
    if len({r.directory for r in runs}) != len(runs):
        parser.error('two runs would share a directory name (their keys differ only in punctuation): '
                     + ', '.join(r.key for r in runs))
    if not RESERVE_USD + MIN_CAP_USD <= args.budget <= 50:
        parser.error(f'--budget is dollars from {RESERVE_USD + MIN_CAP_USD:.2f} (the reserve and a cent) to 50')
    for r in runs:
        if r.tool == 'codex' and r.model not in prices:
            parser.error(f'codex reports tokens only: give --price {r.model}=input,cached,output')
        if r.tool != 'ryter' and shutil.which(r.tool) is None:
            parser.error(f'{r.tool} is not installed (not on PATH)')
    args.binary = args.binary.resolve()
    if not args.binary.is_file():
        parser.error(f'no ryter binary at {args.binary}')
    env = dict(os.environ, CARGO_NET_OFFLINE='true')
    if args.into:
        if args.output or args.task or args.level or args.dry_run:
            parser.error('--into takes the earlier report whole: no --output, no --task, no --level, no --dry-run')
        output = args.into.resolve()
        report = json.loads((output / 'report.json').read_text())
        tasks = [(ROOT / 'bench' / name, tomllib.loads((ROOT / 'bench' / name / 'task.toml').read_text()))
                 for name in report['tasks']]
        for task, _spec in tasks:
            shutil.rmtree(output / task.name / 'validation', ignore_errors=True)
    else:
        output = (args.output or ROOT / 'target' / f'compare-{time.strftime("%Y%m%d-%H%M%S")}').resolve()
        output.mkdir(parents=True, exist_ok=False)
        report = {'date': time.strftime('%Y-%m-%d'), 'ryter': subprocess.run(
                      ['git', 'rev-parse', '--short', 'HEAD'], cwd=ROOT, capture_output=True, text=True).stdout.strip(),
                  'runs': [r.key for r in runs], 'versions': {r.tool: tool_version(r, args.binary) for r in runs},
                  'budget_usd': args.budget, 'timeout_seconds': args.timeout, 'prices': prices,
                  'level': args.level, 'total_usd': 0.0, 'tasks': {}, 'command': sys.argv[1:]}
        tasks = select_tasks(ROOT / 'bench', args.task, args.level)
        if not tasks:
            parser.error('no matching task')
    for task, spec in tasks:
        record = validate(task, spec, output / task.name / 'validation')
        report['tasks'].setdefault(task.name, {'results': {}})['validation'] = record
        print(f'reference {task.name}: {record["valid"]}', flush=True)
        if record['valid']:
            # The solved tree is not left where a tool could read it.
            for tree in ['baseline', 'reference']:
                shutil.rmtree(output / task.name / 'validation' / tree, ignore_errors=True)
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    if not all(t['validation']['valid'] for t in report['tasks'].values()):
        print(f'Invalid or unavailable reference; nothing called. See {output / "report.json"}')
        return 1
    if args.into:
        # Only now, with every fixture proven, does the earlier report lose
        # the runs' results and their directories.
        keys = [r.key for r in runs]
        stale = stale_directories(report, keys)
        report = merge_into(report, keys)
        report.pop('stopped', None)
        report['versions'].update({r.tool: tool_version(r, args.binary) for r in runs})
        report['prices'].update(prices)
        report.setdefault('reruns', []).append({'date': time.strftime('%Y-%m-%d'), 'runs': keys,
                                                'budget_usd': args.budget, 'command': sys.argv[1:]})
        for rel in stale:
            if rel and not rel.startswith('/') and '..' not in Path(rel).parts:
                shutil.rmtree(output / rel, ignore_errors=True)
        for task, _spec in tasks:
            for r in runs:
                shutil.rmtree(output / task.name / r.directory, ignore_errors=True)
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    spent_before = report['total_usd']
    if args.dry_run:
        task, spec = tasks[0]
        cap = allowance(args.budget, 0)
        if cap < MIN_CAP_USD:
            print(f'--budget {args.budget} leaves under a cent past the reserve; no task would start')
            return 1
        print(f'\nFor {task.name}, in a fresh copy of bench/{task.name}/repo, each would run:')
        for r in runs:
            print('  ' + ' '.join(repr(a) if ' ' in a else a for a in
                                  command_for(r, args.binary, '<prompt>', cap)))
        print(f'\nThe prompt:\n{prompt_for(spec)}')
        return 0
    for task, spec in tasks:
        for r in runs:
            cap = allowance(args.budget, report['total_usd'] - spent_before)
            if cap < MIN_CAP_USD:
                report['stopped'] = f'budget reserve reached before {task.name} on {r.key}'
                break
            row = execute(r, task, spec, output / task.name / r.directory, args.binary, env, args.timeout,
                          cap, prices.get(r.model))
            report['tasks'][task.name]['results'][r.key] = row
            if row.get('usd') is not None:
                report['total_usd'] += row['usd']
            (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
            print(f'{task.name} {r.key}: solved={row.get("solved")} cost={usd(row.get("usd"))} '
                  f'{row.get("seconds", 0):.0f}s total={usd(report["total_usd"])}', flush=True)
            if row.get('usd') is None and 'error' not in row:
                report['stopped'] = f'{r.key} did not price {task.name}; reconcile before more paid calls'
                break
        if report.get('stopped'):
            break
    (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    (output / 'RESULTS.md').write_text(markdown(report))
    print(output / 'RESULTS.md')
    return int('stopped' in report)


if __name__ == '__main__':
    sys.exit(main())
