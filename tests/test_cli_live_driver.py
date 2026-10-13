"""Offline regression tests for the manual Live driver; no model or credential."""
import os
from pathlib import Path
import signal
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parent / "support"))
from cli_live_driver import CliSession, TurnStopped, failed_run


FAKE_CLI = r'''
import os,sys,time
from pathlib import Path
mode=sys.argv[1]
Path("child.pid").write_text(str(os.getpid()))
if mode == "timeout":
    time.sleep(30)
elif mode == "missing_id":
    print("you> ",end="",flush=True)
    for line in sys.stdin:
        pass
else:
    if mode == "inherited_pipe":
        import subprocess
        descendant = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"], stdin=subprocess.DEVNULL)
        Path("descendant.pid").write_text(str(descendant.pid))
    print("Started session `fake-session`\nyou> ",end="",flush=True)
    for line in sys.stdin:
        if mode == "stopped":
            print("[turn stopped] model stream protocol failed: fixed",file=sys.stderr,flush=True)
            print("Durable checkpoint retained. Use /resume to rebuild ProviderSession explicitly.\nyou> ",end="",flush=True)
        else:
            print("Echidna> done\n[request budget] admitted=1 primary=1 vision=0 remaining=7 limit=8\nyou> ",end="",flush=True)
    sys.exit(1 if mode in ("stopped","exit_failure") else 0)
'''


class DriverTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.addCleanup(self.directory.cleanup)
        self.fake = self.root / "fake.py"
        self.fake.write_text(FAKE_CLI)

    def start(self, mode, **kwargs):
        return CliSession([sys.executable, str(self.fake), mode], self.root,
                          self.root, mode, **kwargs)

    def test_normal_prompt_is_ready_but_not_accepted(self):
        chat = self.start("ok")
        try:
            record = chat.action("task", "one task", model=True)
            self.assertEqual(record["acceptance"], "not_evaluated")
        finally:
            self.assertEqual(chat.close(), 0)
        self.assertFalse(failed_run([chat.result()], []))

    def test_stopped_turn_is_preserved_and_process_failure_propagates(self):
        chat = self.start("stopped")
        try:
            with self.assertRaises(TurnStopped):
                chat.action("task", "one task", model=True)
        finally:
            self.assertEqual(chat.close(), 1)
        self.assertEqual(chat.actions[0]["status"], "stopped")
        self.assertEqual(len(chat.actions), 1)
        self.assertTrue(failed_run([chat.result()], []))
        self.assertIn("Durable checkpoint retained", (self.root/"task.result.txt").read_text())

    def test_exit_failure_is_not_hidden_by_a_ready_prompt(self):
        chat = self.start("exit_failure")
        chat.action("task", "one task", model=True)
        self.assertEqual(chat.close(), 1)
        self.assertTrue(failed_run([chat.result()], []))
        self.assertTrue(failed_run([], ["startup failed"]))

    def test_initialization_failure_reaps_child_and_closes_files(self):
        for mode in ("missing_id", "timeout"):
            with self.subTest(mode=mode):
                with self.assertRaises((RuntimeError, TimeoutError)):
                    self.start(mode, startup_timeout=0.4)
                pid = int((self.root/"child.pid").read_text())
                with self.assertRaises(ProcessLookupError):
                    os.kill(pid, 0)
                self.assertTrue((self.root/f"{mode}.stdout").exists())

    def test_inherited_output_pipe_does_not_block_cleanup_forever(self):
        chat = self.start("inherited_pipe")
        try:
            with self.assertRaisesRegex(TimeoutError, "output pipes remained open"):
                chat.close()
            self.assertTrue(chat.closed)
            self.assertEqual(chat.returncode, 0)
        finally:
            os.kill(int((self.root/"descendant.pid").read_text()), signal.SIGKILL)


if __name__ == "__main__":
    unittest.main()
