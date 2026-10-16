#![deny(unsafe_op_in_unsafe_fn)]
const HEAP_BYTES: usize = 48 * 1024;
include!("fixed_heap.rs");
