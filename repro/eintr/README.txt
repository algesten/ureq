Linux EINTR reproduction for ureq PR #1205

This standalone crate uses the checkout's ureq without _test or TLS features.
The default binary serves HTTP locally and sends SIGUSR1 to the client thread
while it waits for the response. Its harmless signal handler uses SA_RESTART.

From this directory on Linux:
  cargo run --release
  cargo run --release -- --no-timeout

With current ureq, the first command fails with Interrupted system call
(os error 4), while the second succeeds.

The spawn binary creates the original subprocess race without installing
signal handlers or explicitly sending signals:
  cargo run --release --bin spawn
  cargo run --release --bin spawn -- --no-spawn
  cargo run --release --bin spawn -- --no-timeout

It runs true on four threads alongside up to 2000 loopback HTTP requests.
UREQ_REPRO_REQUESTS can override the request count. A failure in this mode
depends on SIGCHLD race timing; absence of a failure does not rule out the bug.

The EINTR reproduction workflow tests immutable source revisions:
  Baseline: 8fcd72a7881354400c432157e1a60222d61efc5c
  PR #1205: 4ad65d551e5ff87a8c340160cd2c1805e4dd0425

It verifies the baseline's Interrupted error, the successful controls, and
successful signal and subprocess runs with the proposed fix. A baseline
subprocess run that does not win the race is reported explicitly.

To compare local checkouts:
  python3 check.py /path/to/baseline /path/to/fixed

Reference: https://github.com/algesten/ureq/pull/1205
