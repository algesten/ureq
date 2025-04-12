use crate::util::ConsumeBuf;

/// Abstraction over input/output buffers.
///
/// In ureq, the buffers are provided by the [`Transport`](crate::transport::Transport).
pub trait Buffers {
    /// Mut handle to output buffers to write new data. Data is always
    /// written from `0..`.
    fn output(&mut self) -> &mut [u8];

    /// Unconsumed bytes in the input buffer as read only.
    ///
    /// The input buffer is written to by using [`Buffers::input_append_buf`] followed by
    /// [`Buffers::input_appended`] to indiciate how many additional bytes were added to the
    /// input.
    ///
    /// This buffer should return the total unconsumed bytes.
    ///
    /// Example: if the internal buffer is `input: Vec<u8>`, and we have counters for
    /// `filled: usize` and `consumed: usize`. This returns `&input[consumed..filled]`.
    fn input(&self) -> &[u8];

    /// Input buffer to write to. This can be called despite there being unconsumed bytes
    /// left in the buffer already.
    ///
    /// Example: if the internal buffer is `input: Vec<u8>`, and we have counters for
    /// `filled: usize` and `consumed: usize`. This returns `&mut input[filled..]`.
    fn input_append_buf(&mut self) -> &mut [u8];

    /// Add a number of read bytes into [`Buffers::input_append_buf()`].
    ///
    /// Example: if the internal buffer is `input: Vec<u8>`, and we have counters for
    /// `filled: usize` and `consumed: usize`, this increases `filled`.
    fn input_appended(&mut self, amount: usize);

    /// Consume a number of bytes from `&input`.
    ///
    /// Example: if the internal buffer is `input: Vec<u8>`, and we have counters for
    /// `filled: usize` and `consumed: usize`, this increases `consumed`.
    fn input_consume(&mut self, amount: usize);

    /// Helper to get a scratch buffer (`tmp`) and the output buffer. This is used when
    /// sending the request body in which case we use a `Read` trait to read from the
    /// [`SendBody`](crate::SendBody) into tmp and then write it to the output buffer.
    fn tmp_and_output(&mut self) -> (&mut [u8], &mut [u8]);

    /// Helper to determine if the `&input` already holds unconsumed data or we need to
    /// read more input from the transport. This indicates two things:
    ///
    /// 1. There is unconsumed data in the input buffer
    /// 2. The last call to consume was > 0.
    ///
    /// Step 2 is because the input buffer might contain half a response body, and we
    /// cannot parse it until we got the entire buffer. In this case the transport must
    /// read more data first.
    fn can_use_input(&self) -> bool;
}

/// Default buffer implementation.
///
/// The buffers are lazy such that no allocations are made until needed. That means
/// a [`Transport`](crate::transport::Transport) implementation can freely instantiate
/// the `LazyBuffers`.
#[derive(Debug)]
pub struct LazyBuffers {
    input_size: usize,
    output_size: usize,

    input: ConsumeBuf,
    output: Vec<u8>,

    progress: bool,
}

impl LazyBuffers {
    /// Create a new buffer.
    ///
    /// The sizes provided are not allocated until we need to.
    pub fn new(input_size: usize, output_size: usize) -> Self {
        assert!(input_size > 0);
        assert!(output_size > 0);

        LazyBuffers {
            input_size,
            output_size,

            // Vectors don't allocate until they get a size.
            input: ConsumeBuf::new(0),
            output: vec![],

            progress: false,
        }
    }

    fn ensure_allocation(&mut self) {
        if self.output.len() < self.output_size {
            self.output.resize(self.output_size, 0);
        }
        if self.input.unconsumed().len() < self.input_size {
            self.input.resize(self.input_size);
        }
    }
}

impl Buffers for LazyBuffers {
    fn output(&mut self) -> &mut [u8] {
        self.ensure_allocation();
        &mut self.output
    }

    fn input(&self) -> &[u8] {
        self.input.unconsumed()
    }

    fn input_append_buf(&mut self) -> &mut [u8] {
        self.ensure_allocation();
        self.input.free_mut()
    }

    fn tmp_and_output(&mut self) -> (&mut [u8], &mut [u8]) {
        self.ensure_allocation();
        const MIN_TMP_SIZE: usize = 10 * 1024;

        let tmp_available = self.input.free_mut().len();

        if tmp_available < MIN_TMP_SIZE {
            // The tmp space is used for reading the request body from the
            // Body as a Read. There's an outside chance there isn't any space
            // left in the input buffer if we have done Await100 and the peer
            // started sending a ton of data before we asked for it.
            // It's a pathological situation that we don't need to make work well.
            let needed = MIN_TMP_SIZE - tmp_available;
            self.input.resize(self.input.unconsumed().len() + needed);
        }

        (self.input.free_mut(), &mut self.output)
    }

    fn input_appended(&mut self, amount: usize) {
        self.input.add_filled(amount);
    }

    fn input_consume(&mut self, amount: usize) {
        self.progress = amount > 0;
        self.input.consume(amount);
    }

    fn can_use_input(&self) -> bool {
        !self.input.unconsumed().is_empty() && self.progress
    }
}
