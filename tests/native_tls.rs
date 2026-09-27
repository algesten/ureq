// The _test transport reports itself as already encrypted and bypasses TLS.
#![cfg(all(feature = "native-tls-no-default", not(feature = "_test")))]

use std::io::Read;
use std::net::TcpListener;
use std::time::{Duration, Instant};

use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

#[test]
fn native_tls_platform_verifier_starts_handshake() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "HTTPS client never connected");
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("unable to accept HTTPS client: {error}"),
            }
        };
        // Windows may inherit the listener's nonblocking mode.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut header = [0; 3];
        stream.read_exact(&mut header).unwrap();
        // A TLS handshake record has content type 22 and record major version 3.
        assert_eq!(&header[..2], &[22, 3], "expected a TLS handshake record");
    });

    let agent = Agent::config_builder()
        .proxy(None)
        .timeout_global(Some(Duration::from_secs(5)))
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .new_agent();
    let request = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        agent.get(format!("https://{address}/")).call()
    }));
    let handshake = server.join();
    assert!(request.is_ok(), "the native TLS connector must not panic");
    handshake.expect("the client must send a TLS handshake");
    // The peer closes without completing TLS: this must be an ordinary error.
    assert!(request.unwrap().is_err());
}
