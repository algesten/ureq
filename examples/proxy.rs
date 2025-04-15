use std::error::Error;

use ureq::tls::TlsConfig;
use ureq::{config::Config, Agent, Proxy};

// Use this example with something like mitmproxy
// $ mitmproxy --listen-port 8080

fn main() -> Result<(), Box<dyn Error>> {
    let proxy = Proxy::new("http://localhost:8080")?;

    let config = Config::builder()
        .tls_config(
            TlsConfig::builder()
                // The mitmproxy uses a certificate authority we
                // don't know. Do not disable verification in
                // production use.
                .disable_verification(true)
                .build(),
        )
        .proxy(Some(proxy))
        .build();
    let agent = Agent::new_with_config(config);

    let _ = agent.get("https://example.com").call()?;

    Ok(())
}
