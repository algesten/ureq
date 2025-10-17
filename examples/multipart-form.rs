use std::error::Error;
use std::path::PathBuf;

use ureq::multipart::Multipart;

fn main() -> Result<(), Box<dyn Error>> {
    let mut file_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    file_path.push("examples/multipart-form.rs");
    let form = Multipart::new()
        .add_text("foo", "bar")
        .add_file("source", &file_path)
        .prepare()?;
    let res = ureq::post("https://httpbin.org/post")
        .send_multipart_form(form)?
        .into_body()
        .read_to_string()?;
    println!("{res}");
    Ok(())
}
