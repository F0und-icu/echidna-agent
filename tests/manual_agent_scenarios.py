"""Manual seven-task validation on an explicitly prepared, fresh evidence directory.

This command sends real model requests through the configured Echidna CLI.
Run only with operator Live authorization. No setup or credential discovery.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys

sys.path.insert(0, str(Path(__file__).parent / "support"))
from cli_live_driver import CliSession, failed_run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", required=True, type=Path)
    args = parser.parse_args()
    base = args.evidence_dir.resolve()
    for name in ("run-result.json", "run-results-v2.json"):
        if (base/name).exists():
            raise SystemExit("This lane was already attempted; refusing replay")
    setup = json.loads((base/"setup.json").read_text())
    prompts = json.loads((base/"prompts.json").read_text())
    binary = Path(setup["binary"])
    if hashlib.sha256(binary.read_bytes()).hexdigest() != setup["binary_sha256"]:
        raise SystemExit("Prepared binary hash changed")
    argv = [str(binary), "--config", setup["private_config"], "chat"]
    processes, errors = [], []

    def save(finished=False):
        data = {"state": "finished" if finished else "in_progress",
                "processes": processes, "errors": errors,
                "failed": failed_run(processes, errors),
                "acceptance": "requires artifact and durable-event grading"}
        (base/"run-results-v2.json").write_text(json.dumps(data, ensure_ascii=False, indent=2))

    # Reserve this lane before any process can issue a request.
    save()

    def run(label, project, selection, work):
        chat = None
        try:
            chat = CliSession(argv + selection, base/project, base/"transcripts", label)
            processes.append(chat.result())
            work(chat)
        except Exception as exc:
            errors.append({"process": label, "error": str(exc)})
        finally:
            if chat is not None:
                try:
                    chat.close()
                except Exception as exc:
                    errors.append({"process": label, "error": f"CLI cleanup failed: {exc}"})
                finally:
                    processes[-1].update(chat.result())
            save()

    def task(chat, name):
        try:
            chat.action(name, prompts[name], model=True)
        finally:
            save()

    writer_session = None

    def lifecycle(chat):
        nonlocal writer_session
        writer_session = chat.session_id
        task(chat, "01-writer")
        shutil.copy2(base/"definitions/writer/persona.md", base/"artifacts/writer-persona-v1.md")
        shutil.copyfile(base/"artifacts/writer-persona-v2.md", base/"definitions/writer/persona.md")
        task(chat, "02-pinned")
        chat.action("reload", "/agent reload", timeout=30)
        task(chat, "03-reloaded")
        chat.action("compact", "/compact", timeout=30)
        chat.action("use-analyst", "/agent use " + str(base/"definitions/analyst"), timeout=30)
        task(chat, "04-analyst")

    run("lifecycle", "shared-project", ["--agent", str(base/"definitions/writer")], lifecycle)
    if not failed_run(processes, errors):
        retained = base/"artifacts/writer-source-retained"
        (base/"definitions/writer").rename(retained)
        (base/"artifacts/source-relocation.json").write_text(json.dumps({
            "original_path": str(base/"definitions/writer"), "retained_at": str(retained),
            "original_absent": not (base/"definitions/writer").exists(),
            "resume_session": writer_session,
        }, indent=2))
        run("resume", "shared-project", ["--resume", writer_session],
            lambda chat: task(chat, "05-resumed"))
    else:
        errors.append({"process": "resume", "error": "Skipped after failed lifecycle; no implicit recovery"})
        save()
    for name, project, prompt in (("reviewer", "review-project", "06-reviewer"),
                                 ("research", "research-project", "07-research")):
        run(name, project, ["--agent", str(base/"definitions"/name)],
            lambda chat, prompt=prompt: task(chat, prompt))
    save(finished=True)
    return int(failed_run(processes, errors))


if __name__ == "__main__":
    raise SystemExit(main())
