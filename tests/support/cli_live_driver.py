"""Local manual-validation helper; drives the existing CLI without retrying it."""
import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import time


class TurnStopped(RuntimeError):
    """The CLI retained a checkpoint and requires an explicit operator resume."""


class CliSession:
    def __init__(self, argv, cwd, transcript_dir, label, startup_timeout=20):
        self.label = label
        self.stdout = bytearray()
        self.stderr = bytearray()
        self.proc = None
        self.files = {}
        self.selector = selectors.DefaultSelector()
        self.closed = False
        self.actions = []
        self.session_id = None
        self.transcript_dir = Path(transcript_dir)
        try:
            for stream in ("stdout", "stderr"):
                self.files[stream] = (self.transcript_dir / f"{label}.{stream}").open("xb")
            self.proc = subprocess.Popen(
                argv, cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, env={**os.environ, "NO_COLOR": "1"},
            )
            self.selector.register(self.proc.stdout, selectors.EVENT_READ, "stdout")
            self.selector.register(self.proc.stderr, selectors.EVENT_READ, "stderr")
            self._wait_prompt(0, startup_timeout)
            match = re.search(r"session `([^`]+)`", self.stdout.decode())
            if not match:
                raise RuntimeError("CLI startup omitted its Session ID")
            self.session_id = match.group(1)
        except BaseException:
            self.close()
            raise

    def _pump(self, timeout):
        for key, _ in self.selector.select(timeout):
            data = os.read(key.fileobj.fileno(), 65536)
            if not data:
                self.selector.unregister(key.fileobj)
            else:
                getattr(self, key.data).extend(data)
                self.files[key.data].write(data)
                self.files[key.data].flush()

    def _wait_prompt(self, start, timeout):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self._pump(min(0.1, max(0, deadline - time.monotonic())))
            if bytes(self.stdout[start:]).endswith(b"you> "):
                # report_stop writes stderr before the flushed stdout prompt.
                self._pump(0)
                return
            if self.proc.poll() is not None:
                self._pump(0)
                raise RuntimeError(f"CLI exited before prompt: {self.proc.returncode}")
        raise TimeoutError("CLI deadline exceeded; no input or tool is retried")

    def action(self, label, value, model=False, timeout=600):
        if self.closed:
            raise RuntimeError("CLI session is closed")
        if "\n" in value or "\r" in value:
            raise ValueError("one CLI action must occupy one input line")
        record = {"action": label, "input": value, "model_task": model,
                  "session_before": self.session_id, "status": "running"}
        self.actions.append(record)
        start, err_start = len(self.stdout), len(self.stderr)
        began = time.monotonic()
        try:
            self.proc.stdin.write(value.encode() + b"\n")
            self.proc.stdin.flush()
            self._wait_prompt(start, timeout)
            stdout = self.stdout[start:].decode()
            stderr = self.stderr[err_start:].decode()
            record["prompt_ready"] = True
            if ("[turn stopped]" in stderr
                    or "Durable checkpoint retained." in stdout
                    or "Task input not accepted" in stdout):
                record["status"] = "stopped"
                raise TurnStopped("CLI requires explicit recovery; no retry sent")
            if "[request cancelled]" in stdout:
                record["status"] = "cancelled"
                raise RuntimeError("CLI turn was cancelled")
            ids = re.findall(r"session `([^`]+)`", stdout)
            if ids:
                self.session_id = ids[-1]
            if model and "[request budget] admitted=" not in stdout:
                raise RuntimeError("CLI omitted the model-turn budget outcome")
            # A ready prompt is not evidence of task/format correctness.
            record.update(status="returned_to_prompt", session_after=self.session_id,
                          acceptance="not_evaluated")
            return record
        except BaseException as exc:
            if record["status"] == "running":
                record["status"] = "failed"
            record["error"] = str(exc)
            raise
        finally:
            record["seconds"] = round(time.monotonic() - began, 3)
            record["stdout_bytes"] = len(self.stdout) - start
            record["stderr_bytes"] = len(self.stderr) - err_start
            (self.transcript_dir / f"{label}.result.txt").write_bytes(self.stdout[start:])

    def close(self):
        if self.closed:
            return self.returncode
        try:
            if self.proc is not None:
                if self.proc.poll() is None:
                    self.proc.stdin.close()
                    for timeout, next_signal in ((2, signal.SIGINT), (2, signal.SIGKILL)):
                        deadline = time.monotonic() + timeout
                        while self.proc.poll() is None and time.monotonic() < deadline:
                            self._pump(0.05)
                        if self.proc.poll() is not None:
                            break
                        self.proc.send_signal(next_signal)
                    self.proc.wait(timeout=5)
                drain_deadline = time.monotonic() + 2
                while self.selector.get_map() and time.monotonic() < drain_deadline:
                    self._pump(0.05)
                if self.selector.get_map():
                    raise TimeoutError("CLI output pipes remained open after process exit")
            return self.returncode
        finally:
            self.selector.close()
            for handle in self.files.values():
                handle.close()
            if self.proc is not None:
                for handle in (self.proc.stdin, self.proc.stdout, self.proc.stderr):
                    handle.close()
            self.closed = True

    @property
    def returncode(self):
        return None if self.proc is None else self.proc.poll()

    def result(self):
        return {"process": self.label, "returncode": self.returncode,
                "actions": self.actions}


def failed_run(process_results, setup_errors):
    return bool(setup_errors) or any(
        process["returncode"] != 0
        or any(action["status"] != "returned_to_prompt" for action in process["actions"])
        for process in process_results
    )
