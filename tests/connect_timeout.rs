// Use the real transport and clock, not the in-memory _test connector.
#![cfg(not(feature = "_test"))]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn fragmented_connect_response_cannot_renew_total_budget() {
    for global in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                headers.push(byte[0]);
                assert!(headers.len() < 8192);
            }
            assert!(headers.starts_with(b"CONNECT example.test:80 HTTP/1.1\r\n"));
            for fragment in [
                b"HTTP/1.1 200".as_slice(),
                b" Connection",
                b" established\r\n",
                b"X-Test: bounded",
                b"\r\n\r\n",
            ] {
                thread::sleep(Duration::from_millis(80));
                if stream.write_all(fragment).is_err() {
                    break;
                }
            }
        });
        let limit = Duration::from_millis(150);
        let agent = ureq::Agent::config_builder()
            .proxy(Some(
                ureq::Proxy::new(&format!("http://{address}")).unwrap(),
            ))
            .timeout_global(global.then_some(limit))
            .timeout_connect((!global).then_some(limit))
            .build()
            .new_agent();
        let start = Instant::now();
        let result = agent.get("http://example.test/").call();
        let elapsed = start.elapsed();
        server.join().unwrap();
        let expected = if global {
            ureq::Timeout::Global
        } else {
            ureq::Timeout::Connect
        };
        assert!(matches!(result, Err(ureq::Error::Timeout(actual)) if actual == expected));
        // Leave scheduling tolerance while staying below the fragmented response's 400ms.
        assert!(
            elapsed < Duration::from_millis(350),
            "{expected}: {elapsed:?}"
        );
    }
}
