use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args
        .iter()
        .any(|arg| !matches!(arg.as_str(), "--no-timeout" | "--no-spawn"))
    {
        eprintln!("Usage: cargo run --bin spawn -- [--no-timeout] [--no-spawn]");
        return ExitCode::FAILURE;
    }
    let timeout = (!args.iter().any(|arg| arg == "--no-timeout")).then(|| Duration::from_secs(5));
    let spawn = !args.iter().any(|arg| arg == "--no-spawn");
    let requests: usize = std::env::var("UREQ_REPRO_REQUESTS")
        .unwrap_or_else(|_| "2000".into())
        .parse()
        .expect("UREQ_REPRO_REQUESTS must be a positive integer");
    assert!(requests > 0);

    let stop = Arc::new(AtomicBool::new(false));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server_stop = stop.clone();
    let server = thread::spawn(move || -> io::Result<()> {
        while !server_stop.load(Ordering::Relaxed) {
            let mut socket = match listener.accept() {
                Ok((socket, _)) => socket,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_micros(100));
                    continue;
                }
                Err(e) => return Err(e),
            };
            socket.set_nonblocking(false)?;
            socket.set_read_timeout(Some(Duration::from_secs(1)))?;
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                // read_exact handles interruptions in the server itself.
                socket.read_exact(&mut byte)?;
                request.push(byte[0]);
            }
            thread::sleep(Duration::from_millis(2));
            match socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            {
                Ok(()) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
                    ) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    });

    // No signal handler or explicit signal delivery anywhere in this binary.
    let workers: Vec<_> = (0..if spawn { 4 } else { 0 })
        .map(|_| {
            let stop = stop.clone();
            thread::spawn(move || -> io::Result<usize> {
                let mut count = 0;
                while !stop.load(Ordering::Relaxed) {
                    if !Command::new("true").status()?.success() {
                        return Err(io::Error::other("true exited unsuccessfully"));
                    }
                    count += 1;
                }
                Ok(count)
            })
        })
        .collect();

    let agent = ureq::Agent::config_builder()
        .proxy(None)
        .timeout_global(timeout)
        .build()
        .new_agent();
    println!(
        "OS: {}; request timeout: {timeout:?}; subprocesses: {spawn}",
        std::env::consts::OS
    );
    let started = Instant::now();
    let mut failure = None;
    let mut completed = 0;
    for attempt in 1..=requests {
        match agent.get(format!("http://{address}/")).call() {
            Ok(mut response) => match response.body_mut().read_to_string() {
                Ok(body) => {
                    assert_eq!(body, "ok");
                    completed += 1;
                }
                Err(error) => {
                    failure = Some((attempt, error));
                    break;
                }
            },
            Err(error) => {
                failure = Some((attempt, error));
                break;
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    let commands: usize = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().unwrap())
        .sum();
    server.join().unwrap().unwrap();
    println!(
        "{completed} successful requests; {commands} child processes; {:?} elapsed",
        started.elapsed()
    );
    match failure {
        Some((attempt, error)) => {
            println!("FAILED on request {attempt}: {error}");
            if let ureq::Error::Io(error) = &error {
                println!(
                    "I/O kind: {:?}; OS error: {:?}",
                    error.kind(),
                    error.raw_os_error()
                );
            }
            ExitCode::FAILURE
        }
        None => {
            println!("No failure observed. This mode depends on the SIGCHLD race timing.");
            ExitCode::SUCCESS
        }
    }
}
