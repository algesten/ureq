use std::error;
use std::fmt;
use std::io;
use std::time::Duration;
use std::{env, sync::Arc};

use rustls::{
    Certificate, ClientConfig, RootCertStore, ServerCertVerified, ServerCertVerifier, TLSError,
};
use ureq;
use webpki::DNSNameRef;

#[derive(Debug)]
struct StringError(String);

impl error::Error for StringError {}

impl fmt::Display for StringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for StringError {
    fn from(source: String) -> Self {
        Self(source)
    }
}

#[derive(Debug)]
struct Error {
    source: Box<dyn error::Error>,
}

impl error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.source)
    }
}

impl From<StringError> for Error {
    fn from(source: StringError) -> Self {
        Error {
            source: source.into(),
        }
    }
}

impl From<ureq::Error> for Error {
    fn from(source: ureq::Error) -> Self {
        Error {
            source: source.into(),
        }
    }
}

impl From<io::Error> for Error {
    fn from(source: io::Error) -> Self {
        Error {
            source: source.into(),
        }
    }
}

fn get(agent: &ureq::Agent, url: &str, print_headers: bool) -> Result<(), Error> {
    let response = agent.get(url).call()?;
    if print_headers {
        println!(
            "{} {} {}",
            response.http_version(),
            response.status(),
            response.status_text()
        );
        for h in response.headers_names() {
            println!("{}: {}", h, response.header(&h).unwrap_or_default());
        }
        println!();
    }
    let mut reader = response.into_reader();
    io::copy(&mut reader, &mut io::stdout())?;
    Ok(())
}

struct AcceptAll {}

impl ServerCertVerifier for AcceptAll {
    fn verify_server_cert(
        &self,
        _roots: &RootCertStore,
        _presented_certs: &[Certificate],
        _dns_name: DNSNameRef<'_>,
        _ocsp_response: &[u8],
    ) -> Result<ServerCertVerified, TLSError> {
        Ok(ServerCertVerified::assertion())
    }
}

fn main() {
    match main2() {
        Ok(()) => {}
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}

fn main2() -> Result<(), Error> {
    let mut args: Vec<String> = env::args().collect();
    if args.len() == 1 {
        println!(
            r##"Usage: {:#?} url [url ...]
            
Fetch url and copy it to stdout.
"##,
            env::current_exe()?
        );
        return Ok(());
    }
    args.remove(0);
    env_logger::init();
    let mut builder = ureq::builder()
        .timeout_connect(Duration::from_secs(30))
        .timeout(Duration::from_secs(300));
    let flags: Vec<&String> = args.iter().filter(|s| s.starts_with("-")).collect();
    let nonflags: Vec<&String> = args.iter().filter(|s| !s.starts_with("-")).collect();

    let mut print_headers: bool = false;
    for flag in flags {
        match flag.as_ref() {
            "-i" => print_headers = true,
            "-k" => {
                let mut client_config = ClientConfig::new();
                client_config
                    .dangerous()
                    .set_certificate_verifier(Arc::new(AcceptAll {}));
                builder = builder.tls_config(Arc::new(client_config));
            }
            f => Err(StringError(format!("unrecognized flag '{}'", f)))?,
        }
    }

    let agent = builder.build();

    for url in nonflags {
        get(&agent, &url, print_headers)?;
    }
    Ok(())
}
