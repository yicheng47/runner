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
  if [[ "$1" == release && "$3" != nightly ]]; then
    echo "unexpected release: $3" >&2
    return 90
  fi
  case "$1 $2" in
    'run list') if [[ "$CI_RESULT" != missing ]]; then echo 123; fi ;;
    'run watch') [[ "$CI_RESULT" != failure && "$CI_RESULT" != cancelled ]] ;;
    'run view') printf '{"headSha":"%s","conclusion":"%s"}\n' "$CI_SHA" "$CI_RESULT" ;;
    'release view')
      if [[ "$*" == *--jq* ]]; then
        jq -r '.assets[].name' "$RELEASE_STATE"
      elif [[ "$*" == *--json* ]]; then
        cat "$RELEASE_STATE"
      else
        [[ "$RELEASE_EXISTS" == true ]]
      fi ;;
    'release create'|'release edit')
      while (( $# )); do
        if [[ "$1" == --notes-file ]]; then test -s "$2"; return; fi
        shift
      done
      return 90 ;;
    'release upload')
      test -s "$4" && [[ "${4##*/}" != "$FAIL_UPLOAD" ]] || return 1
      jq --arg name "${4##*/}" '.assets |= (map(select(.name != $name)) + [{name: $name}])' "$RELEASE_STATE" > "$RELEASE_STATE.tmp"
      mv "$RELEASE_STATE.tmp" "$RELEASE_STATE" ;;
    'release delete-asset')
      jq --arg name "$4" '.assets |= map(select(.name != $name))' "$RELEASE_STATE" > "$RELEASE_STATE.tmp"
      mv "$RELEASE_STATE.tmp" "$RELEASE_STATE" ;;
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
git() {
  printf 'git %s\n' "$*" >> "$CALLS"
  case "$1" in
    rev-parse)
      if [[ "$*" == *nightly* ]]; then
        [[ -n "$PREVIOUS_TAG_SHA" ]] && printf '%s\n' "$PREVIOUS_TAG_SHA"
      else
        printf '%s\n' "$SOURCE_SHA"
      fi ;;
    fetch) [[ -n "$PREVIOUS_TAG_SHA" ]] ;;
    diff) printf '%s\n' "$CHANGED_FILES" | sed '/^$/d' ;;
    describe) [[ -n "$RELEASE_TAG" ]] && printf '%s\n' "$RELEASE_TAG" ;;
    log) printf '%s\n' "$CHANGELOG" | sed '/^$/d' ;;
    config|tag) return 0 ;;
    push) [[ -z "$FAIL_TAG_PUSH" ]] ;;
    *) echo "unexpected git call: $*" >&2; return 90 ;;
  esac
}
'''


def condition(expression, platform, prepare='success', macos='success', windows='success', cancelled=False, failure=False, skip='false'):
    values = {
        'needs.prepare.outputs.platform': repr(platform),
        'needs.prepare.outputs.skip': repr(skip),
        'needs.prepare.result': repr(prepare),
        'needs.build-macos.result': repr(macos),
        'needs.build-windows.result': repr(windows),
        'cancelled()': repr(cancelled),
        'failure()': repr(failure),
    }
    expression = expression.removeprefix('${{').removesuffix('}}').strip()
    for key, value in values.items():
        expression = expression.replace(key, value)
    expression = expression.replace('!=', ' NE ').replace('&&', ' and ').replace('||', ' or ').replace('!', ' not ').replace(' NE ', ' != ')
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
                        FAIL_UPLOAD='', FAIL_DOWNLOAD='', FAIL_TAG_PUSH='', RUNNER_TEMP=str(self.root),
                        RELEASE_TAG='v0.8.2', PREVIOUS_TAG_SHA='', CHANGED_FILES='', AUTOMATIC='false',
                        CHANGELOG='- feat(nightly): one change (abc1234)\n- fix(ui): another (def5678)',
                        RELEASE_STATE=str(self.root / 'release.json'),
                        CALLS=str(self.root / 'calls'), GITHUB_OUTPUT=str(self.root / 'outputs'),
                        GITHUB_STEP_SUMMARY=str(self.root / 'summary'))
        (self.root / 'script').mkdir()
        (self.root / 'script/verify-nightly-appcast.py').symlink_to(ROOT / 'script/verify-nightly-appcast.py')
        (self.root / 'script/nightly-release-notes.md').symlink_to(ROOT / 'script/nightly-release-notes.md')
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
        self.assets([])

    def write_appcast(self):
        root = ET.Element('rss')
        ET.SubElement(root, 'channel').append(self.item)
        ET.ElementTree(root).write(self.appcast)

    def assets(self, names):
        Path(self.env['RELEASE_STATE']).write_text(json.dumps({
            'isPrerelease': True, 'isDraft': False, 'assets': [{'name': name} for name in names],
        }))

    def published_assets(self):
        release = json.loads(Path(self.env['RELEASE_STATE']).read_text())
        return {asset['name'] for asset in release['assets']}

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
        self.assertEqual(WORKFLOW[True]['workflow_run'], {'workflows': ['CI'], 'types': ['completed'], 'branches': ['main']})
        self.assertNotIn('concurrency', WORKFLOW)
        self.assertEqual(JOBS['build-macos']['concurrency'], {'group': 'nightly-build-macos', 'cancel-in-progress': True})
        self.assertEqual(JOBS['build-windows']['concurrency'], {'group': 'nightly-build-windows', 'cancel-in-progress': True})
        self.assertEqual(JOBS['publish']['concurrency'], {'group': 'nightly-publish', 'cancel-in-progress': False})
        self.assertIn("github.event.workflow_run.conclusion == 'success'", JOBS['prepare']['if'])
        self.assertIn('!cancelled()', JOBS['publish']['if'])
        results = ['success', 'failure', 'cancelled', 'skipped']
        for platform, prepare, macos, windows, cancelled, skip in itertools.product(
                ['both', 'macos', 'windows'], results, results, results, [False, True], ['false', 'true']):
            expected_builds = {'both': ('success', 'success'), 'macos': ('success', 'skipped'),
                               'windows': ('skipped', 'success')}[platform]
            expected = not cancelled and prepare == 'success' and skip == 'false' and (macos, windows) == expected_builds
            with self.subTest(platform=platform, prepare=prepare, macos=macos, windows=windows, cancelled=cancelled, skip=skip):
                self.assertEqual(condition(JOBS['publish']['if'], platform, prepare, macos, windows, cancelled, skip=skip), expected)
        for job in ['build-macos', 'build-windows']:
            own = job.removeprefix('build-')
            for platform, skip in itertools.product(['both', 'macos', 'windows'], ['false', 'true']):
                expected = skip == 'false' and platform in ('both', own)
                self.assertEqual(condition(JOBS[job]['if'], platform, skip=skip), expected, (job, platform, skip))

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
        self.assertEqual(outputs['platform'], 'both')
        self.assertEqual(outputs['skip'], 'false')

    def test_automatic_cuts_skip_rebuilds_and_docs_only_changes(self):
        step = next(step for step in JOBS['prepare']['steps'] if step.get('id') == 'build')
        older = '2222222bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
        cases = [
            ('true', older, 'docs/arch/arch.md\nREADME.md\n.github/workflows/ci.yaml', 'true'),
            ('true', older, 'docs/arch/arch.md\ncrates/runner-app/src/main.rs', 'false'),
            ('true', older, '.github/workflows/nightly.yml', 'false'),
            ('true', older, 'Cargo.lock', 'false'),
            ('true', SHA, '', 'true'),
            ('true', '', '', 'false'),
            ('false', older, 'docs/arch/arch.md', 'false'),
            ('false', SHA, '', 'false'),
        ]
        for automatic, previous, changed, expected in cases:
            with self.subTest(automatic=automatic, previous=previous[:7], changed=changed):
                self.env.update(AUTOMATIC=automatic, PREVIOUS_TAG_SHA=previous, CHANGED_FILES=changed)
                (self.root / 'outputs').write_text('')
                result = self.shell(step['run'])
                self.assertEqual(result.returncode, 0, result.stderr)
                outputs = dict(line.split('=', 1) for line in (self.root / 'outputs').read_text().splitlines())
                self.assertEqual(outputs['skip'], expected)
                self.assertEqual(outputs['sha'], SHA)

    def test_bash_syntax_and_shared_source_artifacts(self):
        self.assertIsNone(re.search(r'nightly-win\b', (ROOT / '.github/workflows/nightly.yml').read_text()))
        for name, job in JOBS.items():
            checkout = job['steps'][0]
            if name == 'prepare':
                self.assertEqual(checkout['with']['ref'], "${{ github.event_name == 'workflow_run' && github.event.workflow_run.head_sha || github.sha }}")
                self.assertEqual(checkout['with']['fetch-depth'], 0)
            else:
                self.assertEqual(checkout['with']['ref'], '${{ needs.prepare.outputs.sha }}')
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
                self.assets([])
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
                self.assertTrue(all(line.split()[3] == 'nightly' for line in uploads))
                self.assertEqual(self.published_assets(), set(expected))
                self.assertEqual(sum(line.startswith('gh release view nightly --json isPrerelease,isDraft,assets') for line in calls), 1)
                self.assertEqual(sum(line.startswith('gh run watch') for line in calls), 1)
                first_upload = calls.index(uploads[0])
                self.assertTrue(any(line.startswith('gh run view') for line in calls[:first_upload]))
                edits = [line for line in calls if line.startswith(('gh release create', 'gh release edit'))]
                self.assertEqual(len(edits), 1)
                self.assertTrue(all(f"--notes-file {self.root}/nightly-notes.md" in line for line in edits))
                tag_push = calls.index('git push --force origin refs/tags/nightly')
                self.assertLess(calls.index(next(line for line in calls if line.startswith('gh run view'))), tag_push)
                self.assertLess(tag_push, calls.index(edits[0]))
                self.assertIn(f'git tag --force --annotate nightly --message Runner Nightly {VERSION} {SHA}', calls)
                self.assertTrue(all('--prerelease' in line and '--latest=false' in line for line in edits))
                self.assertTrue(all('--target ' + SHA in line for line in edits))
                self.assertFalse(any('nightly-win' in line for line in calls))

    def test_release_notes_carry_the_changelog_and_the_tag_moves_only_after_the_ci_gate(self):
        fixed = (ROOT / 'script/nightly-release-notes.md').read_text()
        notes = self.root / 'nightly-notes.md'
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        text = notes.read_text()
        self.assertTrue(text.startswith('## Changes since v0.8.2\n'))
        self.assertIn(f'This nightly is {SHA[:7]}.', text)
        self.assertIn('- feat(nightly): one change (abc1234)\n- fix(ui): another (def5678)\n', text)
        self.assertTrue(text.endswith(fixed))
        calls = self.calls()
        self.assertIn(f'git describe --tags --match v* --abbrev=0 {SHA}', calls)
        self.assertIn(f'git log --no-merges --format=- %s (%h) v0.8.2..{SHA}', calls)
        self.assertFalse(any('refs/tags/nightly' in line and line.startswith('git fetch') for line in calls))

        self.env['CHANGELOG'] = '\n'.join(f'- change {i} ({i:07x})' for i in range(1, 131))
        self.assertEqual(self.publish().returncode, 0)
        text = notes.read_text()
        self.assertIn('- change 100 (0000064)\n- and 30 more commits\n', text)
        self.assertNotIn('- change 101 ', text)

        self.env['CHANGELOG'] = ''
        self.assertEqual(self.publish().returncode, 0)
        self.assertIn('No commits since v0.8.2.', notes.read_text())

        self.env['RELEASE_TAG'] = ''
        Path(self.env['CALLS']).write_text('')
        self.assertEqual(self.publish().returncode, 0)
        text = notes.read_text()
        self.assertTrue(text.startswith('## Changes in this nightly\n'))
        self.assertIn(f'No official release tag is reachable from {SHA[:7]}.', text)
        self.assertTrue(text.endswith(fixed))
        self.assertFalse(any(line.startswith('git log') for line in self.calls()))
        self.assertIn('git push --force origin refs/tags/nightly', self.calls())

        for conclusion in ['failure', 'missing']:
            self.env.update(CI_RESULT=conclusion, RELEASE_TAG='v0.8.2')
            Path(self.env['CALLS']).write_text('')
            self.assertNotEqual(self.publish().returncode, 0)
            self.assertFalse(any(line.startswith(('git tag', 'git push')) for line in self.calls()), conclusion)
        self.env['CI_RESULT'] = 'success'

        self.env['FAIL_TAG_PUSH'] = '1'
        Path(self.env['CALLS']).write_text('')
        Path(self.env['GITHUB_STEP_SUMMARY']).write_text('')
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertFalse(any(line.startswith('gh release') for line in self.calls()))
        self.assertIn('Incomplete', Path(self.env['GITHUB_STEP_SUMMARY']).read_text())

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
        for platform in ['both', 'macos', 'windows']:
            selected = []
            if platform != 'windows':
                selected += [self.env['DMG_NAME'], 'appcast.xml']
            if platform != 'macos':
                selected += [self.env['SETUP_NAME'], self.env['SETUP_NAME'] + '.sig']
            for variable, value in itertools.product(['FAIL_UPLOAD', 'FAIL_DOWNLOAD'], selected):
                with self.subTest(platform=platform, variable=variable, value=value):
                    self.env.update(PLATFORM=platform, FAIL_UPLOAD='', FAIL_DOWNLOAD='')
                    self.env[variable] = value
                    self.assets([])
                    Path(self.env['CALLS']).write_text('')
                    Path(self.env['GITHUB_STEP_SUMMARY']).write_text('')
                    self.assertNotEqual(self.publish().returncode, 0)
                    self.assertFalse(any('delete-asset' in line for line in self.calls()))
                    uploaded = selected[:selected.index(value)] if variable == 'FAIL_UPLOAD' else selected
                    self.assertEqual(self.published_assets(), set(uploaded))
                    summary = Path(self.env['GITHUB_STEP_SUMMARY']).read_text()
                    self.assertIn(f'Incomplete {platform} nightly {VERSION} from {SHA}', summary)
                    self.assertIn('inspect release nightly', summary)
                    self.assertNotIn('Published', summary)

    def test_public_verification_requires_selected_assets_and_public_prerelease_flags(self):
        step = next(step for step in JOBS['publish']['steps'] if step['name'].startswith('Verify the public'))
        selected = {
            'macos': [self.env['DMG_NAME'], 'appcast.xml'],
            'windows': [self.env['SETUP_NAME'], self.env['SETUP_NAME'] + '.sig'],
        }
        all_assets = selected['macos'] + selected['windows']
        for platform in ['both', 'macos', 'windows']:
            self.env['PLATFORM'] = platform
            required = all_assets if platform == 'both' else selected[platform]
            for missing in all_assets:
                with self.subTest(platform=platform, missing=missing):
                    self.assets([name for name in all_assets if name != missing])
                    result = self.shell(step['run'])
                    self.assertEqual(result.returncode == 0, missing not in required, result.stderr)
            for flag, value in [('isPrerelease', False), ('isDraft', True)]:
                self.assets(all_assets)
                path = Path(self.env['RELEASE_STATE'])
                release = json.loads(path.read_text())
                release[flag] = value
                path.write_text(json.dumps(release))
                self.assertNotEqual(self.shell(step['run']).returncode, 0)

    def test_retention_orders_mixed_versions_by_stamp_and_pairs_signatures(self):
        mac_old = [f'Runner-Nightly-9.0.0-nightly.202608{day:02}.0100-universal.dmg' for day in range(1, 12)]
        win_old = [f'Runner-Setup-9.0.0.202608{day:02}.0100-x64.exe' for day in range(1, 12)]
        existing = ['appcast.xml', 'unrelated.txt'] + list(reversed(mac_old)) + win_old + [name + '.sig' for name in win_old]
        for platform in ['both', 'macos', 'windows']:
            with self.subTest(platform=platform):
                self.env['PLATFORM'] = platform
                self.assets(existing)
                Path(self.env['CALLS']).write_text('')
                result = self.publish()
                self.assertEqual(result.returncode, 0, result.stderr)
                calls = self.calls()
                deleted = [line.split()[4] for line in calls if 'delete-asset' in line]
                expected_deleted = []
                expected_added = []
                if platform != 'windows':
                    expected_deleted += mac_old[:2]
                    expected_added += [self.env['DMG_NAME']]
                if platform != 'macos':
                    expected_deleted += win_old[:2] + [name + '.sig' for name in win_old[:2]]
                    expected_added += [self.env['SETUP_NAME'], self.env['SETUP_NAME'] + '.sig']
                self.assertEqual(set(deleted), set(expected_deleted))
                self.assertEqual(self.published_assets(), (set(existing) | set(expected_added)) - set(expected_deleted))
                first_delete = next(i for i, line in enumerate(calls) if 'delete-asset' in line)
                self.assertEqual(sum(line.startswith('curl ') for line in calls[:first_delete]), 4 if platform == 'both' else 2)

    def test_pruning_refuses_to_remove_current_appcast_target_or_installer(self):
        for platform in ['macos', 'windows']:
            with self.subTest(platform=platform):
                if platform == 'macos':
                    newer = [f'Runner-Nightly-0.8.3-nightly.202609{day:02}.0100-arm64.dmg' for day in range(9, 20)]
                else:
                    newer = [f'Runner-Setup-nightly.abc1234.202609{day:02}.0100-x64.exe' for day in range(9, 20)]
                    newer += [name + '.sig' for name in newer]
                self.assets(newer)
                Path(self.env['CALLS']).write_text('')
                result = self.publish()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('refusing to prune', result.stderr)
                self.assertFalse(any('delete-asset' in line for line in self.calls()))


if __name__ == '__main__':
    unittest.main()
