use encoding_rs::{Decoder, Encoder, Encoding};
use std::fmt;
use std::io::{self, BufRead, BufReader};

use crate::util::ConsumeBuf;

const MAX_OUTPUT: usize = 4096;

/// Charset transcoder
pub(crate) struct CharCodec<R> {
    reader: BufReader<R>,
    dec: Option<Decoder>,
    enc: Option<Encoder>,
    buf: ConsumeBuf,
    reached_end: bool,
}

impl<R> CharCodec<R>
where
    R: io::Read,
{
    pub fn new(reader: R, from: &'static Encoding, to: &'static Encoding) -> Self {
        CharCodec {
            reader: BufReader::new(reader),
            dec: Some(from.new_decoder()),
            enc: if to == encoding_rs::UTF_8 {
                None
            } else {
                Some(to.new_encoder())
            },
            buf: ConsumeBuf::new(MAX_OUTPUT),
            reached_end: false,
        }
    }
}

impl<R: io::Read> io::Read for CharCodec<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.reached_end && self.buf.unconsumed().is_empty() {
            return Ok(0);
        }

        let input = 'read: {
            if self.buf.unconsumed().len() > MAX_OUTPUT / 4 {
                // Do not keep filling if we have unused output.
                break 'read self.reader.buffer();
            }

            let tmp = self.reader.fill_buf()?;
            let tmp_len = tmp.len();
            if tmp_len >= 4 {
                // We need some minimum input to make progress.
                break 'read tmp;
            }

            let tmp2 = self.reader.fill_buf()?;
            if tmp2.len() == tmp_len {
                // Made no progress. That means we reached the end.
                self.reached_end = true;
            }

            tmp2
        };

        if self.buf.free_mut().len() < 4 {
            self.buf.add_space(1024);
        }
        let output = self.buf.free_mut();

        if let Some(dec) = &mut self.dec {
            let (_, input_used, output_used, _had_errors) =
                dec.decode_to_utf8(input, output, self.reached_end);

            self.reader.consume(input_used);
            self.buf.add_filled(output_used);

            if self.reached_end {
                // Can't be used again
                self.dec = None;
            }
        }

        // guaranteed to be on a char boundary by encoding_rs
        let bytes = self.buf.unconsumed();

        let amount = if let Some(enc) = &mut self.enc {
            // unwrap is ok because it is on a char boundary, and non-utf8 chars have been replaced
            let utf8 = std::str::from_utf8(bytes).unwrap();
            let (_, input_used, output_used, _) = enc.encode_from_utf8(utf8, buf, self.reached_end);
            self.buf.consume(input_used);

            if self.reached_end {
                // Can't be used again
                self.enc = None;
            }

            output_used
        } else {
            // No encoder, we want utf8
            let max = bytes.len().min(buf.len());
            buf[..max].copy_from_slice(&bytes[..max]);
            self.buf.consume(max);
            max
        };

        Ok(amount)
    }
}

impl<R> fmt::Debug for CharCodec<R> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "CharCodec {{ from: {}, to: {} }}",
            self.dec
                .as_ref()
                .map(|d| d.encoding().name())
                .unwrap_or(encoding_rs::UTF_8.name()),
            self.enc
                .as_ref()
                .map(|e| e.encoding())
                .unwrap_or(encoding_rs::UTF_8)
                .name()
        )
    }
}

#[cfg(all(test, feature = "_test"))]
mod test {
    use super::*;

    #[test]
    fn create_encodings() {
        assert!(Encoding::for_label(b"utf8").is_some());
        assert_eq!(Encoding::for_label(b"utf8"), Encoding::for_label(b"utf-8"));
    }

    #[test]
    #[cfg(feature = "charset")]
    fn non_ascii_reason() {
        use crate::test::init_test_log;
        use crate::Agent;

        init_test_log();
        let agent: Agent = Agent::config_builder().max_redirects(0).build().into();

        let res = agent
            .get("https://my.test/non-ascii-reason")
            .call()
            .unwrap();
        assert_eq!(res.status(), 302);
    }
}
