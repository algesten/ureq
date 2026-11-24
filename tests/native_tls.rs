// The _test transport reports itself as already encrypted and bypasses TLS.
#![cfg(all(feature = "native-tls-no-default", not(feature = "_test")))]

use std::io::Read;
use std::net::{TcpListener, TcpStream};

use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

#[test]
fn native_tls_platform_verifier_starts_handshake() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut header = [0; 3];
        stream.read_exact(&mut header).unwrap();
        // A TLS handshake record has content type 22 and record major version 3.
        assert_eq!(&header[..2], &[22, 3], "expected a TLS handshake record");
    });

    let agent = Agent::config_builder()
        .proxy(None)
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
    // Wake accept if the connector failed before opening its socket.
    let _ = TcpStream::connect(address);
    let handshake = server.join();
    assert!(request.is_ok(), "the native TLS connector must not panic");
    handshake.expect("the client must send a TLS handshake");
    // The peer closes without completing TLS: this must be an ordinary error.
    assert!(request.unwrap().is_err());
}

// The default agent is configured for TlsProvider::Rustls. When rustls is not
// compiled in, a server can still send the agent to https with a redirect.
#[cfg(not(feature = "rustls-no-provider"))]
#[test]
fn https_redirect_without_provider_feature_is_an_error() {
    use std::io::Write;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        let _ = stream.read(&mut request).unwrap();
        write!(
            stream,
            "HTTP/1.1 302 Found\r\nLocation: https://{address}/\r\nContent-Length: 0\r\n\r\n"
        )
        .unwrap();
        // The redirect target only accepts the connection.
        let _ = listener.accept();
    });

    let agent = Agent::config_builder().proxy(None).build().new_agent();
    let request = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        agent.get(format!("http://{address}/")).call()
    }));
    // Wake the second accept if the client never connected to the redirect target.
    let _ = TcpStream::connect(address);
    server.join().unwrap();
    assert!(request.is_ok(), "a redirect to https must not panic");
    assert!(matches!(request.unwrap(), Err(ureq::Error::Tls(_))));
}
