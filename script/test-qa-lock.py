#!/usr/bin/env python3

# Python 3.8+, no dependencies; run with `python3 script/test-qa-lock.py -v`.

import importlib.machinery
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

SCRIPT = Path(__file__).resolve().parent / 'qa-lock'
sys.dont_write_bytecode = True
_loader = importlib.machinery.SourceFileLoader('qa_lock', str(SCRIPT))
qa_lock = importlib.util.module_from_spec(importlib.util.spec_from_loader('qa_lock', _loader))
_loader.exec_module(qa_lock)


class QaLockTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.lock = Path(self.tmp.name) / 'nested' / 'qa.lock'
        self.env = dict(os.environ, RUNNER_QA_LOCK=str(self.lock))
        for name in ('RUNNER_MISSION_ID', 'RUNNER_HANDLE'):
            self.env.pop(name, None)
        self.owners = []

    def tearDown(self):
        for owner in self.owners:
            if owner.poll() is None:
                owner.kill()
            owner.wait()
        self.wait_free()
        self.tmp.cleanup()

    def owner(self):
        process = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(120)'])
        self.owners.append(process)
        return process

    def qa_lock(self, *args):
        return subprocess.run([sys.executable, str(SCRIPT), *args], env=self.env,
                              capture_output=True, text=True, timeout=60)

    def acquire(self, mission, handle, owner, timeout=0):
        return self.qa_lock('acquire', '--mission', mission, '--handle', handle,
                            '--owner-pid', str(owner.pid), '--timeout', str(timeout))

    def status(self):
        result = self.qa_lock('status')
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def holder_pid(self):
        return json.loads(self.lock.read_text())['pid']

    def wait_free(self, seconds=10):
        deadline = time.monotonic() + seconds
        while self.lock.exists() and self.status() != 'qa lock is free':
            if time.monotonic() >= deadline:
                self.fail('the lock was not freed: ' + self.status())
            time.sleep(0.2)

    def test_acquire_status_and_release(self):
        self.assertEqual(self.status(), 'qa lock is free')
        owner = self.owner()
        result = self.acquire('m-a', 'qa', owner)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('acquired for mission m-a, handle qa', result.stdout)
        status = self.status()
        self.assertRegex(status, r'^qa lock is held by mission m-a, handle qa, holder PID \d+, '
                                 r'owner PID {} \(python[\d.]*w?\), since \d{{4}}-'.format(owner.pid))
        again = self.acquire('m-a', 'qa', owner)
        self.assertEqual(again.returncode, 0)
        self.assertIn('already held by this owner', again.stdout)
        result = self.qa_lock('release', '--mission', 'm-a', '--handle', 'qa')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.status(), 'qa lock is free')

    def test_contention_times_out_and_names_the_holder(self):
        self.assertEqual(self.acquire('m-a', 'qa', self.owner()).returncode, 0)
        started = time.monotonic()
        result = self.acquire('m-b', 'qa', self.owner(), timeout=1)
        self.assertGreaterEqual(time.monotonic() - started, 1)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn('timed out after 1.0s; qa lock is held by mission m-a, handle qa', result.stdout)
        self.assertIn('mission m-a', self.status())

    def test_waiting_acquire_gets_the_lock_once_released(self):
        self.assertEqual(self.acquire('m-a', 'qa', self.owner()).returncode, 0)
        waiting = subprocess.Popen([sys.executable, str(SCRIPT), 'acquire', '--mission', 'm-b',
                                    '--handle', 'qa', '--owner-pid', str(self.owner().pid),
                                    '--timeout', '20'],
                                   env=self.env, stdout=subprocess.PIPE, text=True)
        time.sleep(1)
        self.assertEqual(self.qa_lock('release', '--mission', 'm-a', '--handle', 'qa').returncode, 0)
        out, _ = waiting.communicate(timeout=30)
        self.assertEqual(waiting.returncode, 0, out)
        self.assertIn('mission m-b', self.status())

    def test_release_refuses_another_owners_lock(self):
        self.assertEqual(self.acquire('m-a', 'qa', self.owner()).returncode, 0)
        for mission, handle in (('m-b', 'qa'), ('m-a', 'coder')):
            result = self.qa_lock('release', '--mission', mission, '--handle', handle)
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertIn("refusing to release another owner's lock", result.stdout)
        self.assertIn('mission m-a', self.status())

    def test_release_of_a_free_lock_succeeds(self):
        result = self.qa_lock('release', '--mission', 'm-a', '--handle', 'qa')
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), 'qa lock is free')

    def test_owner_exit_frees_the_lock(self):
        owner = self.owner()
        self.assertEqual(self.acquire('m-a', 'qa', owner).returncode, 0)
        owner.kill()
        owner.wait()
        self.wait_free()
        self.assertEqual(self.acquire('m-b', 'qa', self.owner()).returncode, 0)
        self.assertIn('mission m-b', self.status())

    def test_unreaped_owner_frees_the_lock(self):
        owner = self.owner()
        self.assertEqual(self.acquire('m-a', 'qa', owner).returncode, 0)
        owner.kill()
        self.wait_free()
        self.assertIsNone(owner.returncode)

    def test_handover_never_shows_the_previous_record(self):
        self.assertEqual(self.acquire('m-a', 'qa', self.owner()).returncode, 0)
        self.assertEqual(self.qa_lock('release', '--mission', 'm-a', '--handle', 'qa').returncode, 0)
        self.assertIn('"mission": "m-a"', self.lock.read_text())
        waiting = subprocess.Popen([sys.executable, str(SCRIPT), 'acquire', '--mission', 'm-b',
                                    '--handle', 'qa', '--owner-pid', str(self.owner().pid)],
                                   env=dict(self.env, RUNNER_QA_LOCK_TEST_PUBLISH_DELAY='3'),
                                   stdout=subprocess.PIPE, text=True)
        fd = os.open(str(self.lock), os.O_RDWR | getattr(os, 'O_BINARY', 0))
        try:
            deadline = time.monotonic() + 20
            while qa_lock.try_lock(fd):
                qa_lock.unlock(fd)
                self.assertLess(time.monotonic(), deadline, 'm-b never took the lock')
                time.sleep(0.05)
        finally:
            os.close(fd)
        self.assertIn('"mission": "m-a"', self.lock.read_text())
        result = self.acquire('m-a', 'qa', self.owner())
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn('held by mission m-b', result.stdout)
        out, _ = waiting.communicate(timeout=30)
        self.assertEqual(waiting.returncode, 0, out)
        refused = self.qa_lock('release', '--mission', 'm-a', '--handle', 'qa')
        self.assertEqual(refused.returncode, 1, refused.stdout)

    def test_killed_holder_leaves_a_file_that_reads_free(self):
        self.assertEqual(self.acquire('m-a', 'qa', self.owner()).returncode, 0)
        holder = self.holder_pid()
        if os.name == 'nt':
            subprocess.run(['taskkill', '/F', '/PID', str(holder)], capture_output=True, check=True)
        else:
            os.kill(holder, 9)
        self.wait_free()
        self.assertIn('"mission": "m-a"', self.lock.read_text())
        self.assertEqual(self.acquire('m-b', 'qa', self.owner()).returncode, 0)
        self.assertIn('mission m-b', self.status())

    def test_dead_owner_is_rejected(self):
        owner = self.owner()
        owner.kill()
        owner.wait()
        result = self.acquire('m-a', 'qa', owner)
        self.assertEqual(result.returncode, 2)
        self.assertIn('is not running', result.stderr)
        self.assertEqual(self.status(), 'qa lock is free')

    def test_identity_defaults_to_the_runner_environment(self):
        self.env.update(RUNNER_MISSION_ID='m-env', RUNNER_HANDLE='qa')
        result = self.qa_lock('acquire', '--owner-pid', str(self.owner().pid))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('mission m-env', self.status())
        self.assertEqual(self.qa_lock('release').returncode, 0)
        self.assertEqual(self.status(), 'qa lock is free')


class PollOwnerTest(unittest.TestCase):
    def watch(self, stats):
        with mock.patch.object(qa_lock, 'linux_process', side_effect=stats), \
                mock.patch.object(qa_lock.time, 'sleep') as sleep:
            wait = qa_lock.poll_owner(4242)
            if wait is not None:
                wait()
            return wait, sleep.call_count

    def test_reused_pid_counts_as_the_owner_exiting(self):
        wait, polls = self.watch([('S', '100'), ('S', '100'), ('S', '100'), ('R', '900')])
        self.assertIsNotNone(wait)
        self.assertEqual(polls, 2)

    def test_exited_or_zombie_owner_stops_the_wait(self):
        self.assertEqual(self.watch([('S', '100'), ('S', '100'), None])[1], 1)
        self.assertIsNone(self.watch([None])[0])


if __name__ == '__main__':
    unittest.main()
