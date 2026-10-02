#!/usr/bin/env python3
"""Install from signed loopback fixtures; never writes to the user's install dir."""
import base64
import hashlib
import http.server
import io
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile
import threading
import unittest

ROOT = Path(__file__).resolve().parents[1]
OPENSSL = os.environ.get('RYTER_INSTALL_OPENSSL', 'openssl')


class Installer(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='ryter-install-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.downloads = self.root / 'downloads'
        self.downloads.mkdir()
        self.install = self.root / 'bin'
        self.install.mkdir()
        self.existing = self.install / 'ryter'
        self.existing.write_text('old installation')
        self.key = self.root / 'fixture-key.pem'
        self.command('genpkey', '-algorithm', 'ED25519', '-out', str(self.key))
        self.public = self.command('pkey', '-in', str(self.key), '-pubout').stdout
        original = (ROOT / 'install.sh').read_text()
        production_key = (ROOT / 'release/ryter-release.pub.pem').read_text()
        self.assertEqual(original.count(production_key), 1, 'installer and updater must pin the same key')
        # The test copy trusts only this temporary key. Production has no runtime
        # public-key override and never receives the fixture private key.
        self.script = self.root / 'install.sh'
        self.script.write_text(original.replace(production_key, self.public))
        cpu = {'arm64':'aarch64', 'aarch64':'aarch64', 'amd64':'x86_64', 'x86_64':'x86_64'}[platform.machine().lower()]
        target = cpu + ('-apple-darwin' if platform.system() == 'Darwin' else '-unknown-linux-musl')
        self.member = f'ryter-{target}/ryter'
        self.asset = self.downloads / f'ryter-{target}.tar.gz'
        self.archive()
        self.sign()

        class Handler(http.server.SimpleHTTPRequestHandler):
            def log_message(self, *_args):
                pass

        directory = str(self.downloads)
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0),
            lambda *args, **kwargs: Handler(*args, directory=directory, **kwargs))
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.close_server)

    def close_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=10)

    def command(self, *args):
        return subprocess.run([OPENSSL, *args], check=True, capture_output=True, text=True, timeout=10)

    def archive(self, *, linked=False):
        with tarfile.open(self.asset, 'w:gz') as archive:
            info = tarfile.TarInfo(self.member)
            info.mode = 0o755
            if linked:
                info.type = tarfile.SYMTYPE
                info.linkname = '../../outside'
                archive.addfile(info)
            else:
                data = b'#!/bin/sh\nprintf "ryter 0.11.0\\n"\n'
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))

    def sign(self):
        sums = self.downloads / 'SHA256SUMS'
        sums.write_text(hashlib.sha256(self.asset.read_bytes()).hexdigest() + '  ' + self.asset.name + '\n')
        signature = self.root / 'sig.bin'
        self.command('pkeyutl', '-sign', '-inkey', str(self.key), '-rawin', '-in', str(sums), '-out', str(signature))
        (self.downloads / 'SHA256SUMS.sig').write_bytes(base64.b64encode(signature.read_bytes()))

    def install_fixture(self):
        env = {'PATH':os.environ.get('PATH', ''), 'HOME':str(self.root),
               'RYTER_INSTALL_DIR':str(self.install),
               'RYTER_INSTALL_OPENSSL':OPENSSL,
               'RYTER_DOWNLOAD_URL':f'http://127.0.0.1:{self.server.server_port}',
               'TMPDIR':str(self.root)}
        return subprocess.run(['sh', str(self.script)], env=env, capture_output=True, text=True, timeout=20)

    def refused(self):
        result = self.install_fixture()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.existing.read_text(), 'old installation')
        self.assertEqual(list(self.install.glob('.ryter.*')), [])
        return result

    def test_signed_install_and_staging_symlink_is_preserved(self):
        outside = self.root / 'outside'
        outside.write_text('keep')
        link = self.install / '.ryter.new'
        link.symlink_to(outside)
        result = self.install_fixture()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('installed ryter 0.11.0', result.stdout)
        self.assertEqual(outside.read_text(), 'keep')
        self.assertTrue(link.is_symlink())
        self.assertEqual(subprocess.check_output([str(self.existing), '--version'], text=True), 'ryter 0.11.0\n')

    def test_modified_archive_fails_checksum_before_install(self):
        with self.asset.open('ab') as stream:
            stream.write(b'changed')
        self.assertIn('checksum mismatch', self.refused().stderr)

    def test_modified_checksums_cannot_authorize_modified_archive(self):
        with self.asset.open('ab') as stream:
            stream.write(b'changed')
        (self.downloads / 'SHA256SUMS').write_text(hashlib.sha256(self.asset.read_bytes()).hexdigest() + '  ' + self.asset.name + '\n')
        self.assertIn('signature verification failed', self.refused().stderr)

    def test_bad_or_missing_signature_preserves_existing_install(self):
        signature = self.downloads / 'SHA256SUMS.sig'
        signature.write_bytes(base64.b64encode(b'0' * 64))
        self.assertIn('signature verification failed', self.refused().stderr)
        signature.unlink()
        self.assertIn('no signature', self.refused().stderr)

    def test_signed_link_is_not_installed(self):
        self.archive(linked=True)
        self.sign()
        self.refused()


if __name__ == '__main__':
    if shutil.which(OPENSSL) is None:
        raise SystemExit('Tests require OpenSSL 3+; set RYTER_INSTALL_OPENSSL to its executable.')
    unittest.main(verbosity=2)
