use std::io;
use std::io::Read;

/// Wrapper that attempts to buffer up to `max_size` amount on construction.
///
/// If the underlying reader is fully buffered, the reader can be rewinded to
/// restart the position from 0.
pub(crate) struct RewindReader<R> {
    state: RewindReaderState,
    inner: Option<R>,
}

enum RewindReaderState {
    Begin { buffer_size: usize },
    Buffered { buffer: Vec<u8>, pos: usize },
    Unbuffered,
}

impl<R: Read> RewindReader<R> {
    pub fn new(buffer_size: usize, inner: R) -> Self {
        RewindReader {
            state: RewindReaderState::Begin { buffer_size },
            inner: Some(inner),
        }
    }

    /// Check if it's possible to rewind the position. This is only possible
    /// if we fully buffered the content of the underlying reader.
    pub fn can_rewind(&self) -> bool {
        matches!(&self.state, RewindReaderState::Buffered { .. }) && self.inner.is_none()
    }

    /// Rewind position to 0, if possible.
    pub fn rewind(&mut self) {
        match &mut self.state {
            RewindReaderState::Buffered { pos, .. } => {
                *pos = 0;
            }
            _ => {}
        }
    }
}

impl<R: Read> Read for RewindReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match &mut self.state {
                RewindReaderState::Begin { buffer_size } => {
                    if *buffer_size > 0 {
                        // max_size indicates we want to buffer.
                        // Prepare a vector with the exact size needed for this.
                        let mut buffer = Vec::with_capacity(*buffer_size);
                        buffer.resize(*buffer_size, 0);

                        let reader = self.inner.as_mut().expect("Reader on Begin");

                        // Fill the buffer from the reader. "exhausted" indicates that
                        // we reached the end of the reader, in which case we must not
                        // call it again.
                        let exhausted = fill_buffer(reader, &mut buffer)?;

                        if exhausted {
                            // The entire reader was used. We are not allowed to
                            // use it again.
                            self.inner.take();
                        }

                        self.state = RewindReaderState::Buffered { buffer, pos: 0 }
                    } else {
                        self.state = RewindReaderState::Unbuffered;
                    }
                    // try again
                    continue;
                }

                RewindReaderState::Buffered { buffer, pos } => {
                    let buf_left = &buffer[*pos..];

                    if buf_left.len() == 0 {
                        // Nothing more in buffer. Progress state to unbuffered.
                        self.state = RewindReaderState::Unbuffered;
                        continue;
                    }

                    let amt = buf.len().min(buf_left.len());

                    (&mut buf[..amt]).copy_from_slice(&buf_left[..amt]);
                    *pos += amt;

                    return Ok(amt);
                }

                RewindReaderState::Unbuffered => {
                    if let Some(inner) = &mut self.inner {
                        let n = inner.read(buf)?;
                        return Ok(n);
                    } else {
                        return Ok(0);
                    }
                }
            }
        }
    }
}

/// Attempt to read the buffer to its current length.
/// Returns true if the reader was exhausted.
fn fill_buffer<R: Read>(reader: &mut R, buf: &mut Vec<u8>) -> io::Result<bool> {
    // Loop to try and fill the buffer.
    let mut p = 0;
    loop {
        let to_read = buf.len() - p;

        if to_read == 0 {
            // the entire buffer is full. might still be contents in the reader.
            return Ok(false);
        }

        let n = reader.read(&mut buf[to_read..])?;

        if n == 0 {
            // shrink buffer to the size that we have read contents.
            buf.truncate(p);

            // reader was exhausted.
            return Ok(true);
        }

        p += n;
    }
}
