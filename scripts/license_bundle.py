#!/usr/bin/env python3
"""Collect locked dependency source links and license/notice files for releases."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess


def bundle(output):
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--format-version', '1'], text=True))
    output.mkdir(parents=True, exist_ok=False)
    index = ['# Dependency notices', '',
             'These are the locked third-party packages resolved for this release, including target-specific dependencies.',
             'The source links provide the corresponding unmodified crate source and its licensing information.', '']
    for package in sorted(metadata['packages'], key=lambda item: (item['name'], item['version'])):
        if package['source'] is None:
            continue
        name, version = package['name'], package['version']
        source = Path(package['manifest_path']).parent
        destination = output / f'{name}-{version}'
        destination.mkdir()
        copied = []
        # Include nested license directories, but avoid copying arbitrary source.
        for entry in sorted(source.iterdir()):
            if entry.is_symlink():
                continue
            lowered = entry.name.lower()
            if not lowered.startswith(('license', 'copying', 'copyright', 'notice', 'unlicense', 'acknowledg')):
                continue
            if entry.is_file():
                shutil.copyfile(entry, destination / entry.name)
                copied.append(entry.name)
            elif entry.is_dir():
                for file in sorted(entry.rglob('*')):
                    if file.is_file() and not file.is_symlink():
                        target = destination / file.relative_to(source)
                        target.parent.mkdir(parents=True, exist_ok=True)
                        shutil.copyfile(file, target)
                        copied.append(str(file.relative_to(source)))
        index.extend([f'## {name} {version}', '',
                      f'Declared license: `{package.get("license") or "see source"}`.',
                      f'[Corresponding source](https://static.crates.io/crates/{name}/{name}-{version}.crate).',
                      'Bundled notices: ' + (', '.join(f'[{file}]({name}-{version}/{file})' for file in copied)
                                             or 'see the corresponding source archive.'), ''])
    (output / 'README.md').write_text('\n'.join(index))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    bundle(parser.parse_args().output)
