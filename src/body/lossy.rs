use std::io;

use utf8::DecodeError;

use crate::util::ConsumeBuf;

const REPLACEMENT_CHAR: u8 = b'?';
const MIN_BUF: usize = 8;

pub struct LossyUtf8Reader<R> {
    reader: R,
    ended: bool,
    input: ConsumeBuf,
    valid_len: usize,
}
impl<R> LossyUtf8Reader<R> {
    pub(crate) fn new(reader: R) -> Self {
        Self {
            reader,
            ended: false,
            input: ConsumeBuf::new(8),
            valid_len: 0,
        }
    }

    fn process_input(&mut self) -> usize {
        match utf8::decode(self.input.unconsumed()) {
            Ok(_) => {
                // Entire input is valid
                self.input.unconsumed().len()
            }
            Err(e) => match e {
                DecodeError::Invalid {
                    valid_prefix,
                    invalid_sequence,
                    ..
                } => {
                    let valid_len = valid_prefix.len();
                    let invalid_len = invalid_sequence.len();

                    // Switch out the problem input chars
                    let replace_in = self.input.unconsumed_mut();
                    for i in 0..invalid_len {
                        replace_in[valid_len + i] = REPLACEMENT_CHAR;
                    }

                    valid_len + invalid_len
                }
                DecodeError::Incomplete { valid_prefix, .. } => {
                    let valid_len = valid_prefix.len();

                    if self.ended {
                        // blank the rest
                        let replace_in = self.input.unconsumed_mut();
                        let invalid_len = replace_in.len() - valid_len;
                        for i in 0..invalid_len {
                            replace_in[valid_len + i] = REPLACEMENT_CHAR;
                        }
                        valid_len + invalid_len
                    } else {
                        valid_len
                    }
                }
            },
        }
    }
}

impl<R: io::Read> io::Read for LossyUtf8Reader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Match the input buffer size
        if !self.ended {
            let total_len = self.input.unconsumed().len() + self.input.free_mut().len();
            let wanted_len = buf.len().max(MIN_BUF);
            if wanted_len < total_len {
                self.input.add_space(total_len - wanted_len);
            }
        }

        // Fill up to a point where we definitely will make progress.
        while !self.ended && self.input.unconsumed().len() < MIN_BUF {
            let amount = self.reader.read(self.input.free_mut())?;
            self.input.add_filled(amount);

            if amount == 0 {
                self.ended = true;
            }
        }

        if self.ended && self.input.unconsumed().is_empty() {
            return Ok(0);
        }

        if self.valid_len == 0 {
            self.valid_len = self.process_input();
            assert!(self.valid_len > 0);
        }

        let src = &self.input.unconsumed()[..self.valid_len];
        let max = src.len().min(buf.len());
        buf[..max].copy_from_slice(&src[..max]);
        self.input.consume(max);

        self.valid_len -= max;

        Ok(max)
    }
}

#[cfg(test)]
mod test {
    use std::io::Read;

    use super::*;

    fn do_reader<'a>(bytes: &'a mut [&'a [u8]]) -> String {
        let mut r = LossyUtf8Reader::new(TestReader(bytes));
        let mut buf = String::new();
        r.read_to_string(&mut buf).unwrap();
        buf
    }

    #[test]
    fn ascii() {
        assert_eq!(do_reader(&mut [b"abc123"]), "abc123");
    }

    #[test]
    fn utf8_one_read() {
        assert_eq!(do_reader(&mut ["åiåaäeö".as_bytes()]), "åiåaäeö");
    }

    #[test]
    fn utf8_chopped_single_char() {
        assert_eq!(do_reader(&mut [&[195], &[165]]), "å");
    }

    #[test]
    fn utf8_chopped_prefix_ascii() {
        assert_eq!(do_reader(&mut [&[97, 97, 97, 195], &[165]]), "aaaå");
    }

    #[test]
    fn utf8_chopped_suffix_ascii() {
        assert_eq!(do_reader(&mut [&[195], &[165, 97, 97, 97]]), "åaaa");
    }

    #[test]
    fn utf8_broken_single() {
        assert_eq!(do_reader(&mut [&[195]]), "?");
    }

    #[test]
    fn utf8_broken_suffix_ascii() {
        assert_eq!(do_reader(&mut [&[195, 97, 97, 97]]), "?aaa");
    }

    #[test]
    fn utf8_broken_prefix_ascii() {
        assert_eq!(do_reader(&mut [&[97, 97, 97, 195]]), "aaa?");
    }

    #[test]
    fn hiragana() {
        assert_eq!(do_reader(&mut ["あいうえお".as_bytes()]), "あいうえお");
    }

    #[test]
    fn emoji() {
        assert_eq!(do_reader(&mut ["✅✅✅".as_bytes()]), "✅✅✅");
    }

    #[test]
    fn leftover() {
        let s = "あ";
        assert_eq!(s.as_bytes(), &[227, 129, 130]);

        let mut buf = [0; 2];
        let mut r = LossyUtf8Reader::new(s.as_bytes());

        assert_eq!(r.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf[..], &[227, 129]);

        assert_eq!(r.read(&mut buf).unwrap(), 1);
        assert_eq!(&buf[..1], &[130]);

        assert_eq!(r.read(&mut buf).unwrap(), 0);
    }

    struct TestReader<'a>(&'a mut [&'a [u8]]);

    impl<'a> io::Read for TestReader<'a> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.0.iter().all(|c| c.is_empty()) {
                return Ok(0);
            }

            let pos = self.0.iter().position(|c| !c.is_empty()).unwrap();
            let cur = &self.0[pos];

            let max = cur.len().min(buf.len());
            buf[..max].copy_from_slice(&cur[..max]);

            self.0[pos] = &cur[max..];

            Ok(max)
        }
    }
}
