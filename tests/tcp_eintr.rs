// Keep this as a separate integration-test target: the signal handler is process-wide.
#![cfg(all(target_os = "linux", not(feature = "_test")))]

use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static SIGNALS_RECEIVED: AtomicUsize = AtomicUsize::new(0);

extern "C" fn handle_signal(_: libc::c_int) {
    SIGNALS_RECEIVED.fetch_add(1, Ordering::Relaxed);
}

#[test]
fn timed_http_read_survives_restarting_signal() {
    // SA_RESTART normally resumes interrupted I/O, but Linux does not restart
    // socket reads when SO_RCVTIMEO applies. TcpTransport must handle EINTR.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handle_signal as *const () as usize;
        action.sa_flags = libc::SA_RESTART;
        assert_eq!(libc::sigemptyset(&mut action.sa_mask), 0);
        assert_eq!(
            libc::sigaction(libc::SIGUSR1, &action, std::ptr::null_mut()),
            0,
            "{}",
            io::Error::last_os_error()
        );
    }

    // Deliver signals to the thread making the request, not an arbitrary thread.
    let reader_thread = unsafe { libc::pthread_self() };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "client never connected",
                        ));
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(e),
            }
        };
        socket.set_nonblocking(false)?;
        socket.set_read_timeout(Some(Duration::from_secs(5)))?;

        // Wait until the request arrives before interrupting the response read.
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte)?;
            request.push(byte[0]);
        }

        // Several signals tolerate brief scheduling delays. The response and
        // all signals arrive well before the client's two-second timeout.
        for _ in 0..4 {
            thread::sleep(Duration::from_millis(25));
            let error = unsafe { libc::pthread_kill(reader_thread, libc::SIGUSR1) };
            if error != 0 {
                return Err(io::Error::from_raw_os_error(error));
            }
        }
        thread::sleep(Duration::from_millis(100));
        match socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
        {
            Ok(()) => Ok(()),
            // A client that incorrectly propagates EINTR may close the socket.
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
                ) =>
            {
                Ok(())
            }
            Err(e) => Err(e),
        }
    });

    let agent = ureq::Agent::config_builder()
        .proxy(None)
        .timeout_global(Some(Duration::from_secs(2)))
        .build()
        .new_agent();
    let response = agent.get(format!("http://{address}/")).call();
    // Join before asserting on the response so no signals outlive this test.
    server.join().unwrap().unwrap();
    assert!(
        SIGNALS_RECEIVED.load(Ordering::Relaxed) > 0,
        "the requesting thread must receive a signal"
    );
    let mut response = response.expect("a signal must not abort the timed HTTP read");
    assert_eq!(response.status(), 200);
    assert_eq!(response.body_mut().read_to_string().unwrap(), "ok");
}
