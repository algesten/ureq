"""Verify the Linux EINTR failure and compare it with PR #1205."""

import json
import os
from pathlib import Path
import subprocess
import sys


def main():
    if sys.platform != "linux":
        raise SystemExit("The before/after assertions require Linux.")
    if len(sys.argv) != 3:
        raise SystemExit("Usage: python3 check.py BASELINE_CHECKOUT FIXED_CHECKOUT")

    root = Path(__file__).resolve().parent
    target = Path(os.environ.get("CARGO_TARGET_DIR", root / "target")).resolve()
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), UREQ_REPRO_REQUESTS="2000")
    interrupted = "I/O kind: Interrupted; OS error: Some(4)"
    results = []

    def build(source):
        patch = "patch.crates-io.ureq.path=" + json.dumps(str(Path(source).resolve()))
        subprocess.run(
            [
                "cargo", "build", "--release", "--locked", "--bins",
                "--manifest-path", str(root / "Cargo.toml"), "--config", patch,
            ],
            cwd=root,
            env=env,
            check=True,
            timeout=180,
        )

    def run(label, binary, args=(), expect_interrupt=False, race=False):
        print(label, flush=True)
        result = subprocess.run(
            [str(target / "release" / binary), *args],
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=60,
        )
        print(result.stdout, end="", flush=True)
        if expect_interrupt or (race and result.returncode != 0):
            if result.returncode != 1 or interrupted not in result.stdout:
                raise RuntimeError(f"{label}: expected EINTR, got exit {result.returncode}")
            outcome = "EINTR reproduced"
        else:
            result.check_returncode()
            if "SUCCESS" not in result.stdout and "No failure observed" not in result.stdout:
                raise RuntimeError(f"{label}: no successful result reported")
            outcome = "Race not observed" if race else "Passed"
        results.append((label, outcome))

    build(sys.argv[1])
    run("Current ureq: signal with timeout", "ureq-eintr-repro", expect_interrupt=True)
    run("Current ureq: signal without timeout", "ureq-eintr-repro", ["--no-timeout"])
    run("Current ureq: subprocesses with timeout", "spawn", race=True)
    run("Current ureq: no subprocesses", "spawn", ["--no-spawn"])
    run("Current ureq: subprocesses without timeout", "spawn", ["--no-timeout"])

    build(sys.argv[2])
    run("PR #1205: signal with timeout", "ureq-eintr-repro")
    run("PR #1205: subprocesses with timeout", "spawn")

    summary = "| Reproduction | Result |\n| --- | --- |\n"
    summary += "".join(f"| {label} | {outcome} |\n" for label, outcome in results)
    print(summary)
    if "GITHUB_STEP_SUMMARY" in os.environ:
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as output:
            output.write(summary)


if __name__ == "__main__":
    main()
