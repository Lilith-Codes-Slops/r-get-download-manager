use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Eq, PartialEq)]
struct Item {
    priority: i32,
    created: i64,
    id: String,
}

impl Ord for Item {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .priority
            .cmp(&self.priority)
            .then_with(|| self.created.cmp(&other.created))
    }
}

impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub struct PriorityHeap {
    heap: BinaryHeap<Item>,
}

impl PriorityHeap {
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
        }
    }

    pub fn push(&mut self, id: String, priority: i32, created: i64) {
        self.heap.push(Item {
            priority,
            created,
            id,
        });
    }

    pub fn pop(&mut self) -> Option<String> {
        self.heap.pop().map(|i| i.id)
    }
}

impl Default for PriorityHeap {
    fn default() -> Self {
        Self::new()
    }
}
