//! A Happy Eyeballs RFC implementation
//!
//! Races interleaved IPv4 and IPv6 connections to provide the fastest connection
//! in cases where certain addresses or address families might be blocked, broken, or slow.
//! (See <https://datatracker.ietf.org/doc/html/rfc8305>)
//!
//! ureq strives for simplicity, and avoids spawning threads where it can,
//! but - like with SOCKS - there's no way around it here.
//! Some mini internal async executor
//! (discussed in <https://github.com/algesten/ureq/issues/535#issuecomment-1229433311>)
//! wouldn't help - `connect()` is a blocking syscall with no non-blocking alternative.
//! (Big async runtimes like Tokio "solve" this problem by keeping a pool of OS threads
//! around for just these sorts of blocking calls.)
//! We _could_ have some thread pool (a la rayon) to avoid spawning threads
//! on each connection attempt, but spawning a few threads is a cheap operation
//! compared to everything else going on here.
//! (DNS resolution, handshaking across the Internet...)
//!
//! Much of this implementation was inspired by attohttpc's:
//! <https://github.com/sbstp/attohttpc/blob/master/src/happy.rs>

use std::{
    io,
    iter::FusedIterator,
    net::{SocketAddr, TcpStream},
    sync::mpsc::{channel, RecvTimeoutError},
    thread,
    time::Instant,
};

use log::debug;

use crate::timeout::{io_err_timeout, time_until_deadline};

const TIMEOUT_MSG: &str = "timed out connecting";

pub fn connect(
    netloc: String,
    addrs: &[SocketAddr],
    deadline: Option<Instant>,
) -> io::Result<(TcpStream, SocketAddr)> {
    assert!(!addrs.is_empty());

    // No racing needed if there's a single address.
    if let [single] = addrs {
        return single_connection(&netloc, *single, deadline);
    }

    // Interleave IPV4 and IPV6 addresses
    let fours = addrs.iter().filter(|a| matches!(a, SocketAddr::V4(_)));
    let sixes = addrs.iter().filter(|a| matches!(a, SocketAddr::V6(_)));
    let sorted = interleave(fours, sixes);

    let (tx, rx) = channel();
    let mut first_error = None;

    // Race connections!
    // The RFC says:
    //
    // 1. Not to start connections "simultaneously", but since `connect()`
    //    syscalls don't return until they've connected or timed out,
    //    we don't have a way to start an attempt without blocking until it finishes.
    //    (And if we did that, we wouldn't be racing!)
    //
    // 2. Once we have a successful connection, all other attempts should be cancelled.
    //    Doing so would require a lot of nasty (and platform-specific) signal handling,
    //    as it's the only way to interrupt `connect()`.
    for s in sorted {
        // Instead, make a best effort to not start new connections if we've got one already.
        if let Ok(resp) = rx.try_recv() {
            match resp {
                Ok(c) => return Ok(c),
                Err(e) => {
                    let _ = first_error.get_or_insert(e);
                }
            }
        }

        let tx2 = tx.clone();
        let nl2 = netloc.clone();
        let s2 = *s;
        thread::spawn(move || {
            // If the receiver was dropped, someone else already won the race.
            let _ = tx2.send(single_connection(&nl2, s2, deadline));
        });
    }
    drop(tx);

    const UNREACHABLE_MSG: &str =
        "Unreachable: All Happy Eyeballs connections failed, but no error";

    if let Some(d) = deadline {
        // Wait for a successful connection, or for us to run out of time
        loop {
            let timeout = time_until_deadline(d, TIMEOUT_MSG)?;
            match rx.recv_timeout(timeout) {
                Ok(Ok(c)) => return Ok(c),
                Ok(Err(e)) => {
                    let _ = first_error.get_or_insert(e);
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(io_err_timeout(TIMEOUT_MSG.to_string()))
                }
                // If all the connecting threads hung up and none succeeded,
                // return the first error.
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(first_error.expect(UNREACHABLE_MSG))
                }
            };
        }
    } else {
        // If there's no deadline, just wait around.
        let connections = rx.iter();
        for c in connections {
            match c {
                Ok(c) => return Ok(c),
                Err(e) => {
                    let _ = first_error.get_or_insert(e);
                }
            }
        }
        // If we got here, everyone failed. Return the first error.
        Err(first_error.expect(UNREACHABLE_MSG))
    }
}

fn single_connection(
    netloc: &str,
    addr: SocketAddr,
    deadline: Option<Instant>,
) -> io::Result<(TcpStream, SocketAddr)> {
    debug!("connecting to {} at {}", netloc, addr);
    if let Some(d) = deadline {
        let timeout = time_until_deadline(d, TIMEOUT_MSG)?;
        Ok((TcpStream::connect_timeout(&addr, timeout)?, addr))
    } else {
        Ok((TcpStream::connect(addr)?, addr))
    }
}

fn interleave<T, A, B>(mut left: A, mut right: B) -> impl Iterator<Item = T>
where
    A: FusedIterator<Item = T>,
    B: FusedIterator<Item = T>,
{
    let mut last_right = None;

    std::iter::from_fn(move || {
        if let Some(r) = last_right.take() {
            return Some(r);
        }

        match (left.next(), right.next()) {
            (Some(l), Some(r)) => {
                last_right = Some(r);
                Some(l)
            }
            (Some(l), None) => Some(l),
            (None, Some(r)) => Some(r),
            (None, None) => None,
        }
    })
}
