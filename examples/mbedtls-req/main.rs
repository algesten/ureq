use std::io::{self, Read};
use std::sync::Arc;
use std::time::Duration;
use std::{env, error, fmt, result};

pub mod mbedtls_connector;

use log::{error, info};
use ureq;

#[derive(Debug)]
struct Oops(String);

impl From<io::Error> for Oops {
    fn from(e: io::Error) -> Oops {
        Oops(e.to_string())
    }
}

impl From<ureq::Error> for Oops {
    fn from(e: ureq::Error) -> Oops {
        Oops(e.to_string())
    }
}

impl error::Error for Oops {}

impl fmt::Display for Oops {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

type Result<T> = result::Result<T, Oops>;

fn get(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>> {
    let response = agent.get(url).call()?;
    let mut reader = response.into_reader();
    let mut bytes = vec![];
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn get_and_write(agent: &ureq::Agent, url: &str) {
    info!("🕷️ {}", url);
    match get(agent, url) {
        Ok(_) => info!("Good: ✔️ {}\n", url),
        Err(e) => error!("Bad: ⚠️ {} {}\n", url, e),
    }
}

fn main() -> Result<()> {
    let _args = env::args();
    env_logger::init();

    let agent = ureq::builder()
        .tls_connector(Arc::new(mbedtls_connector::MbedTlsConnector::new(
            mbedtls::ssl::config::AuthMode::None,
        )))
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build();

    get_and_write(&agent, "https://example.com/");

    Ok(())
}

/*
 * Local Variables:
 * compile-command: "cargo build --example mbedtls-req --features=\"mbedtls\""
 * mode: rust
 * End:
 */
