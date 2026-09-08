#!/usr/bin/env python3

# Python 3.11+ and PyYAML; run with `make test-nightly` (requires uv).

import base64
import itertools
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import tomllib
import unittest
import xml.etree.ElementTree as ET

import yaml


ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = yaml.safe_load((ROOT / '.github/workflows/nightly.yml').read_text())
JOBS = WORKFLOW['jobs']
STAMP = '20260908.0100'
SHA = '99e32efa36578ffc484a9cbb046b46203278f93b'
VERSION = f'nightly.{SHA[:7]}.{STAMP}'
SPARKLE = '{http://www.andymatuschak.org/xml-namespaces/sparkle}'

MOCKS = r'''
gh() {
  printf 'gh %s\n' "$*" >> "$CALLS"
  case "$1 $2" in
    'run list') if [[ "$CI_RESULT" != missing ]]; then echo 123; fi ;;
    'run watch') [[ "$CI_RESULT" != failure && "$CI_RESULT" != cancelled ]] ;;
    'run view') printf '{"headSha":"%s","conclusion":"%s"}\n' "$CI_SHA" "$CI_RESULT" ;;
    'release view')
      if [[ "$*" == *--jq* ]]; then
        if [[ "$3" == nightly ]]; then printf '%s\n' "$MAC_ASSETS"; else printf '%s\n' "$WIN_ASSETS"; fi
      elif [[ "$*" == *--json* ]]; then
        if [[ "$3" == nightly ]]; then printf '%s\n' "$MAC_RELEASE"; else printf '%s\n' "$WIN_RELEASE"; fi
      else
        [[ "$RELEASE_EXISTS" == true ]]
      fi ;;
    'release create'|'release edit') return 0 ;;
    'release upload') test -s "$4" && [[ "${4##*/}" != "$FAIL_UPLOAD" ]] ;;
    'release delete-asset') return 0 ;;
    *) echo "unexpected gh call: $*" >&2; return 90 ;;
  esac
}
curl() {
  printf 'curl %s\n' "$*" >> "$CALLS"
  if [[ -n "$FAIL_DOWNLOAD" && "$*" == *"$FAIL_DOWNLOAD"* ]]; then return 22; fi
  if [[ "$*" == *--head* ]]; then return 0; fi
  local source=''
  while (( $# )); do
    case "$1" in
      https:*/appcast.xml) source=nightly-artifacts/macos/appcast.xml ;;
      https:*/*.sig) source="nightly-artifacts/windows/$SETUP_NAME.sig" ;;
      --output) cp "$source" "$2"; return ;;
    esac
    shift
  done
  return 90
}
sleep() { :; }
git() { printf '%s\n' "$SOURCE_SHA"; }
'''


def condition(expression, platform, prepare='success', macos='success', windows='success', cancelled=False, failure=False):
    values = {
        'inputs.platform': repr(platform),
        'needs.prepare.result': repr(prepare),
        'needs.build-macos.result': repr(macos),
        'needs.build-windows.result': repr(windows),
        'cancelled()': repr(cancelled),
        'failure()': repr(failure),
    }
    expression = expression.removeprefix('${{').removesuffix('}}').strip()
    for key, value in values.items():
        expression = expression.replace(key, value)
    expression = expression.replace('&&', ' and ').replace('||', ' or ').replace('!', ' not ')
    return eval(' '.join(expression.split()), {'__builtins__': {}}, {})


class NightlyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='runner-nightly-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = dict(os.environ, SOURCE_SHA=SHA, CI_SHA=SHA, GITHUB_SHA=SHA,
                        BUILD_STAMP=STAMP, NIGHTLY_VERSION=VERSION, NIGHTLY_SHORT_SHA=SHA[:7], PLATFORM='both',
                        DMG_NAME=f'Runner-Nightly-{SHA[:7]}.{STAMP}-arm64.dmg',
                        SETUP_NAME=f'Runner-Setup-{VERSION}-x64.exe',
                        CI_RESULT='success', RELEASE_EXISTS='true',
                        FAIL_UPLOAD='', FAIL_DOWNLOAD='', RUNNER_TEMP=str(self.root),
                        CALLS=str(self.root / 'calls'), GITHUB_OUTPUT=str(self.root / 'outputs'),
                        GITHUB_STEP_SUMMARY=str(self.root / 'summary'))
        (self.root / 'script').mkdir()
        (self.root / 'script/verify-nightly-appcast.py').symlink_to(ROOT / 'script/verify-nightly-appcast.py')
        self.mac = self.root / 'nightly-artifacts/macos'
        self.win = self.root / 'nightly-artifacts/windows'
        self.mac.mkdir(parents=True)
        self.win.mkdir(parents=True)
        (self.mac / self.env['DMG_NAME']).write_bytes(b'nightly dmg fixture')
        (self.win / self.env['SETUP_NAME']).write_bytes(b'nightly installer fixture')
        (self.win / (self.env['SETUP_NAME'] + '.sig')).write_bytes(b'installer signature fixture')
        self.appcast = self.mac / 'appcast.xml'
        item = ET.SubElement(ET.SubElement(ET.Element('rss'), 'channel'), 'item')
        release_url = 'https://github.com/yicheng47/runner/releases/tag/nightly'
        for field, value in [('version', STAMP), ('shortVersionString', SHA[:7]),
                             ('hardwareRequirements', 'arm64'), ('fullReleaseNotesLink', release_url)]:
            ET.SubElement(item, SPARKLE + field).text = value
        ET.SubElement(item, 'link').text = release_url
        ET.SubElement(item, 'enclosure', {
            'url': f"https://github.com/yicheng47/runner/releases/download/nightly/{self.env['DMG_NAME']}",
            'length': str((self.mac / self.env['DMG_NAME']).stat().st_size),
            SPARKLE + 'edSignature': base64.b64encode(b'x' * 64).decode(),
        })
        self.item = item
        self.write_appcast()
        self.assets('MAC', [self.env['DMG_NAME'], 'appcast.xml'])
        self.assets('WIN', [self.env['SETUP_NAME'], self.env['SETUP_NAME'] + '.sig'])

    def write_appcast(self):
        root = ET.Element('rss')
        ET.SubElement(root, 'channel').append(self.item)
        ET.ElementTree(root).write(self.appcast)

    def assets(self, platform, names):
        self.env[f'{platform}_ASSETS'] = '\n'.join(names)
        self.env[f'{platform}_RELEASE'] = json.dumps({
            'isPrerelease': True, 'isDraft': False, 'assets': [{'name': name} for name in names],
        })

    def shell(self, script):
        return subprocess.run(['bash', '-euo', 'pipefail', '-c', MOCKS + script],
                              cwd=self.root, env=self.env, capture_output=True, text=True)

    def publish(self):
        failure = None
        for step in JOBS['publish']['steps']:
            if 'run' not in step:
                continue
            expression = step.get('if', '')
            if failure and 'failure()' not in expression and 'cancelled()' not in expression:
                continue
            if expression and not condition(expression, self.env['PLATFORM'], failure=failure is not None):
                continue
            result = self.shell(step['run'])
            if result.returncode:
                failure = result
        return failure or result

    def calls(self):
        path = self.root / 'calls'
        return path.read_text().splitlines() if path.exists() else []

    def test_publish_predicate_including_default_and_cancellation(self):
        self.assertEqual(WORKFLOW[True]['workflow_dispatch']['inputs']['platform']['default'], 'both')
        self.assertEqual(WORKFLOW['concurrency'], {'group': 'nightly', 'cancel-in-progress': True})
        self.assertIn('!cancelled()', JOBS['publish']['if'])
        results = ['success', 'failure', 'cancelled', 'skipped']
        for platform, prepare, macos, windows, cancelled in itertools.product(
                ['both', 'macos', 'windows'], results, results, results, [False, True]):
            expected_builds = {'both': ('success', 'success'), 'macos': ('success', 'skipped'),
                               'windows': ('skipped', 'success')}[platform]
            expected = not cancelled and prepare == 'success' and (macos, windows) == expected_builds
            with self.subTest(platform=platform, prepare=prepare, macos=macos, windows=windows, cancelled=cancelled):
                self.assertEqual(condition(JOBS['publish']['if'], platform, prepare, macos, windows, cancelled), expected)

    def test_preparation_identifies_the_commit_without_reading_an_official_version(self):
        stub = self.root / 'script/bundle-mac'
        stub.write_text('#!/bin/sh\necho "nightly must not read the official version" >&2\nexit 1\n')
        stub.chmod(0o755)
        step = next(step for step in JOBS['prepare']['steps'] if step.get('id') == 'build')
        result = self.shell(step['run'])
        self.assertEqual(result.returncode, 0, result.stderr)
        outputs = dict(line.split('=', 1) for line in (self.root / 'outputs').read_text().splitlines())
        self.assertEqual(outputs['sha'], SHA)
        self.assertEqual(outputs['short_sha'], SHA[:7])
        self.assertRegex(outputs['stamp'], r'^\d{8}\.\d{4}$')
        self.assertEqual(outputs['version'], f'nightly.{SHA[:7]}.' + outputs['stamp'])

    def test_bash_syntax_and_shared_source_artifacts(self):
        for name, job in JOBS.items():
            checkout = job['steps'][0]
            self.assertEqual(checkout['with']['ref'], '${{ github.sha }}' if name == 'prepare' else '${{ needs.prepare.outputs.sha }}')
            for step in job['steps']:
                if 'run' in step and step.get('shell', 'bash') == 'bash':
                    script = re.sub(r'\$\{\{.*?\}\}', 'fixture', step['run'])
                    result = subprocess.run(['bash', '-n', '-c', script], capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, f"{name}: {step['name']}: {result.stderr}")
            if name.startswith('build-'):
                build = next(step for step in job['steps'] if 'RUNNER_BUILD_STAMP' in step.get('env', {}))
                self.assertEqual(build['env']['RUNNER_BUILD_STAMP'], '${{ needs.prepare.outputs.stamp }}')
                self.assertEqual(build['env']['RUNNER_BUILD_SHA'], '${{ needs.prepare.outputs.short_sha }}')
                self.assertFalse(any('gh release ' in step.get('run', '') for step in job['steps']))
                upload = next(step for step in job['steps'] if step.get('uses', '').startswith('actions/upload-artifact@'))
                download = next(step for step in JOBS['publish']['steps'] if step.get('with', {}).get('name') == upload['with']['name'])
                self.assertEqual(set(download['with']), {'name', 'path'})
                identity = 'version' if name == 'build-windows' else 'short_sha'
                self.assertIn('${{ needs.prepare.outputs.' + identity + ' }}', upload['with']['path'])

    def test_versions_and_lockfile_agree(self):
        lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
        version = tomllib.loads((ROOT / 'crates/runner-app/Cargo.toml').read_text())['package']['version']
        for name in ['runner-app', 'runner-backend', 'runner-terminal']:
            manifest = tomllib.loads((ROOT / 'crates' / name / 'Cargo.toml').read_text())
            self.assertEqual(manifest['package']['version'], version)
            self.assertEqual(next(p['version'] for p in lock['package'] if p['name'] == name), version)

    def test_publication_order_and_single_platform_isolation(self):
        for platform, exists in itertools.product(['both', 'macos', 'windows'], ['true', 'false']):
            with self.subTest(platform=platform, exists=exists):
                self.env.update(PLATFORM=platform, RELEASE_EXISTS=exists)
                Path(self.env['CALLS']).write_text('')
                result = self.publish()
                self.assertEqual(result.returncode, 0, result.stderr)
                calls = self.calls()
                uploads = [line for line in calls if line.startswith('gh release upload')]
                expected = []
                if platform != 'windows':
                    expected += [self.env['DMG_NAME'], 'appcast.xml']
                if platform != 'macos':
                    expected += [self.env['SETUP_NAME'], self.env['SETUP_NAME'] + '.sig']
                self.assertEqual([Path(line.split()[4]).name for line in uploads], expected)
                self.assertEqual(sum(line.startswith('gh run watch') for line in calls), 1)
                first_upload = calls.index(uploads[0])
                self.assertTrue(any(line.startswith('gh run view') for line in calls[:first_upload]))
                edits = [line for line in calls if line.startswith(('gh release create', 'gh release edit'))]
                self.assertTrue(all('--prerelease' in line and '--latest=false' in line for line in edits))
                self.assertTrue(all('--target ' + SHA in line for line in edits))
                if platform != 'both':
                    forbidden = 'nightly-win' if platform == 'macos' else 'nightly '
                    self.assertFalse(any(forbidden in line for line in calls if line.startswith('gh release')))

    def test_ci_missing_failed_cancelled_skipped_or_wrong_sha_blocks_mutation(self):
        for conclusion, sha in [('missing', SHA), ('failure', SHA), ('cancelled', SHA),
                                ('skipped', SHA), ('success', 'wrong-sha')]:
            with self.subTest(conclusion=conclusion, sha=sha):
                self.env.update(CI_RESULT=conclusion, CI_SHA=sha)
                Path(self.env['CALLS']).write_text('')
                result = self.publish()
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(any(line.startswith('gh release') for line in self.calls()))

    def test_missing_artifact_and_invalid_feed_block_mutation(self):
        for filename in [self.mac / self.env['DMG_NAME'], self.appcast,
                         self.win / self.env['SETUP_NAME'], self.win / (self.env['SETUP_NAME'] + '.sig')]:
            data = filename.read_bytes()
            filename.unlink()
            result = self.publish()
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(self.calls(), [])
            filename.write_bytes(data)
        for field, wrong in [('version', '20260901.0100'), ('shortVersionString', '0.8.2'),
                             ('hardwareRequirements', 'x86_64'), ('fullReleaseNotesLink', 'https://example.com')]:
            node = self.item.find(SPARKLE + field)
            previous = node.text
            node.text = wrong
            self.write_appcast()
            self.assertNotEqual(self.publish().returncode, 0, field)
            node.text = previous
        self.write_appcast()
        enclosure = self.item.find('enclosure')
        for field, wrong in [('url', 'https://github.com/yicheng47/runner/releases/latest/download/Runner.dmg'),
                             ('length', '999'), (SPARKLE + 'edSignature', '')]:
            previous = enclosure.get(field)
            enclosure.set(field, wrong)
            self.write_appcast()
            self.assertNotEqual(self.publish().returncode, 0, field)
            enclosure.set(field, previous)
        self.assertEqual(self.calls(), [])

    def test_partial_upload_or_public_verification_failure_never_prunes(self):
        for variable, value in [('FAIL_UPLOAD', 'appcast.xml'), ('FAIL_UPLOAD', self.env['SETUP_NAME'] + '.sig'),
                                ('FAIL_DOWNLOAD', 'appcast.xml'), ('FAIL_DOWNLOAD', '.sig')]:
            with self.subTest(variable=variable, value=value):
                self.env.update(FAIL_UPLOAD='', FAIL_DOWNLOAD='')
                self.env[variable] = value
                Path(self.env['CALLS']).write_text('')
                Path(self.env['GITHUB_STEP_SUMMARY']).write_text('')
                self.assertNotEqual(self.publish().returncode, 0)
                self.assertFalse(any('delete-asset' in line for line in self.calls()))
                summary = Path(self.env['GITHUB_STEP_SUMMARY']).read_text()
                self.assertIn(f'Incomplete both nightly {VERSION} from {SHA}', summary)
                self.assertNotIn('Published', summary)

    def test_retention_orders_mixed_versions_by_stamp_and_pairs_signatures(self):
        mac_old = [f'Runner-Nightly-9.0.0-nightly.202608{day:02}.0100-universal.dmg' for day in range(1, 12)]
        win_old = [f'Runner-Setup-9.0.0.202608{day:02}.0100-x64.exe' for day in range(1, 12)]
        self.assets('MAC', [self.env['DMG_NAME'], 'appcast.xml'] + list(reversed(mac_old)))
        self.assets('WIN', [self.env['SETUP_NAME'], self.env['SETUP_NAME'] + '.sig'] + win_old + [name + '.sig' for name in win_old])
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        deleted = [line.split()[4] for line in calls if 'delete-asset' in line]
        self.assertEqual(set(deleted), set(mac_old[:2] + win_old[:2] + [name + '.sig' for name in win_old[:2]]))
        first_delete = next(i for i, line in enumerate(calls) if 'delete-asset' in line)
        self.assertEqual(sum(line.startswith('curl ') for line in calls[:first_delete]), 4)

    def test_pruning_refuses_to_remove_current_appcast_target(self):
        newer = [f'Runner-Nightly-0.8.3-nightly.202609{day:02}.0100-arm64.dmg' for day in range(9, 20)]
        self.assets('MAC', [self.env['DMG_NAME'], 'appcast.xml'] + newer)
        result = self.publish()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('refusing to prune', result.stderr)
        self.assertFalse(any('delete-asset' in line for line in self.calls()))


if __name__ == '__main__':
    unittest.main()
