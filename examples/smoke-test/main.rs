use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::{env, error, fmt, result, result::Result};

use log::{error, info};
use ureq::{self, Response};

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

fn get(agent: &ureq::Agent, url: &str) -> std::result::Result<(), Oops> {
    let r = get_response(agent, url)?;
    let mut reader = r.into_reader();
    let mut bytes = vec![];
    reader.read_to_end(&mut bytes)?;
    std::io::stdout().write_all(&bytes)?;
    Ok(())
}

fn get_and_write(agent: &ureq::Agent, url: &str) {
    info!("🕷️ {}", url);
    match get(agent, url) {
        Ok(_) => info!("✔️ {}", url),
        Err(e) => error!("⚠️ {} {}", url, e),
    }
}

fn get_response(agent: &ureq::Agent, url: &str) -> result::Result<Response, ureq::Error> {
    let fetch = || agent.get(url).call();
    let mut result = fetch();
    for _ in 1..4 {
        let err = match result {
            Ok(r) => return Ok(r),
            Err(e) => e,
        };
        match err {
            ureq::Error::Status(429, r) => {
                let retry: Option<u64> = r.header("retry-after").and_then(|h| h.parse().ok());
                eprintln!("429 for {}, retrying in {}", r.status(), r.get_url());
                thread::sleep(Duration::from_secs(retry.unwrap_or(5)));
                eprintln!("error status {} body {}", r.status(), r.into_string()?);
            }
            ureq::Error::Status(_, r) => return Err(r.into()),
            ureq::Error::Transport(e) => return Err(e.into()),
        };
        result = fetch();
    }
    println!("Failed after 5 tries: {:?}", &result);
    result
}

fn get_many(urls: Vec<String>, simultaneous_fetches: usize) -> Result<(), Oops> {
    let agent = ureq::builder()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build();
    let mutex = Arc::new(Mutex::new(urls));
    let mut join_handles: Vec<JoinHandle<()>> = vec![];
    for _ in 0..simultaneous_fetches {
        let mutex = mutex.clone();
        let agent = agent.clone();
        join_handles.push(thread::spawn(move || loop {
            let mut guard = mutex.lock().unwrap();
            let u = match guard.pop() {
                Some(u) => u,
                None => return,
            };

            drop(guard);

            get_and_write(&agent, &u);
        }));
    }
    for h in join_handles {
        h.join().map_err(|e| Oops(format!("{:?}", e)))?;
    }
    Ok(())
}

fn main() -> Result<(), Oops> {
    let args = env::args();
    if args.len() == 1 {
        println!(
            r##"Usage: {:#?} top-1m.csv
        
Where top-1m.csv is a simple, unquoted CSV containing two fields, a rank and
a domain name. For instance you can get such a list from https://tranco-list.eu/.

For each domain, this program will attempt to GET four URLs: The domain name
name with HTTP and HTTPS, and with and without a www prefix. It will fetch
using 50 threads concurrently.
"##,
            env::current_exe()?
        );
        return Ok(());
    }
    env_logger::init();
    let file = std::fs::File::open(args.skip(1).next().unwrap())?;
    let bufreader = BufReader::new(file);
    let mut urls = vec![];
    for line in bufreader.lines() {
        let domain = line?.rsplit(",").next().unwrap().to_string();
        urls.push(format!("http://{}/", domain));
        urls.push(format!("https://{}/", domain));
        urls.push(format!("http://www.{}/", domain));
        urls.push(format!("https://www.{}/", domain));
    }
    get_many(urls, 50)?;
    Ok(())
}
