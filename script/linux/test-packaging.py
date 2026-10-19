#!/usr/bin/env python3
"""Installer ownership/preservation tests; binary package checks accept real outputs."""
import argparse
import contextlib
import hashlib
import importlib.machinery
import importlib.util
import io
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tarfile
import json
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]


def run_captured(command, *, check=True):
    result = subprocess.run(command, capture_output=True, text=True)
    if check and result.returncode:
        print(f'Command failed ({result.returncode}): {command!r}', file=sys.stderr)
        print(f'stdout:\n{result.stdout}', file=sys.stderr)
        print(f'stderr:\n{result.stderr}', file=sys.stderr)
        result.check_returncode()
    return result


class CommandDiagnosticsTests(unittest.TestCase):
    def test_unexpected_failure_reports_command_and_both_streams(self):
        command = [sys.executable, '-c', 'import sys; print("OUT"); print("ERR", file=sys.stderr); sys.exit(7)']
        diagnostics = io.StringIO()
        with contextlib.redirect_stderr(diagnostics), self.assertRaises(subprocess.CalledProcessError) as raised:
            run_captured(command)
        self.assertEqual(raised.exception.returncode, 7)
        self.assertIn(repr(command), diagnostics.getvalue())
        self.assertIn('stdout:\nOUT\n', diagnostics.getvalue())
        self.assertIn('stderr:\nERR\n', diagnostics.getvalue())

    def test_success_and_expected_failure_are_quiet(self):
        for code in [0, 7]:
            with self.subTest(code=code):
                diagnostics = io.StringIO()
                with contextlib.redirect_stderr(diagnostics), contextlib.redirect_stdout(diagnostics):
                    result = run_captured([sys.executable, '-c', f'import sys; print("captured"); sys.exit({code})'], check=code == 0)
                self.assertEqual(result.returncode, code)
                self.assertEqual(result.stdout, 'captured\n')
                self.assertEqual(diagnostics.getvalue(), '')


class DebLifecycleTests(unittest.TestCase):
    def test_first_install_has_existing_root_and_private_log(self):
        with tempfile.TemporaryDirectory(prefix='r823-dpkg-', dir=ROOT / 'target') as temp:
            scratch = Path(temp)
            def fail_before_install(command, **kwargs):
                self.assertTrue((scratch / 'dpkg-root').is_dir())
                self.assertIn('--log=' + str(scratch / 'dpkg.log'), command)
                self.assertIn('--instdir=' + str(scratch / 'dpkg-root'), command)
                return subprocess.CompletedProcess(command, 1, '', 'fixture stops before install')
            with mock.patch('subprocess.run', side_effect=fail_before_install) as run:
                with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(subprocess.CalledProcessError):
                    check_deb_lifecycle(scratch / 'fixture.deb', scratch)
                self.assertEqual(run.call_count, 1)


class VersionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        loader = importlib.machinery.SourceFileLoader('runner_bundle_linux', str(ROOT / 'script/bundle-linux'))
        spec = importlib.util.spec_from_loader(loader.name, loader)
        cls.bundle = importlib.util.module_from_spec(spec)
        loader.exec_module(cls.bundle)

    def identities(self):
        return [self.bundle.package_identity('0.16.2', 'nightly', stamp, sha)
                for stamp, sha in [('20261016.2359', 'fffffff'), ('20261017.0000', 'aaaaaaa')]]

    def test_filenames_retain_commit_identity_and_metadata_orders_by_stamp(self):
        older, newer = self.identities()
        self.assertEqual(older[0], 'nightly.fffffff.20261016.2359')
        self.assertEqual(newer[0], 'nightly.aaaaaaa.20261017.0000')
        self.assertIn('nightly.20261017.0000.aaaaaaa', newer[1])
        self.assertIn('nightly.20261017.0000.aaaaaaa', newer[2])
        self.assertEqual(self.bundle.package_identity('0.16.2', 'production', '20261017.0000', 'aaaaaaa'), ('0.16.2', '0.16.2', '1'))

    def test_remote_nightly_identity_preserves_channel_and_stamp_order(self):
        older = self.bundle.package_identity('0.16.5', 'remote-nightly', '20261016.2359', 'fffffff')
        newer = self.bundle.package_identity('0.16.5', 'remote-nightly', '20261017.0000', 'aaaaaaa')
        self.assertEqual(older[0], 'remote-nightly.fffffff.20261016.2359')
        self.assertEqual(newer, ('remote-nightly.aaaaaaa.20261017.0000',
                                 '0.16.5~remote.nightly.20261017.0000.aaaaaaa',
                                 '0.remote.nightly.20261017.0000.aaaaaaa'))
        if shutil.which('dpkg'):
            run_captured(['dpkg', '--compare-versions', older[1], 'lt', newer[1]])
        if shutil.which('rpm'):
            expression = "%{lua: print(rpm.vercmp('" + older[2] + "', '" + newer[2] + "'))}"
            with tempfile.TemporaryDirectory(prefix='r823-rpm-', dir=ROOT / 'target') as database:
                self.assertEqual(run_captured(['rpm', '--dbpath', database, '--eval', expression]).stdout.strip(), '-1')

    def test_prebuilt_binary_inputs_are_rejected(self):
        # Even real ci binaries cannot be re-labelled as production package inputs.
        result = subprocess.run(['python3', str(ROOT / 'script/bundle-linux'), '--binary-dir', str(ROOT / 'target/ci')], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('unrecognized arguments: --binary-dir', result.stderr)

    @unittest.skipUnless(shutil.which('dpkg'), 'requires dpkg on Linux package host')
    def test_debian_later_stamp_upgrades_even_when_sha_sorts_lower(self):
        older, newer = self.identities()
        subprocess.run(['dpkg', '--compare-versions', older[1], 'lt', newer[1]], check=True)

    @unittest.skipUnless(shutil.which('rpm'), 'requires rpm on Linux package host')
    def test_rpm_later_stamp_upgrades_even_when_sha_sorts_lower(self):
        older, newer = self.identities()
        expression = "%{lua: print(rpm.vercmp('" + older[2] + "', '" + newer[2] + "'))}"
        with tempfile.TemporaryDirectory(prefix='r823-rpm-', dir=ROOT / 'target') as database:
            result = run_captured(['rpm', '--dbpath', database, '--eval', expression])
            self.assertEqual(result.stdout.strip(), '-1')


class NativeAliasTests(unittest.TestCase):
    def test_native_package_aliases_refuse_and_preserve_foreign_paths(self):
        with tempfile.TemporaryDirectory(prefix='r823-alias-', dir=ROOT / 'target') as temp:
            root = Path(temp)
            env = dict(os.environ, DPKG_ROOT=str(root))
            script = str(ROOT / 'packaging/linux/package-aliases.sh')
            def action(name):
                return subprocess.run(['sh', script, name], env=env, capture_output=True, text=True)
            self.assertEqual(action('check').returncode, 0)
            self.assertEqual(action('install').returncode, 0)
            runner = root / 'usr/bin/runner'
            self.assertEqual(os.readlink(runner), '../lib/runner/runner-agent-cli')
            self.assertEqual(action('check').returncode, 0)
            self.assertEqual(action('install').returncode, 0)
            runner.unlink()
            runner.write_text('FOREIGN')
            self.assertNotEqual(action('check').returncode, 0)
            self.assertNotEqual(action('install').returncode, 0)
            self.assertEqual(action('remove').returncode, 0)
            self.assertEqual(runner.read_text(), 'FOREIGN')
            self.assertFalse((root / 'usr/bin/runnerd').is_symlink())
            runner.unlink()
            target = root / 'foreign'
            target.write_text('FOREIGN-SYMLINK-TARGET')
            runner.symlink_to(target)
            self.assertNotEqual(action('check').returncode, 0)
            self.assertEqual(action('remove').returncode, 0)
            self.assertTrue(runner.is_symlink())
            self.assertEqual(target.read_text(), 'FOREIGN-SYMLINK-TARGET')


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='linux-installer-', dir=ROOT / 'target')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.home = self.root / 'home'
        self.prefix = self.home / '.local'
        self.env = dict(os.environ, HOME=str(self.home), XDG_CONFIG_HOME=str(self.home / '.config'))
        self.tree = self.root / 'archive'
        package = self.tree / 'lib/runner'
        package.mkdir(parents=True)
        (package / 'Runner').write_text('APP-BYTES')
        (package / 'runner-agent-cli').write_text('CLI-BYTES')
        (package / 'runnerd').write_text('CLI-BYTES')
        (package / 'install-layout-v1').write_text('runner-linux-package-v1\n')
        (package / 'icon.png').write_bytes(b'ICON')
        for src, dest in [('packaging/linux/runnerd.service', 'lib/systemd/user/runnerd.service'),
                          ('packaging/linux/runner.desktop', 'share/applications/runner.desktop'),
                          ('script/linux/install.sh', 'install.sh')]:
            target = self.tree / dest
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / src, target)
        icon = self.tree / 'share/icons/hicolor/256x256/apps/runner.png'
        icon.parent.mkdir(parents=True)
        icon.write_bytes(b'ICON')

    def install(self, *args):
        return subprocess.run(['sh', str(self.tree / 'install.sh'), '--prefix', str(self.prefix), *args], env=self.env, capture_output=True, text=True)

    def test_install_upgrade_uninstall_retains_user_data_and_foreign_replacements(self):
        self.assertEqual(self.install().returncode, 0)
        command = self.prefix / 'bin/runner'
        self.assertEqual(command.read_text(), 'CLI-BYTES')
        self.assertEqual((self.prefix / 'bin/runnerd').read_bytes(), command.read_bytes())
        data = self.home / '.local/share/com.wycstudios.runner/runner.db'
        data.parent.mkdir(parents=True)
        data.write_text('KEEP-DATABASE')
        source = self.tree / 'lib/runner/runner-agent-cli'
        source.write_text('NEW-CLI-BYTES')
        (self.tree / 'lib/runner/runnerd').write_text('NEW-CLI-BYTES')
        self.assertEqual(self.install().returncode, 0)
        self.assertEqual(command.read_text(), 'NEW-CLI-BYTES')
        command.unlink()
        command.write_text('FOREIGN-COMMAND')
        self.assertEqual(self.install('--uninstall').returncode, 0)
        self.assertEqual(command.read_text(), 'FOREIGN-COMMAND')
        self.assertEqual(data.read_text(), 'KEEP-DATABASE')
        self.assertFalse((self.prefix / 'lib/runner').exists())
        self.assertFalse((self.home / '.config/systemd/user/runnerd.service').exists())

    def test_refuses_foreign_commands_before_writing(self):
        bin_dir = self.prefix / 'bin'
        bin_dir.mkdir(parents=True)
        foreign = bin_dir / 'runner'
        foreign.write_text('FOREIGN')
        self.assertNotEqual(self.install().returncode, 0)
        self.assertEqual(foreign.read_text(), 'FOREIGN')
        self.assertFalse((self.prefix / 'lib/runner').exists())

    def test_custom_prefix_quotes_and_service_is_not_enabled(self):
        self.prefix = self.home / "中文 odd space & 'quote \"double\" % $dollar `tick`"
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        unit = (self.home / '.config/systemd/user/runnerd.service').read_text()
        self.assertIn('Restart=no', unit)
        self.assertIn('%%', unit)
        self.assertIn('KillMode=mixed', unit)
        self.assertFalse((self.home / '.config/systemd/user/default.target.wants').exists())
        desktop = (self.prefix / 'share/applications/runner.desktop').read_text()
        self.assertIn('\\$dollar', desktop)
        self.assertEqual(self.install('--uninstall').returncode, 0)

    def test_upgrade_and_uninstall_preserve_replaced_foreign_icons(self):
        self.assertEqual(self.install().returncode, 0)
        icon = self.prefix / 'share/icons/hicolor/256x256/apps/runner.png'
        foreign = self.root / 'foreign-icon'
        foreign.write_bytes(b'FOREIGN-ICON')
        icon.unlink()
        icon.symlink_to(foreign)
        (self.tree / 'lib/runner/runner-agent-cli').write_text('NEW-CLI')
        self.assertNotEqual(self.install().returncode, 0)
        self.assertEqual(foreign.read_bytes(), b'FOREIGN-ICON')
        self.assertEqual((self.prefix / 'bin/runner').read_text(), 'CLI-BYTES')
        icon.unlink()
        icon.write_bytes(b'FOREIGN-REGULAR-ICON')
        self.assertNotEqual(self.install().returncode, 0)
        self.assertEqual(icon.read_bytes(), b'FOREIGN-REGULAR-ICON')
        self.assertEqual(self.install('--uninstall').returncode, 0)
        self.assertEqual(icon.read_bytes(), b'FOREIGN-REGULAR-ICON')
        self.assertEqual(foreign.read_bytes(), b'FOREIGN-ICON')

    def test_uninstall_preserves_replacement_unit_and_desktop_symlinks(self):
        self.assertEqual(self.install().returncode, 0)
        paths = [self.home / '.config/systemd/user/runnerd.service', self.prefix / 'share/applications/runner.desktop']
        for index, path in enumerate(paths):
            foreign = self.root / f'foreign-{index}'
            foreign.write_bytes(path.read_bytes())
            path.unlink()
            path.symlink_to(foreign)
        self.assertEqual(self.install('--uninstall').returncode, 0)
        for path in paths:
            self.assertTrue(path.is_symlink())
            self.assertTrue(path.read_bytes())

    def test_system_prefix_spellings_are_rejected_without_writing(self):
        for prefix in ['/usr/', '/usr/local///', '/', '/tmp/../usr', '//usr', '/usr/./local']:
            with self.subTest(prefix=prefix):
                self.prefix = Path(prefix)
                self.assertNotEqual(self.install().returncode, 0)

    def test_foreign_service_desktop_and_package_directory_are_preserved(self):
        for path in [self.home / '.config/systemd/user/runnerd.service', self.prefix / 'share/applications/runner.desktop', self.prefix / 'lib/runner/install-layout-v1']:
            with self.subTest(path=path):
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('FOREIGN')
                self.assertNotEqual(self.install().returncode, 0)
                self.assertEqual(path.read_text(), 'FOREIGN')
                path.unlink()


def exercise_headless(prefix, home):
    env = {key: value for key, value in os.environ.items() if not key.startswith('RUNNER_') and key != 'SSH_CONNECTION'}
    env.update(HOME=str(home), XDG_DATA_HOME=str(home / 'data'), XDG_CONFIG_HOME=str(home / 'config'))
    home.mkdir(parents=True, exist_ok=True)
    assert len(str(home / 'data/com.wycstudios.runner/runnerd.sock').encode()) < 100
    cli = prefix / 'bin/runner'
    try:
        result = subprocess.run([str(cli), 'role', 'list', '--json'], env=env, capture_output=True, text=True, timeout=75)
        if result.returncode:
            raise AssertionError(f'headless first launch failed: {result.stderr}')
        json.loads(result.stdout)
        data = home / 'data/com.wycstudios.runner'
        installed = data / 'bin/runner'
        source = prefix / 'lib/runner/runner-agent-cli'
        assert installed.read_bytes() == source.read_bytes(), 'headless app-data CLI bootstrap missing/stale'
        assert (prefix / 'lib/runner/runnerd').read_bytes() == source.read_bytes(), 'runnerd differs from app sidecar'
    finally:
        stopped = subprocess.run([str(cli), 'daemon', 'stop'], env=env, capture_output=True, text=True, timeout=25)
        if stopped.returncode not in [0, 3]:
            raise AssertionError(f'owned daemon cleanup failed: {stopped.stderr}')
    return env


def check_deb_lifecycle(package, scratch):
    root, db = scratch / 'dpkg-root', scratch / 'dpkg-db'
    root.mkdir()
    db.mkdir()
    command = ['dpkg', '--admindir=' + str(db), '--instdir=' + str(root),
               '--log=' + str(scratch / 'dpkg.log'),
               '--force-not-root', '--force-script-chrootless', '--force-depends']
    def dpkg(*args, check=True):
        return run_captured([*command, *args], check=check)
    dpkg('--install', str(package))
    env = exercise_headless(root / 'usr', scratch / 'home')
    data = scratch / 'home/data/com.wycstudios.runner/runner.db'
    before = data.read_bytes()
    dpkg('--install', str(package))
    runner = root / 'usr/bin/runner'
    runner.unlink()
    runner.write_text('FOREIGN-PACKAGE-COMMAND')
    refused = dpkg('--install', str(package), check=False)
    assert refused.returncode != 0 and runner.read_text() == 'FOREIGN-PACKAGE-COMMAND'
    dpkg('--remove', 'runner')
    dpkg('--purge', 'runner')
    assert runner.read_text() == 'FOREIGN-PACKAGE-COMMAND'
    assert data.read_bytes() == before
    assert not (root / 'usr/lib/runner').exists()
    # Fresh install also refuses an existing foreign command before unpacking payload.
    refused = dpkg('--install', str(package), check=False)
    assert refused.returncode != 0
    assert runner.read_text() == 'FOREIGN-PACKAGE-COMMAND'
    assert not (root / 'usr/lib/runner').exists()


def check_packages(directory):
    """Inspect real packages and exercise isolated tar/deb headless install lifetimes."""
    directory = Path(directory).resolve()
    packages = list(directory.glob('Runner-*-linux-*'))
    if not packages:
        raise SystemExit('No Linux packages found')
    for package in packages:
        with tempfile.TemporaryDirectory(prefix='r823-pkg-', dir='/tmp') as scratch:
            scratch = Path(scratch)
            if package.name.endswith('.tar.gz'):
                with tarfile.open(package) as archive:
                    for member in archive.getmembers():
                        assert not Path(member.name).is_absolute() and '..' not in Path(member.name).parts
                    archive.extractall(scratch / 'archive')
                tree = next((scratch / 'archive').iterdir())
                home = scratch / 'home'
                prefix = home / '.local'
                env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home / 'config'))
                subprocess.run(['sh', str(tree / 'install.sh')], env=env, check=True)
                env = exercise_headless(prefix, home)
                data = home / 'data/com.wycstudios.runner/runner.db'
                before = hashlib.sha256(data.read_bytes()).hexdigest()
                subprocess.run(['sh', str(tree / 'install.sh')], env=env, check=True)
                subprocess.run(['sh', str(tree / 'install.sh'), '--uninstall'], env=env, check=True)
                assert hashlib.sha256(data.read_bytes()).hexdigest() == before, 'uninstall changed user database'
                assert not (prefix / 'lib/runner').exists()
            elif package.suffix == '.deb':
                subprocess.run(['dpkg-deb', '--info', str(package)], check=True)
                listing = subprocess.check_output(['dpkg-deb', '--contents', str(package)], text=True)
                subprocess.run(['dpkg-deb', '--extract', str(package), str(scratch / 'extract')], check=True)
                check_deb_lifecycle(package, scratch)
            elif package.suffix == '.rpm':
                database = scratch / 'rpm-db'
                database.mkdir()
                metadata = run_captured(['rpm', '--dbpath', str(database), '-qip', str(package)])
                print(metadata.stdout, end='')
                listing = run_captured(['rpm', '--dbpath', str(database), '-qlp', str(package)]).stdout
            else:
                continue
            if package.suffix in ['.deb', '.rpm']:
                for expected in ['usr/lib/runner/Runner', 'usr/lib/runner/runner-agent-cli', 'usr/lib/runner/runnerd', 'usr/lib/systemd/user/runnerd.service', 'usr/share/applications/runner.desktop']:
                    if expected not in listing:
                        raise SystemExit(f'{package}: missing {expected}')
        print(hashlib.sha256(package.read_bytes()).hexdigest(), package.name)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--packages')
    args = parser.parse_args()
    if args.packages:
        check_packages(args.packages)
    else:
        unittest.main(argv=[__file__], verbosity=2)
