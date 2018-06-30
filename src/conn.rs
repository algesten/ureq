use std::io::Write;
use url::Url;
use chunked_transfer;

const CHUNK_SIZE: usize = 1024 * 1024;

#[derive(Debug, Default, Clone)]
pub struct ConnectionPool {}

impl ConnectionPool {
    pub fn new() -> Self {
        ConnectionPool {}
    }
}

fn send_body(body: SizedReader, do_chunk: bool, stream: &mut Stream) -> IoResult<()> {
    if do_chunk {
        pipe(body.reader, chunked_transfer::Encoder::new(stream))?;
    } else {
        pipe(body.reader, stream)?;
    }

    Ok(())
}

fn pipe<R, W>(mut reader: R, mut writer: W) -> IoResult<()>
where
    R: Read,
    W: Write,
{
    let mut buf = [0_u8; CHUNK_SIZE];
    loop {
        let len = reader.read(&mut buf)?;
        if len == 0 {
            break;
        }
        writer.write_all(&buf[0..len])?;
    }
    Ok(())
}
