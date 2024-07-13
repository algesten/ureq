use crate::util::ConsumeBuf;

pub trait Buffers {
    fn output(&self) -> &[u8];
    fn output_mut(&mut self) -> &mut [u8];
    fn input(&self) -> &[u8];
    fn input_mut(&mut self) -> &mut [u8];
    fn input_and_output(&mut self) -> (&[u8], &mut [u8]);
    fn tmp_and_output(&mut self) -> (&mut [u8], &mut [u8]);
    fn add_filled(&mut self, amount: usize);
    fn consume(&mut self, amount: usize);
    fn can_use_input(&self) -> bool;
}

pub struct LazyBuffers {
    input_size: usize,
    output_size: usize,

    input: ConsumeBuf,
    output: Vec<u8>,

    progress: bool,
}

impl LazyBuffers {
    pub fn empty() -> Self {
        Self::new(0, 0)
    }

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
        if self.input.len() < self.input_size {
            self.input.resize(self.input_size);
        }
    }
}

impl Buffers for LazyBuffers {
    fn output(&self) -> &[u8] {
        &self.output
    }

    fn output_mut(&mut self) -> &mut [u8] {
        self.ensure_allocation();
        &mut self.output
    }

    fn input(&self) -> &[u8] {
        self.input.unconsumed()
    }

    fn input_mut(&mut self) -> &mut [u8] {
        self.ensure_allocation();
        self.input.free_mut()
    }

    fn input_and_output(&mut self) -> (&[u8], &mut [u8]) {
        self.ensure_allocation();
        (self.input.unconsumed(), &mut self.output)
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
            self.input.resize(self.input.len() + needed);
        }

        (self.input.free_mut(), &mut self.output)
    }

    fn add_filled(&mut self, amount: usize) {
        self.input.add_filled(amount);
    }

    fn consume(&mut self, amount: usize) {
        self.progress = amount > 0;
        self.input.consume(amount);
    }

    fn can_use_input(&self) -> bool {
        !self.input.is_empty() && self.progress
    }
}

pub struct NoBuffers;

impl Buffers for NoBuffers {
    fn output(&self) -> &[u8] {
        &[]
    }

    fn output_mut(&mut self) -> &mut [u8] {
        &mut []
    }

    fn input(&self) -> &[u8] {
        &[]
    }

    fn input_mut(&mut self) -> &mut [u8] {
        &mut []
    }

    fn input_and_output(&mut self) -> (&[u8], &mut [u8]) {
        (&[], &mut [])
    }

    fn tmp_and_output(&mut self) -> (&mut [u8], &mut [u8]) {
        (&mut [], &mut [])
    }

    fn add_filled(&mut self, _amount: usize) {
        unreachable!()
    }

    fn consume(&mut self, _amount: usize) {
        unreachable!()
    }

    fn can_use_input(&self) -> bool {
        unreachable!()
    }
}
