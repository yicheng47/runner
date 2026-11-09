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
PREVIEW_PATH = ROOT / '.github/workflows/preview-nightly.yml'
ON_MERGE = yaml.safe_load((ROOT / '.github/workflows/nightly-on-merge.yml').read_text())
STAMP = '20260908.0100'
SHA = '99e32efa36578ffc484a9cbb046b46203278f93b'
VERSION = f'nightly.{SHA[:7]}.{STAMP}'
SPARKLE = '{http://www.andymatuschak.org/xml-namespaces/sparkle}'
CHANNELS = {'nightly': ('Runner Nightly', 'Runner-Nightly'),
            'preview-nightly': ('Runner Preview Nightly', 'Runner-Preview-Nightly')}

MOCKS = r'''
gh() {
  printf 'gh %s\n' "$*" >> "$CALLS"
  if [[ "$1" == release && "$3" != "$CHANNEL" ]]; then
    echo "unexpected release: $3" >&2
    return 90
  fi
  case "$1 $2" in
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
      https:*/appcast*.xml) source="nightly-artifacts/macos/${1##*/}" ;;
      https:*/Runner_*_linux-*.sig) source="nightly-artifacts/linux/${1##*/}" ;;
      https:*/*.sig) source="nightly-artifacts/windows/$SETUP_NAME.sig" ;;
      --output) cp "$source" "$2"; return ;;
    esac
    shift
  done
  return 90
}
sleep() { :; }
yq() {
  [[ "$1 $2 $3" == "-o json .on.push.branches" ]] || { echo "unexpected yq call: $*" >&2; return 90; }
  python3 -c 'import json, sys, yaml; print(json.dumps(yaml.safe_load(open(sys.argv[1]))[True]["push"]["branches"]))' "$4"
}
git() {
  printf 'git %s\n' "$*" >> "$CALLS"
  case "$1" in
    rev-parse) printf '%s\n' "$SOURCE_SHA" ;;
    describe) [[ -n "$RELEASE_TAG" ]] && printf '%s\n' "$RELEASE_TAG" ;;
    log) printf '%s\n' "$CHANGELOG" | sed '/^$/d' ;;
    config|tag) return 0 ;;
    push) [[ -z "$FAIL_TAG_PUSH" ]] ;;
    *) echo "unexpected git call: $*" >&2; return 90 ;;
  esac
}
'''


def condition(expression, platform, prepare='success', macos='success', windows='success', cancelled=False, failure=False, linux='skipped'):
    values = {
        'inputs.platform': repr(platform),
        'needs.prepare.result': repr(prepare),
        'needs.build-macos.result': repr(macos),
        'needs.build-windows.result': repr(windows),
        'needs.build-linux.result': repr(linux),
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
        self.env = dict(os.environ, SOURCE_SHA=SHA, GITHUB_SHA=SHA, GITHUB_REF='refs/heads/main',
                        BUILD_STAMP=STAMP, NIGHTLY_SHORT_SHA=SHA[:7], PLATFORM='both',
                        RELEASE_EXISTS='true',
                        FAIL_UPLOAD='', FAIL_DOWNLOAD='', FAIL_TAG_PUSH='', RUNNER_TEMP=str(self.root),
                        RELEASE_TAG='v0.8.2',
                        CHANGELOG='- feat(nightly): one change (abc1234)\n- fix(ui): another (def5678)',
                        RELEASE_STATE=str(self.root / 'release.json'),
                        CALLS=str(self.root / 'calls'), GITHUB_OUTPUT=str(self.root / 'outputs'),
                        GITHUB_STEP_SUMMARY=str(self.root / 'summary'))
        (self.root / 'script').mkdir()
        (self.root / 'script/verify-nightly-appcast.py').symlink_to(ROOT / 'script/verify-nightly-appcast.py')
        for channel in CHANNELS:
            (self.root / f'script/{channel}-release-notes.md').symlink_to(ROOT / f'script/{channel}-release-notes.md')
        (self.root / '.github/workflows').mkdir(parents=True)
        self.mac = self.root / 'nightly-artifacts/macos'
        self.win = self.root / 'nightly-artifacts/windows'
        self.mac.mkdir(parents=True)
        self.win.mkdir(parents=True)
        self.use_channel('nightly')

    def use_channel(self, channel):
        title, dmg_prefix = CHANNELS[channel]
        version = f'{channel}.{SHA[:7]}.{STAMP}'
        self.env.update(CHANNEL=channel, RELEASE_TITLE=title, NIGHTLY_VERSION=version, DMG_PREFIX=dmg_prefix,
                        DMG_NAME=f'{dmg_prefix}-{SHA[:7]}.{STAMP}-arm64.dmg',
                        DMG_X64_NAME=f'{dmg_prefix}-{SHA[:7]}.{STAMP}-x64.dmg',
                        SETUP_NAME=f'Runner-Setup-{version}-x64.exe')
        dmgs = {'arm64': self.env['DMG_NAME'], 'x64': self.env['DMG_X64_NAME']}
        for arch, dmg in dmgs.items():
            (self.mac / dmg).write_bytes(f'{arch} nightly dmg fixture'.encode())
        self.mac_assets = [self.env['DMG_NAME'], 'appcast.xml', self.env['DMG_X64_NAME'], 'appcast-x64.xml']
        (self.win / self.env['SETUP_NAME']).write_bytes(b'nightly installer fixture')
        (self.win / (self.env['SETUP_NAME'] + '.sig')).write_bytes(b'installer signature fixture')
        self.linux = self.root / 'nightly-artifacts/linux'
        self.linux.mkdir(parents=True, exist_ok=True)
        self.linux_names = [f'Runner_{version}_linux-{arch}.{ext}' for arch in ['x64', 'arm64'] for ext in ['tar.gz', 'deb']]
        self.linux_names += [name + ".sig" for name in self.linux_names]
        for name in self.linux_names:
            (self.linux / name).write_bytes(b'Linux package fixture')
        self.appcasts = {'arm64': self.mac / 'appcast.xml', 'x64': self.mac / 'appcast-x64.xml'}
        release_url = f'https://github.com/yicheng47/runner/releases/tag/{channel}'
        self.items = {}
        for arch, dmg in dmgs.items():
            item = ET.SubElement(ET.SubElement(ET.Element('rss'), 'channel'), 'item')
            fields = [('version', STAMP), ('shortVersionString', SHA[:7])]
            if arch == 'arm64':
                fields.append(('hardwareRequirements', 'arm64'))
            for field, value in fields + [('fullReleaseNotesLink', release_url)]:
                ET.SubElement(item, SPARKLE + field).text = value
            ET.SubElement(item, 'link').text = release_url
            ET.SubElement(item, 'enclosure', {
                'url': f"https://github.com/yicheng47/runner/releases/download/{channel}/{dmg}",
                'length': str((self.mac / dmg).stat().st_size),
                SPARKLE + 'edSignature': base64.b64encode(b'x' * 64).decode(),
            })
            self.items[arch] = item
        self.write_appcasts()
        self.assets([])

    def write_appcasts(self):
        for arch, item in self.items.items():
            root = ET.Element('rss')
            ET.SubElement(root, 'channel').append(item)
            ET.ElementTree(root).write(self.appcasts[arch])

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
        self.assertEqual(WORKFLOW[True]['workflow_dispatch']['inputs']['platform']['default'], 'all')
        self.assertEqual(WORKFLOW[True]['workflow_dispatch']['inputs']['channel']['default'], 'nightly')
        self.assertEqual(WORKFLOW[True]['workflow_dispatch']['inputs']['channel']['options'], list(CHANNELS))
        self.assertEqual(WORKFLOW['concurrency'], {'group': '${{ inputs.channel }}', 'cancel-in-progress': True})
        self.assertIn('!cancelled()', JOBS['publish']['if'])
        results = ['success', 'failure', 'cancelled', 'skipped']
        for platform, prepare, macos, windows, cancelled in itertools.product(
                ['both', 'macos', 'windows'], results, results, results, [False, True]):
            expected_builds = {'both': ('success', 'success'), 'macos': ('success', 'skipped'),
                               'windows': ('skipped', 'success')}[platform]
            expected = not cancelled and prepare == 'success' and (macos, windows) == expected_builds
            with self.subTest(platform=platform, prepare=prepare, macos=macos, windows=windows, cancelled=cancelled):
                self.assertEqual(condition(JOBS['publish']['if'], platform, prepare, macos, windows, cancelled), expected)

    def test_all_and_linux_publish_gates_cover_each_selected_build(self):
        results = ['success', 'failure', 'cancelled', 'skipped']
        for platform, macos, windows, linux, cancelled in itertools.product(
                ['all', 'linux'], results, results, results, [False, True]):
            expected = not cancelled and linux == 'success' and (
                (platform == 'all' and macos == windows == 'success') or
                (platform == 'linux' and macos == windows == 'skipped'))
            self.assertEqual(condition(JOBS['publish']['if'], platform, macos=macos, windows=windows, linux=linux, cancelled=cancelled), expected)

    def test_linux_and_all_publication_require_both_architectures_and_never_touch_stable_feeds(self):
        for platform in ['linux', 'all']:
            self.env.update(PLATFORM=platform, FAIL_UPLOAD='', FAIL_DOWNLOAD='')
            self.assets([])
            Path(self.env['CALLS']).write_text('')
            result = self.publish()
            self.assertEqual(result.returncode, 0, result.stderr)
            expected = set(self.linux_names)
            if platform == 'all':
                expected.update(self.mac_assets + [self.env['SETUP_NAME'], self.env['SETUP_NAME'] + '.sig'])
            self.assertEqual(self.published_assets(), expected)
            self.assertFalse(any('latest' in line for line in self.calls() if line.startswith('gh release upload')))
            for name in self.linux_names:
                for variable in ['FAIL_UPLOAD', 'FAIL_DOWNLOAD']:
                    self.env.update(FAIL_UPLOAD='', FAIL_DOWNLOAD='')
                    self.env[variable] = name
                    self.assets([])
                    Path(self.env['CALLS']).write_text('')
                    self.assertNotEqual(self.publish().returncode, 0)
                    self.assertFalse(any('delete-asset' in line for line in self.calls()))
            self.env.update(FAIL_UPLOAD='', FAIL_DOWNLOAD='')
            self.assertFalse(any(line.startswith('gh run') for line in self.calls()))
            for missing in [self.linux_names[0], self.linux_names[-1]]:
                removed = self.linux / missing
                removed.rename(removed.with_suffix('.saved'))
                try:
                    self.assets([])
                    Path(self.env['CALLS']).write_text('')
                    self.assertNotEqual(self.publish().returncode, 0)
                    self.assertEqual(self.calls(), [])
                finally:
                    removed.with_suffix('.saved').rename(removed)

    def test_linux_retention_keeps_one_stamp_and_does_not_prune_other_platforms(self):
        self.env['PLATFORM'] = 'linux'
        old = [f'Runner_nightly.abc1234.202608{day:02}.0100_linux-{arch}.{ext}'
               for day in [1, 2] for arch in ['x64', 'arm64'] for ext in ['tar.gz', 'deb']]
        old += [name + ".sig" for name in old]
        foreign = ['unrelated.txt', 'appcast.xml', 'appcast-x64.xml', 'Runner-Nightly-abc1234.20260801.0100-arm64.dmg',
                   'Runner-Nightly-abc1234.20260801.0100-x64.dmg', 'Runner-Setup-nightly.abc1234.20260801.0100-x64.exe']
        self.assets(old + foreign)
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.published_assets(), set(self.linux_names) | set(foreign))

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
        self.assertEqual(outputs['title'], 'Runner Nightly')
        self.assertEqual(outputs['dmg_prefix'], 'Runner-Nightly')

    def preview_list(self, branches):
        path = self.root / '.github/workflows/preview-nightly.yml'
        if branches is None:
            path.unlink(missing_ok=True)
        else:
            path.write_text('on:\n  push:\n    branches: ' + json.dumps(branches) + '\n')

    def test_each_release_is_cut_only_from_its_own_branches(self):
        step = next(step for step in JOBS['prepare']['steps'] if step.get('id') == 'build')
        umbrellas = ['feat/1-first-umbrella', 'feat/2-second']
        cases = [(umbrellas, 'nightly', 'refs/heads/main', True),
                 (umbrellas, 'nightly', 'refs/heads/feat/1-first-umbrella', False),
                 (umbrellas, 'other', 'refs/heads/main', False),
                 (umbrellas, 'preview-nightly', 'refs/heads/main', False),
                 (umbrellas, 'preview-nightly', 'refs/heads/feat/1', False),
                 (umbrellas, 'preview-nightly', 'refs/heads/feat/1-first-umbrella-old', False),
                 (umbrellas, 'preview-nightly', 'refs/tags/feat/1-first-umbrella', False),
                 (umbrellas[:1], 'preview-nightly', 'refs/heads/feat/2-second', False),
                 ([], 'preview-nightly', 'refs/heads/feat/1-first-umbrella', False),
                 (None, 'preview-nightly', 'refs/heads/feat/1-first-umbrella', False),
                 (None, 'nightly', 'refs/heads/main', True)]
        cases += [(umbrellas, 'preview-nightly', f'refs/heads/{branch}', True) for branch in umbrellas]
        for branches, channel, ref, allowed in cases:
            with self.subTest(branches=branches, channel=channel, ref=ref):
                self.preview_list(branches)
                self.env.update(CHANNEL=channel, GITHUB_REF=ref)
                (self.root / 'outputs').write_text('')
                result = self.shell(step['run'])
                self.assertEqual(result.returncode == 0, allowed, result.stderr)
                outputs = dict(line.split('=', 1) for line in (self.root / 'outputs').read_text().splitlines())
                if allowed:
                    title, dmg_prefix = CHANNELS[channel]
                    self.assertEqual(outputs['version'], f'{channel}.{SHA[:7]}.' + outputs['stamp'])
                    self.assertEqual((outputs['title'], outputs['dmg_prefix']), (title, dmg_prefix))
                else:
                    self.assertEqual(outputs, {})

    def test_merges_still_cut_nightly_from_main_only(self):
        self.assertEqual(ON_MERGE[True]['push']['branches'], ['main'])
        merge_dispatch = ON_MERGE['jobs']['dispatch']['steps'][0]['run']
        self.assertIn('--ref main -f platform=all', merge_dispatch)
        self.assertNotIn('channel', merge_dispatch)

    def test_preview_pushes_dispatch_an_all_platform_preview_cut_of_the_pushed_umbrella(self):
        if not PREVIEW_PATH.exists():
            self.skipTest('no umbrella is listed for preview nightlies')
        preview = yaml.safe_load(PREVIEW_PATH.read_text())
        push = preview[True]['push']
        # The prepare guard matches names exactly, so the trigger must not use patterns.
        self.assertTrue(push['branches'])
        for branch in push['branches']:
            self.assertRegex(branch, r'^[A-Za-z0-9._/-]+$')
            self.assertNotEqual(branch, 'main')
        self.assertEqual(push['paths-ignore'], ON_MERGE[True]['push']['paths-ignore'])
        self.assertEqual(push['paths-ignore'], ['docs/**', '**/*.md'])
        self.assertEqual(preview['permissions'], {'actions': 'write', 'contents': 'read'})
        [step] = preview['jobs']['dispatch']['steps']
        self.env.update(GITHUB_REPOSITORY='yicheng47/runner', GITHUB_REF_NAME='feat/1-any-umbrella')
        mock = 'gh() { printf "gh %s\\n" "$*" >> "$CALLS"; }\n'
        result = subprocess.run(['bash', '-euo', 'pipefail', '-c', mock + step['run']],
                                cwd=self.root, env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls(), ['gh workflow run nightly.yml --repo yicheng47/runner --ref feat/1-any-umbrella -f channel=preview-nightly -f platform=all'])

    def test_preview_nightly_publishes_only_to_its_own_release_tag_and_feeds(self):
        self.use_channel('preview-nightly')
        self.env.update(PLATFORM='all', GITHUB_REF='refs/heads/feat/1-any-umbrella')
        old_stamps = [f'202608{day:02}.0100' for day in range(1, 4)]
        mac_old = [f'Runner-Preview-Nightly-abc1234.{stamp}-{arch}.dmg' for arch in ['arm64', 'x64'] for stamp in old_stamps]
        win_old = [f'Runner-Setup-preview-nightly.abc1234.{stamp}-x64.exe' for stamp in old_stamps]
        linux_old = [f'Runner_preview-nightly.abc1234.{old_stamps[0]}_linux-{arch}.{ext}'
                     for arch in ['x64', 'arm64'] for ext in ['tar.gz', 'deb']]
        foreign = ['Runner-Nightly-abc1234.20260801.0100-arm64.dmg', 'Runner-Nightly-abc1234.20260801.0100-x64.dmg',
                   'Runner_nightly.abc1234.20260801.0100_linux-x64.deb']
        self.assets(mac_old + win_old + [name + '.sig' for name in win_old] + linux_old + [name + '.sig' for name in linux_old] + foreign)
        result = self.publish()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        release_calls = [line for line in calls if line.startswith('gh release ')]
        self.assertTrue(release_calls)
        self.assertTrue(all(line.split()[3] == 'preview-nightly' for line in release_calls))
        self.assertIn(f'git tag --force --annotate preview-nightly --message Runner Preview Nightly preview-nightly.{SHA[:7]}.{STAMP} {SHA}', calls)
        self.assertIn('git push --force origin refs/tags/preview-nightly', calls)
        self.assertFalse(any('refs/tags/nightly' in line or '/download/nightly/' in line for line in calls))
        curls = [line for line in calls if line.startswith('curl ')]
        self.assertTrue(curls)
        self.assertTrue(all('/releases/download/preview-nightly/' in line for line in curls))
        edits = [line for line in calls if line.startswith(('gh release create', 'gh release edit'))]
        self.assertTrue(edits and all('--title Runner Preview Nightly' in line for line in edits))
        self.assertTrue((self.root / 'nightly-notes.md').read_text().endswith((ROOT / 'script/preview-nightly-release-notes.md').read_text()))
        deleted = {line.split()[4] for line in calls if 'delete-asset' in line}
        self.assertEqual(deleted, set([name for name in mac_old if old_stamps[2] not in name] + win_old[:2] + [name + '.sig' for name in win_old[:2]]
                                      + linux_old + [name + '.sig' for name in linux_old]))
        self.assertTrue(set(foreign) <= self.published_assets())
        self.assertTrue(set(self.linux_names) <= self.published_assets())
        self.assertIn('to release preview-nightly.', Path(self.env['GITHUB_STEP_SUMMARY']).read_text())

    def test_bash_syntax_and_shared_source_artifacts(self):
        self.assertIsNone(re.search(r'nightly-win\b', (ROOT / '.github/workflows/nightly.yml').read_text()))
        for name, job in JOBS.items():
            checkout = next(step for step in job['steps'] if step.get('uses', '').startswith('actions/checkout@'))
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
                if name == 'build-windows':
                    download = next(step for step in JOBS['publish']['steps'] if step.get('with', {}).get('name') == upload['with']['name'])
                    self.assertEqual(set(download['with']), {'name', 'path'})
                    self.assertIn('${{ needs.prepare.outputs.version }}', upload['with']['path'])
                    continue
                prefix = upload['with']['name'].removesuffix('-${{ matrix.arch }}')
                download = next(step for step in JOBS['publish']['steps'] if step.get('with', {}).get('pattern') == prefix + '-*')
                self.assertTrue(download['with']['merge-multiple'])
                if name == 'build-linux':
                    self.assertEqual(job['container'], 'ubuntu:22.04')
                    continue
                self.assertIn('${{ needs.prepare.outputs.short_sha }}', upload['with']['path'])

    def test_each_macos_architecture_builds_with_its_own_feed(self):
        job = JOBS['build-macos']
        self.assertEqual(job['strategy']['matrix']['include'], [
            {'arch': 'arm64', 'target': 'aarch64-apple-darwin', 'appcast': 'appcast.xml'},
            {'arch': 'x64', 'target': 'x86_64-apple-darwin', 'appcast': 'appcast-x64.xml'},
        ])
        build = next(step for step in job['steps'] if 'RUNNER_BUILD_STAMP' in step.get('env', {}))
        self.assertIn('--arch ${{ matrix.arch }}', build['run'])
        upload = next(step for step in job['steps'] if step.get('uses', '').startswith('actions/upload-artifact@'))
        self.assertIn('-${{ matrix.arch }}.dmg', upload['with']['path'])
        self.assertIn('/${{ matrix.appcast }}', upload['with']['path'])

    def test_versions_and_lockfile_agree(self):
        lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
        version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
        for name, path in [
            ('runner-app', 'crates/runner-app'),
            ('runner-daemon', 'crates/runner-daemon'),
            ('runner-terminal', 'crates/runner-terminal'),
            ('runner-core', 'crates/runner-core'),
            ('runner-cli', 'crates/runner-cli'),
        ]:
            manifest = tomllib.loads((ROOT / path / 'Cargo.toml').read_text())
            self.assertEqual(manifest['package']['version'], {'workspace': True}, name)
            self.assertEqual(next(p['version'] for p in lock['package'] if p['name'] == name), version, name)

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
                    expected += self.mac_assets
                if platform != 'macos':
                    expected += [self.env['SETUP_NAME'], self.env['SETUP_NAME'] + '.sig']
                self.assertEqual([Path(line.split()[4]).name for line in uploads], expected)
                self.assertTrue(all(line.split()[3] == 'nightly' for line in uploads))
                self.assertEqual(self.published_assets(), set(expected))
                self.assertEqual(sum(line.startswith('gh release view nightly --json isPrerelease,isDraft,assets') for line in calls), 1)
                self.assertFalse(any(line.startswith('gh run') for line in calls))
                edits = [line for line in calls if line.startswith(('gh release create', 'gh release edit'))]
                self.assertEqual(len(edits), 1)
                self.assertTrue(all(f"--notes-file {self.root}/nightly-notes.md" in line for line in edits))
                tag_push = calls.index('git push --force origin refs/tags/nightly')
                self.assertLess(tag_push, calls.index(edits[0]))
                self.assertIn(f'git tag --force --annotate nightly --message Runner Nightly {VERSION} {SHA}', calls)
                self.assertTrue(all('--prerelease' in line and '--latest=false' in line for line in edits))
                self.assertTrue(all('--target ' + SHA in line for line in edits))
                self.assertFalse(any('nightly-win' in line for line in calls))

    def test_release_notes_carry_the_changelog_and_a_failed_tag_push_blocks_the_release(self):
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

        self.env.update(RELEASE_TAG='v0.8.2', FAIL_TAG_PUSH='1')
        Path(self.env['CALLS']).write_text('')
        Path(self.env['GITHUB_STEP_SUMMARY']).write_text('')
        self.assertNotEqual(self.publish().returncode, 0)
        self.assertFalse(any(line.startswith('gh release') for line in self.calls()))
        self.assertIn('Incomplete', Path(self.env['GITHUB_STEP_SUMMARY']).read_text())

    def test_missing_artifact_and_invalid_feed_block_mutation(self):
        for filename in [self.mac / self.env['DMG_NAME'], self.mac / self.env['DMG_X64_NAME'], *self.appcasts.values(),
                         self.win / self.env['SETUP_NAME'], self.win / (self.env['SETUP_NAME'] + '.sig')]:
            data = filename.read_bytes()
            filename.unlink()
            result = self.publish()
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(self.calls(), [])
            filename.write_bytes(data)
        other_dmg = {'arm64': self.env['DMG_X64_NAME'], 'x64': self.env['DMG_NAME']}
        for arch, item in self.items.items():
            fields = [('version', '20260901.0100'), ('shortVersionString', '0.8.2'), ('fullReleaseNotesLink', 'https://example.com')]
            if arch == 'arm64':
                fields.append(('hardwareRequirements', 'x86_64'))
            for field, wrong in fields:
                node = item.find(SPARKLE + field)
                previous = node.text
                node.text = wrong
                self.write_appcasts()
                self.assertNotEqual(self.publish().returncode, 0, (arch, field))
                node.text = previous
            if arch == 'x64':
                requirement = ET.SubElement(item, SPARKLE + 'hardwareRequirements')
                requirement.text = 'arm64'
                self.write_appcasts()
                self.assertNotEqual(self.publish().returncode, 0, 'Intel feed requiring arm64')
                item.remove(requirement)
            self.write_appcasts()
            enclosure = item.find('enclosure')
            for field, wrong in [('url', 'https://github.com/yicheng47/runner/releases/latest/download/Runner.dmg'),
                                 ('url', f"https://github.com/yicheng47/runner/releases/download/nightly/{other_dmg[arch]}"),
                                 ('length', '999'), (SPARKLE + 'edSignature', '')]:
                previous = enclosure.get(field)
                enclosure.set(field, wrong)
                self.write_appcasts()
                self.assertNotEqual(self.publish().returncode, 0, (arch, field))
                enclosure.set(field, previous)
            self.write_appcasts()
        self.assertEqual(self.calls(), [])

    def test_partial_upload_or_public_verification_failure_never_prunes(self):
        for platform in ['both', 'macos', 'windows']:
            selected = []
            if platform != 'windows':
                selected += self.mac_assets
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
            'macos': self.mac_assets,
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

    def test_public_verification_retries_transient_failures(self):
        step = next(step for step in JOBS['publish']['steps'] if step['name'].startswith('Verify the public'))
        self.assets(self.mac_assets + [self.env['SETUP_NAME'], self.env['SETUP_NAME'] + '.sig'])
        result = self.shell(step['run'])
        self.assertEqual(result.returncode, 0, result.stderr)
        curls = [line for line in self.calls() if line.startswith('curl ')]
        self.assertEqual(len(curls), 6)
        self.assertTrue(all('--retry 5 --retry-delay 10' in line for line in curls), curls)

    def test_retention_orders_mixed_versions_by_stamp_and_pairs_signatures(self):
        mac_old = {arch: [f'Runner-Nightly-9.0.0-nightly.202608{day:02}.0100-{arch}.dmg' for day in range(1, 12)]
                   for arch in ['arm64', 'x64']}
        win_old = [f'Runner-Setup-9.0.0.202608{day:02}.0100-x64.exe' for day in range(1, 12)]
        existing = (['appcast.xml', 'appcast-x64.xml', 'unrelated.txt'] + list(reversed(mac_old['arm64'])) + mac_old['x64']
                    + win_old + [name + '.sig' for name in win_old])
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
                    expected_deleted += mac_old['arm64'][:-1] + mac_old['x64'][:-1]
                    expected_added += [self.env['DMG_NAME'], self.env['DMG_X64_NAME']]
                if platform != 'macos':
                    expected_deleted += win_old[:-1] + [name + '.sig' for name in win_old[:-1]]
                    expected_added += [self.env['SETUP_NAME'], self.env['SETUP_NAME'] + '.sig']
                self.assertEqual(set(deleted), set(expected_deleted))
                self.assertEqual(self.published_assets(), (set(existing) | set(expected_added)) - set(expected_deleted))
                first_delete = next(i for i, line in enumerate(calls) if 'delete-asset' in line)
                self.assertEqual(sum(line.startswith('curl ') for line in calls[:first_delete]),
                                 {'both': 6, 'macos': 4, 'windows': 2}[platform])

    def test_pruning_refuses_to_remove_current_appcast_target_or_installer(self):
        for platform in ['macos-arm64', 'macos-x64', 'windows']:
            with self.subTest(platform=platform):
                if platform.startswith('macos-'):
                    arch = platform.removeprefix('macos-')
                    newer = [f'Runner-Nightly-0.8.3-nightly.202609{day:02}.0100-{arch}.dmg' for day in range(9, 20)]
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
