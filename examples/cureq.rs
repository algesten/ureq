use std::io::{stdout, BufRead, BufReader, Write};
use std::process;
use std::time::Duration;

use auto_args::AutoArgs;
use ureq::tls::TlsConfig;
use ureq::{Agent, AgentConfig};

#[derive(Debug, AutoArgs)]
struct Opt {
    /// Print headers
    include: Option<bool>,

    /// Timeout for entire request (seconds)
    max_time: Option<u32>,

    /// Disable certificate verification
    insecure: Option<bool>,

    /// URL to request
    url: String,
}

fn main() {
    env_logger::init();
    let opt = Opt::from_args();
    if let Err(e) = run(&opt) {
        eprintln!("{} - {}", e, opt.url);
        process::exit(1);
    }
}

fn run(opt: &Opt) -> Result<(), ureq::Error> {
    let agent: Agent = AgentConfig {
        timeout_global: opt.max_time.map(|t| Duration::from_secs(t.into())),
        tls_config: TlsConfig {
            disable_verification: opt.insecure.unwrap_or(false),
            ..Default::default()
        },
        ..Default::default()
    }
    .into();

    let mut res = agent.get(&opt.url).call()?;

    if opt.include.unwrap_or(false) {
        eprintln!("{:#?}", res.headers());
    }

    const MAX_BODY_SIZE: u64 = 5 * 1024 * 1024;

    let reader = BufReader::new(res.body_mut().as_reader(MAX_BODY_SIZE));
    let mut lines = reader.lines();

    let mut has_lf = false;

    while let Some(r) = lines.next() {
        let line = match r {
            Ok(v) => v,
            Err(e) => return Err(e.into()),
        };
        let bytes = line.as_bytes();
        has_lf = bytes.is_empty() || bytes[bytes.len() - 1] == b'\n';
        stdout().write_all(line.as_bytes())?;
    }

    if !has_lf {
        println!();
    }

    Ok(())
}
