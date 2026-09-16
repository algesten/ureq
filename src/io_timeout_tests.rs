use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::Config;
use crate::transport::{Buffers, ConnectionDetails, Connector, LazyBuffers};
use crate::transport::{NextTimeout, Transport};
use crate::unversioned::resolver::DefaultResolver;
use crate::{Agent, Error, Timeout};

#[derive(Debug, Clone)]
struct Script(Arc<Mutex<State>>);

#[derive(Debug, Default)]
struct State {
    connections: usize,
    reads: Vec<NextTimeout>,
    writes: Vec<NextTimeout>,
    responses: VecDeque<&'static [u8]>,
    fail_read: bool,
    fail_write: bool,
}

impl Connector for Script {
    type Out = ScriptTransport;

    fn connect(&self, _: &ConnectionDetails, _: Option<()>) -> Result<Option<Self::Out>, Error> {
        self.0.lock().unwrap().connections += 1;
        Ok(Some(ScriptTransport {
            script: self.clone(),
            buffers: LazyBuffers::new(1024, 1024),
        }))
    }
}

#[derive(Debug)]
struct ScriptTransport {
    script: Script,
    buffers: LazyBuffers,
}

impl Transport for ScriptTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, _: usize, timeout: NextTimeout) -> Result<(), Error> {
        let timeout = timeout.for_write().check()?;
        let mut state = self.script.0.lock().unwrap();
        state.writes.push(timeout);
        if state.fail_write {
            return Err(Error::Timeout(timeout.reason));
        }
        Ok(())
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        let timeout = timeout.for_read().check()?;
        let mut state = self.script.0.lock().unwrap();
        state.reads.push(timeout);
        if state.fail_read {
            return Err(Error::Timeout(timeout.reason));
        }
        let response = state.responses.pop_front().expect("unexpected read");
        self.buffers.input_append_buf()[..response.len()].copy_from_slice(response);
        self.buffers.input_appended(response.len());
        Ok(!response.is_empty())
    }

    fn is_open(&mut self) -> bool {
        true
    }
}

fn setup(config: Config) -> (Agent, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    let agent = Agent::with_parts(config, Script(state.clone()), DefaultResolver::default());
    (agent, state)
}

fn config() -> Config {
    Agent::config_builder()
        .proxy(None)
        .timeout_per_read(Some(Duration::from_secs(5)))
        .timeout_per_write(Some(Duration::from_secs(7)))
        .build()
}

const HEADERS: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n";

#[test]
fn headers_and_bodies_use_directional_limits_and_pool_overrides() {
    let (agent, state) = setup(config());
    for limits in [None, Some((2, 3)), Some((0, 0)), None] {
        state
            .lock()
            .unwrap()
            .responses
            .extend([HEADERS, b"o", b"k"]);
        let request = agent.post("http://127.0.0.1/");
        let request = if let Some((read, write)) = limits {
            request
                .config()
                .timeout_per_read((read != 0).then(|| Duration::from_secs(read)))
                .timeout_per_write((write != 0).then(|| Duration::from_secs(write)))
                .build()
        } else {
            request
        };
        let mut response = request.send("body").unwrap();
        assert_eq!(response.body_mut().read_to_string().unwrap(), "ok");
        let mut state = state.lock().unwrap();
        assert_eq!(
            state.connections, 1,
            "overrides must reuse the same transport"
        );
        assert_eq!(state.reads.len(), 3, "headers and two body chunks");
        assert!(state.writes.len() >= 2, "headers and body");
        let (read, write) = limits.unwrap_or((5, 7));
        for (operations, seconds, reason) in [
            (&state.reads, read, Timeout::PerRead),
            (&state.writes, write, Timeout::PerWrite),
        ] {
            for timeout in operations {
                if seconds == 0 {
                    assert!(timeout.after.is_not_happening());
                } else {
                    assert_eq!(*timeout.after, Duration::from_secs(seconds));
                    assert_eq!(timeout.reason, reason);
                }
            }
        }
        state.reads.clear();
        state.writes.clear();
    }
}

#[test]
fn read_timeout_while_awaiting_100_is_not_swallowed() {
    let config = Agent::config_builder()
        .proxy(None)
        .timeout_per_read(Some(Duration::from_millis(10)))
        .build();
    let (agent, state) = setup(config);
    state.lock().unwrap().fail_read = true;
    let error = agent
        .post("http://127.0.0.1/")
        .header("expect", "100-continue")
        .send("body")
        .unwrap_err();
    assert!(
        matches!(error, Error::Timeout(Timeout::PerRead)),
        "{error:?}"
    );
    assert_eq!(
        state.lock().unwrap().writes.len(),
        1,
        "body must not be sent"
    );
}

#[test]
fn write_timeout_reports_per_write() {
    let (agent, state) = setup(config());
    state.lock().unwrap().fail_write = true;
    let error = agent.get("http://127.0.0.1/").call().unwrap_err();
    assert!(
        matches!(error, Error::Timeout(Timeout::PerWrite)),
        "{error:?}"
    );
}

#[test]
fn zero_limits_fail_without_invoking_transport_io() {
    for read in [false, true] {
        let config = Agent::config_builder()
            .proxy(None)
            .timeout_per_read(read.then_some(Duration::ZERO))
            .timeout_per_write((!read).then_some(Duration::ZERO))
            .build();
        let (agent, state) = setup(config);
        let error = agent.get("http://127.0.0.1/").call().unwrap_err();
        let reason = if read {
            Timeout::PerRead
        } else {
            Timeout::PerWrite
        };
        assert!(matches!(error, Error::Timeout(actual) if actual == reason));
        let state = state.lock().unwrap();
        assert!(state.reads.is_empty());
        if !read {
            assert!(state.writes.is_empty());
        }
    }
}

#[test]
fn connect_proxy_negotiation_uses_both_limits() {
    use crate::transport::ConnectProxyConnector;
    use crate::transport::time::Instant;
    use crate::unversioned::resolver::Resolver;

    let config = Agent::config_builder()
        .proxy(Some(crate::Proxy::new("http://127.0.0.1:8080").unwrap()))
        .build();
    let state = Arc::new(Mutex::new(State::default()));
    state
        .lock()
        .unwrap()
        .responses
        .push_back(b"HTTP/1.1 200 Connection established\r\n\r\n");
    let script = Script(state.clone());
    let resolver = DefaultResolver::default();
    let uri = "http://example.com/".parse().unwrap();
    let details = ConnectionDetails {
        uri: &uri,
        addrs: resolver.empty(),
        config: &config,
        request_level: false,
        resolver: &resolver,
        now: Instant::now(),
        timeout: NextTimeout {
            per_read: Some(Duration::from_secs(2).into()),
            per_write: Some(Duration::from_secs(3).into()),
            ..NextTimeout::default()
        },
        current_time: Arc::new(Instant::now),
        run_connector: Arc::new(move |details| Ok(script.connect(details, None)?.unwrap().boxed())),
    };
    ConnectProxyConnector::default()
        .connect(&details, None::<()>)
        .unwrap()
        .unwrap();
    let state = state.lock().unwrap();
    assert!(!state.writes.is_empty());
    assert_eq!(state.reads.len(), 1);
    for timeout in &state.writes {
        assert_eq!(timeout.reason, Timeout::PerWrite);
        assert_eq!(*timeout.after, Duration::from_secs(3));
    }
    assert_eq!(state.reads[0].reason, Timeout::PerRead);
    assert_eq!(*state.reads[0].after, Duration::from_secs(2));
}

#[test]
fn layered_read_uses_write_allowance_for_protocol_output() {
    use crate::transport::TransportAdapter;
    use std::io::Write;

    // TLS reads may emit protocol writes (for example, a KeyUpdate response).
    struct ReadWrites(TransportAdapter<ScriptTransport>);

    impl std::fmt::Debug for ReadWrites {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("ReadWrites")
        }
    }

    impl Transport for ReadWrites {
        fn buffers(&mut self) -> &mut dyn Buffers {
            self.0.get_mut().buffers()
        }

        fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
            self.0
                .get_mut()
                .transmit_output(amount, timeout.for_write())
        }

        fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
            self.0.set_timeout(timeout);
            self.0.write_all(b"protocol output")?;
            self.0.get_mut().await_input(timeout.for_read())
        }

        fn is_open(&mut self) -> bool {
            true
        }
    }

    #[derive(Debug)]
    struct Layered(Script);

    impl Connector for Layered {
        type Out = ReadWrites;

        fn connect(
            &self,
            details: &ConnectionDetails,
            _: Option<()>,
        ) -> Result<Option<Self::Out>, Error> {
            Ok(Some(ReadWrites(TransportAdapter::new(
                self.0.connect(details, None)?.unwrap(),
            ))))
        }
    }

    let state = Arc::new(Mutex::new(State::default()));
    state.lock().unwrap().responses.push_back(HEADERS);
    let agent = Agent::with_parts(
        config(),
        Layered(Script(state.clone())),
        DefaultResolver::default(),
    );
    let _response = agent.get("http://127.0.0.1/").call().unwrap();
    let state = state.lock().unwrap();
    let protocol_write = state.writes.last().unwrap();
    assert_eq!(protocol_write.reason, Timeout::PerWrite);
    assert_eq!(*protocol_write.after, Duration::from_secs(7));
}
