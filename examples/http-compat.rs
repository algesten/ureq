use ureq::Agent;

fn main() {
    let agent = Agent::new();
    let request = http::Request::builder()
        .method(http::Method::GET)
        .uri(http::Uri::from_static("http://localhost:9000"))
        .body("Hello, world!")
        .unwrap();
    let response = agent.send_http(request).unwrap();
    let body = String::from_utf8_lossy(response.body());
    println!("Response Body: {}", body);
}
