use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

mod mbedtls_connector;

use ureq;

fn get(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, ureq::Error> {
    let response = agent.get(url).call()?;
    let mut reader = response.into_reader();
    let mut bytes = vec![];
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn get_and_write(agent: &ureq::Agent, url: &str) {
    println!("🕷️ {}", url);
    match get(agent, url) {
        Ok(_) => println!("Good: ✔️ {}\n", url),
        Err(e) => println!("Bad: ⚠️ {} {}\n", url, e),
    }
}

fn main() -> Result<(), ureq::Error> {
    let agent = ureq::builder()
        .tls_connector(Arc::new(mbedtls_connector::MbedTlsConnector::new(
            mbedtls::ssl::config::AuthMode::None,
        )))
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build();

    get_and_write(&agent, "https://httpbin.org/get");

    Ok(())
}

/*
 * Local Variables:
 * compile-command: "cargo build --example mbedtls-req"
 * mode: rust
 * End:
 */
