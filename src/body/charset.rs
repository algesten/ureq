use encoding_rs::{Decoder, Encoder, Encoding};
use std::fmt;
use std::io;

use crate::util::ConsumeBuf;

const MAX_OUTPUT: usize = 4096;

/// Charset transcoder
pub(crate) struct CharCodec<R> {
    reader: R,
    input_buf: ConsumeBuf,
    dec: Option<Decoder>,
    enc: Option<Encoder>,
    output_buf: ConsumeBuf,
    reached_end: bool,
}

impl<R> CharCodec<R>
where
    R: io::Read,
{
    pub fn new(reader: R, from: &'static Encoding, to: &'static Encoding) -> Self {
        CharCodec {
            reader,
            input_buf: ConsumeBuf::new(8192),
            dec: Some(from.new_decoder()),
            enc: if to == encoding_rs::UTF_8 {
                None
            } else {
                Some(to.new_encoder())
            },
            output_buf: ConsumeBuf::new(MAX_OUTPUT),
            reached_end: false,
        }
    }
}

impl<R: io::Read> io::Read for CharCodec<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.reached_end && self.output_buf.unconsumed().is_empty() {
            return Ok(0);
        }

        // Ensure we have at least 4 bytes of input to decode, or we've reached EOF
        while self.input_buf.unconsumed().len() < 4 && !self.reached_end {
            let free = self.input_buf.free_mut();
            let n = self.reader.read(free)?;
            if n == 0 {
                // Reached EOF
                self.reached_end = true;
                break;
            }
            self.input_buf.add_filled(n);
        }

        let input = self.input_buf.unconsumed();

        if self.output_buf.free_mut().len() < 4 {
            self.output_buf.add_space(1024);
        }
        let output = self.output_buf.free_mut();

        if let Some(dec) = &mut self.dec {
            let (_, input_used, output_used, _had_errors) =
                dec.decode_to_utf8(input, output, self.reached_end);

            self.input_buf.consume(input_used);
            self.output_buf.add_filled(output_used);

            if self.reached_end {
                // Can't be used again
                self.dec = None;
            }
        }

        // The output_buf contains UTF-8 data produced by decode_to_utf8(), which guarantees
        // char boundaries. When we have an encoder (converting UTF-8 to another encoding),
        // encode_from_utf8() returns input_used on char boundaries, so consume() maintains
        // the invariant. When we have no encoder (already UTF-8), we copy arbitrary byte
        // amounts, but that's safe because we never need to parse it as UTF-8 - we just
        // pass the bytes through.
        let bytes = self.output_buf.unconsumed();

        let amount = if let Some(enc) = &mut self.enc {
            // unwrap is ok because it is on a char boundary, and non-utf8 chars have been replaced
            let utf8 = std::str::from_utf8(bytes).unwrap();
            let (_, input_used, output_used, _) = enc.encode_from_utf8(utf8, buf, self.reached_end);
            self.output_buf.consume(input_used);

            if self.reached_end {
                // Can't be used again
                self.enc = None;
            }

            output_used
        } else {
            // No encoder, we want utf8
            let max = bytes.len().min(buf.len());
            buf[..max].copy_from_slice(&bytes[..max]);
            self.output_buf.consume(max);
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
    use std::io::Read;

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

    #[test]
    fn multibyte_chars() {
        const CHAR_COUNT: usize = 8193;

        let cases: &[(&[u8], _, _)] = &[
            // ž
            // in utf-8: 0xC5 0xBE
            // CharCodec stops at 16384 bytes
            (
                &[0x01, 0x7E],
                encoding_rs::UTF_16BE,
                "2B utf-16be chars -> 2B utf-8",
            ),
            (
                &[0xB8],
                encoding_rs::ISO_8859_15,
                "1B iso-8859-15 chars -> 2B utf-8",
            ),
            // ‽
            // in utf-8: 0xE2 0x80 0xBD
            // CharCodec stops at 24576 bytes
            (
                &[0x20, 0x3D],
                encoding_rs::UTF_16BE,
                "2B utf-16be chars -> 3B utf-8",
            ),
        ];

        for (char_bytes, from_encoding, case_name) in cases {
            let source_bytes = char_bytes.repeat(CHAR_COUNT);

            let encoding_rs_result = from_encoding.decode(&source_bytes).0;
            let char_codec_result = io::read_to_string(CharCodec::new(
                source_bytes.as_slice(),
                from_encoding,
                encoding_rs::UTF_8,
            ))
            .unwrap();

            assert_eq!(
                char_codec_result.len(),
                encoding_rs_result.len(),
                "{CHAR_COUNT} * {case_name}",
            );
        }
    }

    /// Helper that limits reads to 1-3 bytes at a time to force partial reads
    struct SlowReader<'a> {
        data: &'a [u8],
        pos: usize,
    }

    impl<'a> SlowReader<'a> {
        fn new(data: &'a [u8]) -> Self {
            SlowReader { data, pos: 0 }
        }
    }

    impl<'a> io::Read for SlowReader<'a> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            // Read 1-3 bytes at a time
            let max_read = 3.min(buf.len()).min(self.data.len() - self.pos);
            let amount = 1.max(max_read);
            buf[..amount].copy_from_slice(&self.data[self.pos..self.pos + amount]);
            self.pos += amount;
            Ok(amount)
        }
    }

    #[test]
    fn char_boundary_utf8_to_utf8_small_reads() {
        // Test UTF-8 to UTF-8 with multibyte chars and small reads
        let input = "åäö🎉café";
        let input_bytes = input.as_bytes();

        let mut codec = CharCodec::new(
            SlowReader::new(input_bytes),
            encoding_rs::UTF_8,
            encoding_rs::UTF_8,
        );

        // Read in very small chunks
        let mut result = Vec::new();
        let mut buf = [0u8; 2];
        loop {
            let n = codec.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            result.extend_from_slice(&buf[..n]);
        }

        let output = String::from_utf8(result).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn char_boundary_iso8859_to_utf8_small_reads() {
        // Test ISO-8859-15 to UTF-8 - this doesn't use encoder but tests decoder
        let input = "café";

        // Encode to ISO-8859-15 first
        let (encoded, _, _) = encoding_rs::ISO_8859_15.encode(input);

        // Decode back to UTF-8 through CharCodec with slow reader
        let mut codec = CharCodec::new(
            SlowReader::new(&encoded),
            encoding_rs::ISO_8859_15,
            encoding_rs::UTF_8,
        );

        // Read in very small chunks to force partial reads
        let mut result = Vec::new();
        let mut buf = [0u8; 1];
        loop {
            let n = codec.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            result.extend_from_slice(&buf[..n]);
        }

        let output = String::from_utf8(result).unwrap();
        assert_eq!(output, input);
    }
}
