use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static SIGNALS_RECEIVED: AtomicUsize = AtomicUsize::new(0);

extern "C" fn handle_signal(_: libc::c_int) {
    SIGNALS_RECEIVED.fetch_add(1, Ordering::Relaxed);
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let timeout = match args.as_slice() {
        [] => Some(Duration::from_secs(2)),
        [arg] if arg == "--no-timeout" => None,
        _ => {
            eprintln!("Usage: cargo run -- [--no-timeout]");
            return ExitCode::FAILURE;
        }
    };

    // A harmless handler explicitly asks the kernel to restart interrupted I/O.
    // Linux makes an exception for sockets with SO_RCVTIMEO configured.
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

    // Target the thread making the request, rather than an arbitrary thread.
    let reader_thread = unsafe { libc::pthread_self() } as usize;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || -> io::Result<()> {
        let (mut socket, _) = listener.accept()?;
        socket.set_read_timeout(Some(Duration::from_secs(5)))?;

        // Wait until the request has arrived before interrupting the client.
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte)?;
            request.push(byte[0]);
        }

        // Several signals make this robust to brief scheduling delays. They all
        // arrive well before the two-second timeout or the server's response.
        for _ in 0..4 {
            thread::sleep(Duration::from_millis(25));
            let error =
                unsafe { libc::pthread_kill(reader_thread as libc::pthread_t, libc::SIGUSR1) };
            if error != 0 {
                return Err(io::Error::from_raw_os_error(error));
            }
        }
        thread::sleep(Duration::from_millis(100));
        // Current ureq may already have closed the socket after EINTR.
        match socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
        {
            Ok(()) => Ok(()),
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
        .timeout_global(timeout)
        .build()
        .new_agent();
    println!("OS: {}", std::env::consts::OS);
    println!("Request timeout: {timeout:?}; signal handler: SA_RESTART");
    let started = Instant::now();
    let result = agent.get(format!("http://{address}/")).call();
    let elapsed = started.elapsed();
    server.join().unwrap().unwrap();
    println!(
        "Signals delivered to the request thread: {}",
        SIGNALS_RECEIVED.load(Ordering::Relaxed)
    );

    match result {
        Ok(mut response) => {
            let body = response.body_mut().read_to_string().unwrap();
            assert_eq!(body, "ok");
            println!(
                "SUCCESS after {elapsed:?}: HTTP {} / {body}",
                response.status()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("FAILED after {elapsed:?}: {error}");
            if let ureq::Error::Io(error) = &error {
                println!(
                    "I/O kind: {:?}; OS error: {:?}",
                    error.kind(),
                    error.raw_os_error()
                );
            }
            ExitCode::FAILURE
        }
    }
}
