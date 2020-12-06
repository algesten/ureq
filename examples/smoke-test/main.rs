use std::io::{self, BufRead, BufReader, Read};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::{env, error, fmt, result};

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
        Ok(_) => info!("✔️ {}", url),
        Err(e) => error!("⚠️ {} {}", url, e),
    }
}

fn get_many(urls: Vec<String>, simultaneous_fetches: usize) -> Result<()> {
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

fn main() -> Result<()> {
    let args = env::args();
    env_logger::init();
    for a in args {
        get_and_write(&ureq::agent(), &a);
    }
    Ok(())
}
