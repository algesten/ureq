//! Multipart support.

use mime_guess::Mime;
use ureq_proto::http;

use crate::{util::private::Private, AsSendBody, SendBody};
use std::io::{self, Read};
use std::path::Path;

const BOUNDARY_PREFIX: &str = "----formdata-ureq-";
const BOUNDARY_SUFFIX_LEN: usize = 16;

/// A multipart/form-data request.
///
/// Use this to send multipart form data, which is commonly used for file uploads
/// and forms with mixed content types.
///
/// # Examples
///
/// Basic usage with file upload:
///
/// ```
/// # async fn no_run() -> Result<(), ureq::Error> {
/// use ureq::multipart::Form;
///
/// let form = Form::new()
///     .text("description", "My uploaded file")
///     .file("upload", "path/to/file.txt").await?;
///
/// // Send the form as part of a POST request
/// let response = ureq::post("http://httpbin.org/post")
///     .send(form)?;
/// # Ok(())}
/// ```
///
/// Adding different types of parts:
///
/// ```
/// # fn no_run() -> Result<(), ureq::Error> {
/// use ureq::multipart::{Form, Part};
///
/// let data = b"binary data";
/// let form = Form::new()
///     .text("field1", "text value")
///     .part("field2", Part::bytes(data))
///     .part("field3", Part::text("another text").file_name("data.txt"));
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

impl<'a> Form<'a> {
    /// Creates a new async Form without any content.
    pub fn new() -> Self {
        // Generate a random boundary using fastrand
        use std::iter::repeat_with;
        let mut boundary = String::with_capacity(BOUNDARY_PREFIX.len() + BOUNDARY_SUFFIX_LEN);
        boundary.push_str(BOUNDARY_PREFIX);
        boundary.extend(repeat_with(fastrand::alphanumeric).take(BOUNDARY_SUFFIX_LEN));

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
    pub fn text(mut self, name: &'a str, value: &'a str) -> Self {
        let part = Part::text(value);
        self.parts.push((name, part));
        self
    }

    /// Adds a file field.
    pub async fn file<P: AsRef<Path>>(mut self, name: &'a str, path: P) -> std::io::Result<Self> {
        let part = Part::file(path).await?;
        self.parts.push((name, part));
        Ok(self)
    }

    /// Adds a customized Part.
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
    pub async fn file<P: AsRef<Path>>(path: P) -> std::io::Result<Part<'a>> {
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
    pub fn mime_str(mut self, mime: &str) -> Self {
        if let Ok(mime_type) = mime.parse() {
            self.meta.mime = Some(mime_type);
        }
        self
    }

    /// Get the headers for this part.
    pub fn headers(&self) -> &http::HeaderMap {
        &self.meta.headers
    }
}

impl<'a> Private for Form<'a> {}
impl<'a> AsSendBody for Form<'a> {
    fn as_body(&mut self) -> SendBody {
        // TODO(martin): here we should be able to know the size of the body
        // and therefore use (some new) constructor in SendBody that sets the size.
        SendBody::from_reader(self)
    }
}

#[derive(Default)]
struct ReadState {
    // TODO(martin)
}

impl io::Read for Form<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // TODO(martin): implement a streaming reader of the multipart body.
        todo!()
    }
}
