use std::io::{self, Read};
use std::time;

/// Retry reads interrupted by a signal (EINTR) when a timeout applies.
///
/// Linux does not restart socket reads with SO_RCVTIMEO, even with SA_RESTART.
/// Retries use the remaining timeout; exhausted budgets return `TimedOut`.
/// Without a timeout, preserve `Interrupted` for intentional signal interruptions.
/// See <https://github.com/algesten/ureq/pull/1205> for the signal details.
pub(super) fn read_retrying_interrupts<R: Read>(
    reader: &mut R,
    buf: &mut [u8],
    timeout: Option<time::Duration>,
    mut elapsed: impl FnMut() -> time::Duration,
    mut set_timeout: impl FnMut(&mut R, time::Duration) -> io::Result<()>,
) -> io::Result<usize> {
    loop {
        match reader.read(buf) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                let Some(timeout) = timeout else {
                    return Err(e);
                };
                let left = timeout.saturating_sub(elapsed());
                if left.is_zero() {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                set_timeout(reader, left)?;
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // Script read outcomes and elapsed times, returning what the caller saw and every
    // timeout it was given.
    fn scripted_read(
        outcomes: Vec<io::Result<usize>>,
        timeout: Option<time::Duration>,
        elapsed: Vec<time::Duration>,
    ) -> (io::Result<usize>, Vec<time::Duration>) {
        struct Script(std::vec::IntoIter<io::Result<usize>>);
        impl Read for Script {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                self.0.next().unwrap()
            }
        }
        let mut elapsed = elapsed.into_iter();
        let mut timeouts = vec![];
        let result = read_retrying_interrupts(
            &mut Script(outcomes.into_iter()),
            &mut [0; 8],
            timeout,
            || elapsed.next().unwrap(),
            |_, left| {
                timeouts.push(left);
                Ok(())
            },
        );
        (result, timeouts)
    }

    fn interrupted() -> io::Result<usize> {
        Err(io::ErrorKind::Interrupted.into())
    }

    fn secs(s: u64) -> time::Duration {
        time::Duration::from_secs(s)
    }

    #[test]
    fn interrupted_read_is_retried_with_remaining_timeout() {
        let (result, timeouts) = scripted_read(
            vec![interrupted(), interrupted(), Ok(3)],
            Some(secs(10)),
            vec![secs(3), secs(7)],
        );
        assert_eq!(result.unwrap(), 3);
        assert_eq!(timeouts, vec![secs(7), secs(3)]);
    }

    #[test]
    fn interrupted_read_past_timeout_times_out() {
        let (result, timeouts) =
            scripted_read(vec![interrupted(), Ok(3)], Some(secs(1)), vec![secs(1)]);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(timeouts.is_empty());
    }

    #[test]
    fn only_interrupted_reads_are_retried() {
        let (result, timeouts) = scripted_read(
            vec![
                interrupted(),
                Err(io::ErrorKind::ConnectionReset.into()),
                Ok(3),
            ],
            Some(secs(10)),
            vec![secs(1)],
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionReset);
        assert_eq!(timeouts, vec![secs(9)]);
    }

    // Without a timeout, only a handler installed without SA_RESTART causes EINTR. It asked
    // for the interruption, so the read must not be retried.
    #[test]
    fn interrupted_read_without_timeout_is_not_retried() {
        let (result, timeouts) = scripted_read(vec![interrupted(), Ok(3)], None, vec![]);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert!(timeouts.is_empty());
    }
}
