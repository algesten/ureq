use chrono::Local;
use rayon::prelude::*;
use rayon_core;

use std::io::{self, BufRead, BufReader, Read};
use std::iter::Iterator;
use std::time::Duration;
use std::{env, error, fmt, result};

use ureq;

#[derive(Debug)]
struct Oops(String);

impl From<io::Error> for Oops {
    fn from(e: io::Error) -> Oops {
        Oops(e.to_string())
    }
}

impl From<&ureq::Error> for Oops {
    fn from(e: &ureq::Error) -> Oops {
        Oops(e.to_string())
    }
}

impl From<rayon_core::ThreadPoolBuildError> for Oops {
    fn from(e: rayon_core::ThreadPoolBuildError) -> Oops {
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

fn get(agent: &ureq::Agent, url: &String) -> Result<Vec<u8>> {
    let response = agent
        .get(url)
        .timeout_connect(5_000)
        .timeout(Duration::from_secs(20))
        .call();
    if let Some(err) = response.synthetic_error() {
        return Err(err.into());
    }
    let mut reader = response.into_reader();
    let mut bytes = vec![];
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn get_and_write(agent: &ureq::Agent, url: &String) -> Result<()> {
    println!("🕷️ {} {}", Local::now(), url);
    match get(agent, url) {
        Ok(_) => println!("✔️ {} {}", Local::now(), url),
        Err(e) => println!("⚠️ {} {} {}", Local::now(), url, e),
    }
    Ok(())
}

fn get_many(urls: Vec<String>, simultaneous_fetches: usize) -> Result<()> {
    let agent = ureq::Agent::default().build();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(simultaneous_fetches)
        .build()?;
    pool.scope(|_| {
        urls.par_iter().map(|u| get_and_write(&agent, u)).count();
    });
    Ok(())
}

fn main() -> Result<()> {
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
