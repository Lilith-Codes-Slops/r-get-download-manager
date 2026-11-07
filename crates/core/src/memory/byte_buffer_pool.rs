use bytes::{BytesMut, BufMut};
use std::collections::VecDeque;
use std::sync::Mutex;

/// Reusable BytesMut slab to reduce heap fragmentation under high concurrency.
pub struct ByteBufferPool {
    free: Mutex<VecDeque<BytesMut>>,
    capacity: usize,
    max_pooled: usize,
}

impl ByteBufferPool {
    pub fn new(capacity: usize, max_pooled: usize) -> Self {
        Self {
            free: Mutex::new(VecDeque::with_capacity(max_pooled)),
            capacity,
            max_pooled,
        }
    }

    /// Obtain a buffer of at least `self.capacity` bytes. Reuses a pooled one when available.
    pub fn acquire(&self) -> BytesMut {
        let mut free = self.free.lock().unwrap();
        if let Some(mut buf) = free.pop_front() {
            buf.clear();
            if buf.capacity() < self.capacity {
                buf.reserve(self.capacity - buf.capacity());
            }
            buf
        } else {
            BytesMut::with_capacity(self.capacity)
        }
    }

    /// Return a buffer to the pool (or drop it if the pool is full).
    pub fn release(&self, mut buf: BytesMut) {
        buf.clear();
        let mut free = self.free.lock().unwrap();
        if free.len() < self.max_pooled {
            free.push_back(buf);
        }
    }

    pub fn pooled_count(&self) -> usize {
        self.free.lock().unwrap().len()
    }
}

impl Default for ByteBufferPool {
    fn default() -> Self {
        Self::new(64 * 1024, 32)
    }
}
