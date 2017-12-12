use std::io::Read;
use std::io::Result;

pub struct VecRead {
    bytes: Vec<u8>,
    index: usize,
}

impl VecRead {
    pub fn new(bytes: &[u8]) -> Self {
        Self::from_vec(bytes.to_owned())
    }
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        VecRead {
            bytes,
            index: 0,
        }
    }
    pub fn from_str(s: &str) -> Self {
        Self::new(s.as_bytes())
    }
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
}

impl Read for VecRead {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let len = buf.len().min(self.bytes.len() - self.index);
        (&mut buf[0..len]).copy_from_slice(&self.bytes[self.index..self.index + len]);
        self.index += len;
        Ok(len)
    }
}
