/// Fixed-capacity FIFO that drops the newest item when full.
pub(crate) struct RingBuffer<T: Copy, const N: usize> {
    items: [Option<T>; N],
    head: usize,
    tail: usize,
    len: usize,
    dropped: usize,
}

impl<T: Copy, const N: usize> RingBuffer<T, N> {
    pub(crate) const fn new() -> Self {
        assert!(N != 0, "ring buffer capacity must be non-zero");
        Self {
            items: [None; N],
            head: 0,
            tail: 0,
            len: 0,
            dropped: 0,
        }
    }

    pub(crate) fn push(&mut self, item: T) {
        if self.len == N {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.items[self.tail] = Some(item);
        self.tail = (self.tail + 1) % N;
        self.len += 1;
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let item = self.items[self.head].take();
        self.head = (self.head + 1) % N;
        self.len -= 1;
        item
    }

    #[allow(dead_code)]
    pub(crate) const fn dropped(&self) -> usize {
        self.dropped
    }
}
