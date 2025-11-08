//! Multipart support.
//!
//! //! **NOTE multipart does not (yet) [follow semver][super].**
//!
//! The multipart API is currently provided under unversioned, because we would like feedback on
//! how well it works before stabilizing it. One decision we are uncertain about is that it
//! has a lifetime parameter on the [`Form`] struct. That lifetime allows us to not take ownership of
//! stuff like `&[u8]`, `&str`, `&mut dyn Read`, etc meaning we can keep the cloning and memory allocation
//! to a minimum. The flip side is that it's not easy to pass `Form` around to other
//! threads, mpsc channels, etc.
//!
//! Please provide API feedback in this issue: [issue-1128](https://github.com/algesten/ureq/issues/1128).
//!
use mime_guess::Mime;
use ureq_proto::http::{self, HeaderValue};

use crate::{util::private::Private, AsSendBody, Error, SendBody};
use std::io::{self, Read};
use std::path::Path;

const BOUNDARY_PREFIX: &str = "----formdata-ureq-";
const BOUNDARY_SUFFIX_LEN: usize = 16;

/// A multipart/form-data request.
///
/// Use this to send multipart form data, which is commonly used for file uploads
/// and forms with mixed content types.
///
/// When using [`RequestBuilder::send()`](crate::RequestBuilder::send) with a `Form`,
/// the `Content-Type: multipart/form-data; boundary=...` header is automatically set.
///
/// # Examples
///
/// Basic usage with file upload:
///
/// ```
/// # fn no_run() -> Result<(), ureq::Error> {
/// use ureq::unversioned::multipart::Form;
///
/// let form = Form::new()
///     .text("description", "My uploaded file")
///     .file("upload", "path/to/file.txt")?;
///
/// // Send the form as part of a POST request
/// let response = ureq::post("http://httpbin.org/post")
///     .send(form)?;
/// # Ok(())}
/// ```
///
/// Uploading a file with custom filename and MIME type:
///
/// ```
/// # fn no_run() -> Result<(), ureq::Error> {
/// use ureq::unversioned::multipart::{Form, Part};
///
/// let form = Form::new()
///     .text("description", "My uploaded file")
///     .part(
///         "upload",
///          // File path gives us no clue what it is
///         Part::file("path/to/file").unwrap()
///             // Override the file name
///             .file_name("avatar.jpg")
///             // Override the mime type
///             .mime_str("image/jpeg")?,
///     );
///
/// let response = ureq::post("http://httpbin.org/post")
///     .send(form)?;
/// # Ok(())}
/// ```
pub struct Form<'a> {
    parts: Vec<(&'a str, Part<'a>)>,
    boundary: String,
    state: ReadState,
}

/// A field in a multipart form.
///
/// This gives you more control over the part, such as setting the file name and/or mime type.
///
/// # Example
///
/// Uploading a file with custom filename and MIME type:
///
/// ```
/// # fn no_run() -> Result<(), ureq::Error> {
/// use ureq::unversioned::multipart::{Form, Part};
///
/// let form = Form::new()
///     .text("description", "My uploaded file")
///     .part(
///         "upload",
///          // File path gives us no clue what it is
///         Part::file("path/to/file").unwrap()
///             // Override the file name
///             .file_name("avatar.jpg")
///             // Override the mime type
///             .mime_str("image/jpeg")?,
///     );
///
/// let response = ureq::post("http://httpbin.org/post")
///     .send(form)?;
/// # Ok(())}
/// ```
pub struct Part<'a> {
    inner: PartInner<'a>,
    meta: PartMeta,
}

enum PartInner<'a> {
    Borrowed(SendBody<'a>),
    Owned(SendBody<'static>),
}

struct PartMeta {
    mime: Option<Mime>,
    file_name: Option<String>,
    headers: http::HeaderMap,
}

impl<'a> Default for Form<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Form<'a> {
    /// Creates a new async Form without any content.
    pub fn new() -> Self {
        // Generate a random boundary using getrandom
        // NB. We use getrandom instead of fastrand since fastrand is
        // only present in the dependencies if we use native-tls. In
        // all other cases we have getrandom already.
        let mut random_bytes = [0u8; BOUNDARY_SUFFIX_LEN];
        getrandom::getrandom(&mut random_bytes).expect("failed to generate random boundary");

        // *2 since we're using hex encoding
        let mut boundary = String::with_capacity(BOUNDARY_PREFIX.len() + BOUNDARY_SUFFIX_LEN * 2);

        boundary.push_str(BOUNDARY_PREFIX);
        for byte in random_bytes {
            boundary.push_str(&format!("{:02x}", byte));
        }

        Form {
            parts: Vec::new(),
            boundary,
            state: ReadState::default(),
        }
    }

    /// Get the boundary that this form will use.
    pub fn boundary(&self) -> &str {
        &self.boundary
    }

    /// Add a data field with supplied name and value.
    ///
    /// * `name` is the form field name.
    /// * `value` is the value of the form field.
    pub fn text(mut self, name: &'a str, value: &'a str) -> Self {
        let part = Part::text(value);
        self.parts.push((name, part));
        self
    }

    /// Adds a file field.
    ///
    /// * `name` is the form field name.
    /// * `path` is the path to the file to upload.
    ///
    /// The file name will be extracted from the path, i.e. `path/to/file.txt` will be `file.txt`.
    /// The mime type will be guessed fromt he file extension using the [`mime_guess`][mime_guess] crate.
    ///
    /// If you need finer control over the file name and/or mime type, use
    /// the [`part`][Self::part] method instead.
    ///
    /// [mime_guess]: https://docs.rs/mime_guess/latest/mime_guess/
    pub fn file<P: AsRef<Path>>(mut self, name: &'a str, path: P) -> std::io::Result<Self> {
        let part = Part::file(path)?;
        self.parts.push((name, part));
        Ok(self)
    }

    /// Adds a customized Part.
    ///
    /// * `name` is the form field name.
    /// * `part` is the part to add.
    ///
    /// This allows more fine grained control over the part, such as setting the file name and/or mime type.
    pub fn part(mut self, name: &'a str, part: Part<'a>) -> Self {
        self.parts.push((name, part));
        self
    }
}

impl<'a> Part<'a> {
    /// Create a text part.
    pub fn text(text: &'a str) -> Self {
        Part {
            inner: PartInner::Borrowed(SendBody::from_bytes(text.as_bytes())),
            meta: PartMeta {
                mime: None,
                file_name: None,
                headers: http::HeaderMap::new(),
            },
        }
    }

    /// Create a part from bytes.
    pub fn bytes(bytes: &'a [u8]) -> Self {
        Part {
            inner: PartInner::Borrowed(SendBody::from_bytes(bytes)),
            meta: PartMeta {
                mime: None,
                file_name: None,
                headers: http::HeaderMap::new(),
            },
        }
    }

    /// Create a part from a reader.
    pub fn reader(reader: &'a mut dyn Read) -> Self {
        Part {
            inner: PartInner::Borrowed(SendBody::from_reader(reader)),
            meta: PartMeta {
                mime: None,
                file_name: None,
                headers: http::HeaderMap::new(),
            },
        }
    }

    /// Create a part from an owned reader.
    pub fn owned_reader(reader: impl Read + 'static) -> Part<'a> {
        Part {
            inner: PartInner::Owned(SendBody::from_owned_reader(reader)),
            meta: PartMeta {
                mime: None,
                file_name: None,
                headers: http::HeaderMap::new(),
            },
        }
    }

    /// Create a part from a file.
    pub fn file<P: AsRef<Path>>(path: P) -> std::io::Result<Part<'a>> {
        let mime = mime_guess::from_path(&path).first();
        let file_name = path
            .as_ref()
            .file_name()
            .map(|filename| filename.to_string_lossy().into_owned());
        let file = std::fs::File::open(path)?;
        Ok(Part {
            inner: PartInner::Owned(SendBody::from_file(file)),
            meta: PartMeta {
                mime,
                file_name,
                headers: http::HeaderMap::new(),
            },
        })
    }

    /// Set the file name for this part.
    pub fn file_name(mut self, name: &str) -> Self {
        self.meta.file_name = Some(name.to_string());
        self
    }

    /// Set the MIME type for this part.
    pub fn mime_str(mut self, mime: &str) -> Result<Self, Error> {
        let mime_type = mime.parse().map_err(Error::InvalidMimeType)?;
        self.meta.mime = Some(mime_type);
        Ok(self)
    }

    /// Get the headers for this part.
    pub fn headers(&self) -> &http::HeaderMap {
        &self.meta.headers
    }
}

impl<'a> Private for Form<'a> {}
impl<'a> AsSendBody for Form<'a> {
    fn as_body(&mut self) -> SendBody {
        use crate::send_body::BodyInner;

        let size = self.calculate_size();
        let content_type = format!("multipart/form-data; boundary={}", self.boundary());
        let body: SendBody = (size, BodyInner::Reader(self)).into();
        body.with_content_type(HeaderValue::from_str(&content_type).unwrap())
    }
}

struct ReadState {
    /// Current part index in the form
    part_idx: usize,
    /// Current phase of reading
    phase: Phase,
    /// Offset within the current phase for progressive generation
    phase_offset: usize,
    /// Internal buffer for generating boundaries/headers
    buffer: Vec<u8>,
    /// Current position in buffer for reading
    buffer_pos: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Writing initial or subsequent boundary
    PartBoundary,
    /// Writing headers for current part
    PartHeaders,
    /// Reading body from current part
    PartBody,
    /// Writing the final boundary after all parts
    FinalBoundary,
    /// All done
    Done,
}

impl Default for ReadState {
    fn default() -> Self {
        ReadState {
            part_idx: 0,
            phase: Phase::PartBoundary,
            phase_offset: 0,
            buffer: Vec::with_capacity(8192),
            buffer_pos: 0,
        }
    }
}

impl<'a> Form<'a> {
    /// Calculate the total size of the multipart body if all parts have known sizes.
    /// Returns None if any part has an unknown size.
    fn calculate_size(&self) -> Option<u64> {
        let mut total_size: u64 = 0;

        for (idx, (name, part)) in self.parts.iter().enumerate() {
            // Boundary: "\r\n--{boundary}\r\n" or "--{boundary}\r\n" for first
            let boundary_size = if idx == 0 {
                2 + self.boundary.len() + 2 // "--" + boundary + "\r\n"
            } else {
                2 + 2 + self.boundary.len() + 2 // "\r\n--" + boundary + "\r\n"
            };
            total_size += boundary_size as u64;

            // Headers
            let mut header_size = 0;
            // Content-Disposition: form-data; name="..."
            header_size += b"Content-Disposition: form-data; name=\"".len();
            header_size += name.len();
            header_size += b"\"".len();

            if let Some(ref filename) = part.meta.file_name {
                header_size += b"; filename=\"".len();
                header_size += filename.len();
                header_size += b"\"".len();
            }
            header_size += 2; // \r\n

            // Content-Type if present
            if let Some(ref mime) = part.meta.mime {
                header_size += b"Content-Type: ".len();
                header_size += mime.as_ref().len();
                header_size += 2; // \r\n
            }

            // Custom headers
            for (name, value) in part.meta.headers.iter() {
                header_size += name.as_str().len();
                header_size += 2; // ": "
                header_size += value.len();
                header_size += 2; // \r\n
            }

            // Empty line after headers
            header_size += 2; // \r\n

            total_size += header_size as u64;

            // Body size - need to check if part has known size
            let body_size = match &part.inner {
                PartInner::Borrowed(body) => body.size(),
                PartInner::Owned(body) => body.size(),
            };

            total_size += body_size?; // Return None if any body size is unknown
        }

        // Final boundary: "\r\n--{boundary}--\r\n"
        total_size += (2 + 2 + self.boundary.len() + 2 + 2) as u64;

        Some(total_size)
    }

    /// Fill buffer with boundary string
    fn fill_boundary_buffer(&mut self) -> io::Result<()> {
        self.state.buffer.clear();

        // First part doesn't have leading \r\n
        if self.state.part_idx > 0 {
            self.state.buffer.extend_from_slice(b"\r\n");
        }

        self.state.buffer.extend_from_slice(b"--");
        self.state
            .buffer
            .extend_from_slice(self.boundary.as_bytes());
        self.state.buffer.extend_from_slice(b"\r\n");

        Ok(())
    }

    /// Fill buffer with headers for current part
    fn fill_headers_buffer(&mut self) -> io::Result<()> {
        self.state.buffer.clear();

        let (name, part) = &self.parts[self.state.part_idx];

        // Content-Disposition header
        self.state
            .buffer
            .extend_from_slice(b"Content-Disposition: form-data; name=\"");
        self.state.buffer.extend_from_slice(name.as_bytes());
        self.state.buffer.extend_from_slice(b"\"");

        if let Some(ref filename) = part.meta.file_name {
            self.state.buffer.extend_from_slice(b"; filename=\"");
            self.state.buffer.extend_from_slice(filename.as_bytes());
            self.state.buffer.extend_from_slice(b"\"");
        }

        self.state.buffer.extend_from_slice(b"\r\n");

        // Content-Type header if present
        if let Some(ref mime) = part.meta.mime {
            self.state.buffer.extend_from_slice(b"Content-Type: ");
            self.state
                .buffer
                .extend_from_slice(mime.as_ref().as_bytes());
            self.state.buffer.extend_from_slice(b"\r\n");
        }

        // Custom headers
        for (name, value) in part.meta.headers.iter() {
            self.state
                .buffer
                .extend_from_slice(name.as_str().as_bytes());
            self.state.buffer.extend_from_slice(b": ");
            self.state.buffer.extend_from_slice(value.as_bytes());
            self.state.buffer.extend_from_slice(b"\r\n");
        }

        // Empty line after headers
        self.state.buffer.extend_from_slice(b"\r\n");

        Ok(())
    }

    /// Fill buffer with final boundary
    fn fill_final_boundary_buffer(&mut self) -> io::Result<()> {
        self.state.buffer.clear();

        self.state.buffer.extend_from_slice(b"\r\n--");
        self.state
            .buffer
            .extend_from_slice(self.boundary.as_bytes());
        self.state.buffer.extend_from_slice(b"--\r\n");

        Ok(())
    }

    /// Read from a part's SendBody
    fn read_from_part(part: &mut Part<'a>, buf: &mut [u8]) -> io::Result<usize> {
        match &mut part.inner {
            PartInner::Borrowed(body) => body.read(buf),
            PartInner::Owned(body) => body.read(buf),
        }
    }
}

impl io::Read for Form<'_> {
    fn read(&mut self, mut buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        // Handle empty form
        if self.parts.is_empty() {
            if self.state.phase != Phase::Done {
                self.state.phase = Phase::Done;
            }
            return Ok(0);
        }

        let original_len = buf.len();

        loop {
            // First, try to drain any buffered data
            if self.state.buffer_pos < self.state.buffer.len() {
                let available = self.state.buffer.len() - self.state.buffer_pos;
                let to_copy = available.min(buf.len());
                buf[..to_copy].copy_from_slice(
                    &self.state.buffer[self.state.buffer_pos..self.state.buffer_pos + to_copy],
                );
                self.state.buffer_pos += to_copy;
                buf = &mut buf[to_copy..];

                if buf.is_empty() {
                    return Ok(original_len);
                }
            }

            // Buffer is drained, reset for next use
            self.state.buffer_pos = 0;
            self.state.buffer.clear();

            // Process current phase
            match self.state.phase {
                Phase::Done => {
                    return Ok(original_len - buf.len());
                }
                Phase::PartBoundary => {
                    self.fill_boundary_buffer()?;
                    self.state.phase = Phase::PartHeaders;
                    self.state.phase_offset = 0;
                }
                Phase::PartHeaders => {
                    self.fill_headers_buffer()?;
                    self.state.phase = Phase::PartBody;
                    self.state.phase_offset = 0;
                }
                Phase::PartBody => {
                    // Read directly from the part body into the caller's buffer
                    let part = &mut self.parts[self.state.part_idx].1;
                    let n = Self::read_from_part(part, buf)?;

                    buf = &mut buf[n..];

                    // If we read something, return it
                    if n > 0 {
                        return Ok(original_len - buf.len());
                    }

                    // Part body exhausted, move to next part or final
                    self.state.part_idx += 1;
                    if self.state.part_idx < self.parts.len() {
                        self.state.phase = Phase::PartBoundary;
                    } else {
                        self.state.phase = Phase::FinalBoundary;
                    }
                    self.state.phase_offset = 0;
                }
                Phase::FinalBoundary => {
                    self.fill_final_boundary_buffer()?;
                    self.state.phase = Phase::Done;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_form_read_simple() {
        let form = Form::new()
            .text("field1", "value1")
            .text("field2", "value2");

        let mut result = Vec::new();
        let mut form = form;
        form.read_to_end(&mut result).unwrap();

        let output = String::from_utf8(result).unwrap();

        // Check that it contains the boundary
        assert!(output.contains(&form.boundary));

        // Check field names and values
        assert!(output.contains("Content-Disposition: form-data; name=\"field1\""));
        assert!(output.contains("value1"));
        assert!(output.contains("Content-Disposition: form-data; name=\"field2\""));
        assert!(output.contains("value2"));

        // Check proper endings
        assert!(output.ends_with("--\r\n"));
    }

    #[test]
    fn test_form_read_empty() {
        let form = Form::new();
        let mut result = Vec::new();
        let mut form = form;
        form.read_to_end(&mut result).unwrap();

        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_form_read_with_filename() {
        let form = Form::new().part("upload", Part::text("file content").file_name("test.txt"));

        let mut result = Vec::new();
        let mut form = form;
        form.read_to_end(&mut result).unwrap();

        let output = String::from_utf8(result).unwrap();

        assert!(output.contains("filename=\"test.txt\""));
        assert!(output.contains("file content"));
    }

    #[test]
    fn test_form_read_with_mime() {
        let form = Form::new().part(
            "data",
            Part::bytes(b"binary")
                .mime_str("application/octet-stream")
                .unwrap(),
        );

        let mut result = Vec::new();
        let mut form = form;
        form.read_to_end(&mut result).unwrap();

        let output = String::from_utf8(result).unwrap();

        assert!(output.contains("Content-Type: application/octet-stream"));
        assert!(output.contains("binary"));
    }

    #[test]
    fn test_form_read_incremental() {
        let form = Form::new().text("field", "data");

        let mut form = form;
        let mut result = Vec::new();

        // Read in small chunks to test buffering
        let mut buf = [0u8; 16];
        loop {
            let n = form.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            result.extend_from_slice(&buf[..n]);
        }

        let output = String::from_utf8(result).unwrap();
        assert!(output.contains("field"));
        assert!(output.contains("data"));
    }

    #[test]
    fn test_form_size_calculation() {
        let form = Form::new()
            .text("field1", "value1")
            .text("field2", "value2");

        // Calculate expected size
        let size = form.calculate_size();
        assert!(size.is_some(), "Size should be calculable for text fields");

        // Read the actual data
        let mut result = Vec::new();
        let mut form = form;
        form.read_to_end(&mut result).unwrap();

        // Verify the size matches
        assert_eq!(
            size.unwrap() as usize,
            result.len(),
            "Calculated size should match actual size"
        );
    }

    #[test]
    fn test_form_with_reader_no_size() {
        use std::io::Cursor;

        // A reader with unknown size
        let mut data = Cursor::new(b"some data".to_vec());
        let form = Form::new().part("file", Part::reader(&mut data));

        // Should not be able to calculate size for readers without known size
        let size = form.calculate_size();
        assert!(
            size.is_none(),
            "Size should be None for readers without known size"
        );
    }

    #[test]
    fn test_invalid_mime_type() {
        // Invalid MIME type should return an error
        let result = Part::text("data").mime_str("invalid/mime/type/with/too/many/slashes");
        assert!(result.is_err());

        assert!(matches!(result, Err(Error::InvalidMimeType(_))));
    }

    #[test]
    fn test_form_sets_content_type() {
        let mut form = Form::new().text("field", "value");
        let mut body = form.as_body();

        let content_type = body.take_content_type();
        assert!(content_type.is_some());
        let ct = content_type.unwrap();
        let ct_str = ct.to_str().unwrap();
        assert!(ct_str.starts_with("multipart/form-data; boundary="));
        assert!(ct_str.contains(&form.boundary()));
    }
}
