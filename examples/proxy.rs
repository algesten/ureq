use std::error::Error;

use ureq::{config::Config, Agent, Proxy};

// Use this example with something like mitmproxy
// $ mitmproxy --listen-port 8080
// $ mitmproxy --listen-port 8081
//
// Set HTTP_PROXY to localhost:8080
// Set HTTPS_PROXY to locaqlhost:8081

fn main() -> Result<(), Box<dyn Error>> {
    let (http_proxy, https_proxy) = Proxy::try_from_env();

    let config = Config::builder()
        .proxy_http(http_proxy)
        .proxy_https(https_proxy)
        .build();
    let agent = Agent::new_with_config(config);

    let res = agent.get("http://www.example.com").call()?;
    println!("HTTP Target: {}", res.status());

    let res = agent.get("https://www.example.com").call()?;
    println!("HTTPS Target: {}", res.status());

    Ok(())
}
