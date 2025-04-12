use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use ureq::config::Config;
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{Buffers, ConnectionDetails, Connector, LazyBuffers};
use ureq::unversioned::transport::{NextTimeout, RustlsConnector, Transport};
use ureq::{Agent, Error};

pub fn main() {
    // To see some inner workings of ureq, this example can be interesting to run with:
    // RUST_LOG=trace cargo run --example mpsc-transport
    env_logger::init();

    // Our very own connector.
    let connector = MpscConnector::default();

    // Spawn a fake server in a thread. We take the server_side TxRx to be able to
    // communicate with the client.
    let server_side = connector.server_side.clone();
    thread::spawn(|| run_server(server_side));

    let chain = connector
        // For educational purposes, we wrap add the RustlsConnector to the chain.
        // This would mean an URL that starts with https:// will be wrapped in TLS
        // letting our MpscConnector handle the "underlying" transport
        // with TLS on top.
        //
        // This example does not use an https URL since that would require us
        // to terminate the TLS in the server side, which is more involved than
        // this example is trying to show.
        .chain(RustlsConnector::default());

    // Use default config and resolver.
    let config = Config::default();
    let resolver = DefaultResolver::default();

    // Construct an agent from the parts
    let agent = Agent::with_parts(config, chain, resolver);

    // This request will use the MpscTransport to communicate with the fake server.
    // If you change this to "https", the RustlsConnector will be used, but the
    // fake server is not made to handle TLS.
    let mut res = agent.get("http://example.com").call().unwrap();

    println!(
        "CLIENT got response: {:?}",
        res.body_mut().read_to_string().unwrap()
    );
}

#[derive(Debug, Default)]
pub struct MpscConnector {
    server_side: Arc<Mutex<Option<TxRx>>>,
}

impl<In: Transport> Connector<In> for MpscConnector {
    type Out = MpscTransport;

    fn connect(
        &self,
        details: &ConnectionDetails,
        _: Option<In>,
    ) -> Result<Option<Self::Out>, Error> {
        println!(
            "Making an mpsc connection to {:?} (with addrs: {:?})",
            details.uri,
            // The default resolver does resolve this to some IP addresses.
            &details.addrs[..]
        );

        let (txrx1, txrx2) = TxRx::pair();

        let transport = MpscTransport::new(txrx1, 1024, 1024);

        // This is how we pass the server_side TxRx to the server thread.
        // A more realistic example would not do this.
        {
            let mut server_side = self.server_side.lock().unwrap();
            *server_side = Some(txrx2);
        }

        Ok(Some(transport))
    }
}

/// A pair of channels for transmitting and receiving data.
///
/// These will be connected to another such pair.
#[derive(Debug)]
pub struct TxRx {
    tx: mpsc::SyncSender<Vec<u8>>,
    // The Mutex here us unfortunate for this example since we are not using rx in
    // a "Sync way", but we also don't want to make an unsafe impl Sync to risk
    // having the repo flagged as unsafe by overzealous compliance tools.
    rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    alive: bool,
}

impl TxRx {
    pub fn pair() -> (TxRx, TxRx) {
        let (tx1, rx1) = mpsc::sync_channel(10);
        let (tx2, rx2) = mpsc::sync_channel(10);
        (TxRx::new(tx1, rx2), TxRx::new(tx2, rx1))
    }

    fn new(tx: mpsc::SyncSender<Vec<u8>>, rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            tx,
            rx: Mutex::new(rx),
            alive: true,
        }
    }

    pub fn send(&mut self, data: Vec<u8>) {
        if let Err(e) = self.tx.send(data) {
            println!("Failed to send data: {}", e);
            self.alive = false;
        }
    }

    pub fn recv(&mut self) -> Option<Vec<u8>> {
        let rx = self.rx.lock().unwrap();
        match rx.recv() {
            Ok(data) => Some(data),
            Err(e) => {
                println!("Failed to receive data: {}", e);
                self.alive = false;
                None
            }
        }
    }

    pub fn is_alive(&self) -> bool {
        self.alive
    }
}

/// A transport over TxRx channel.
#[derive(Debug)]
pub struct MpscTransport {
    buffers: LazyBuffers,
    txrx: TxRx,
}

impl MpscTransport {
    pub fn new(txrx: TxRx, input_buffer_size: usize, output_buffer_size: usize) -> Self {
        Self {
            buffers: LazyBuffers::new(input_buffer_size, output_buffer_size),
            txrx,
        }
    }
}

impl Transport for MpscTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, _timeout: NextTimeout) -> Result<(), Error> {
        // The data to send. Must use the amount to know how much of the buffer
        // is relevant.
        let to_send = &self.buffers.output()[..amount];

        // Blocking send until the other side receives it.
        self.txrx.send(to_send.to_vec());

        Ok(())
    }

    fn await_input(&mut self, _timeout: NextTimeout) -> Result<bool, Error> {
        let Some(data) = self.txrx.recv() else {
            return Ok(false);
        };

        // Append the data to the input buffer.
        let input = self.buffers.input_append_buf();
        let len = data.len();
        input[..len].copy_from_slice(data.as_slice());

        // Report how many bytes appended to the input buffer.
        self.buffers.input_appended(len);

        // Return true if we made progress, i.e. if we managed to fill the input buffer with any bytes.
        Ok(len > 0)
    }

    fn is_open(&mut self) -> bool {
        self.txrx.is_alive()
    }
}

// A fake HTTP server that responds with "Hello, world!"
fn run_server(server_side: Arc<Mutex<Option<TxRx>>>) {
    // Wait until the server side is present.
    let txrx = loop {
        // Scope to not hold lock while sleeping
        let txrx = {
            let mut lock = server_side.lock().unwrap();
            lock.take()
        };

        if let Some(txrx) = txrx {
            break txrx;
        }

        thread::sleep(Duration::from_millis(100));
    };

    // No contention on this lock. See above why we even need it.e
    let rx = txrx.rx.lock().unwrap();

    let mut incoming = String::new();

    // We are not guaranteed to receive the entire request in one go.
    // Loop until we know we have it.
    loop {
        let data = rx.recv().unwrap();

        let s = String::from_utf8_lossy(&data);
        incoming.push_str(&s);

        if incoming.contains("\r\n\r\n") {
            break;
        }
    }

    println!("SERVER received request: {:?}", incoming);

    // A random response.
    let response = "HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\nHello, world!";

    println!("SERVER sending response: {:?}", response);

    txrx.tx.send(response.as_bytes().to_vec()).unwrap();
}
