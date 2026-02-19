//! Pebble game error types.

use alloc::string::String;

verdict::display_error! {
    #[derive(Clone, PartialEq, Eq)]
    pub enum PebbleError {
        #[display("fast memory exhausted: {current}/{max_size}")]
        FastMemoryExhausted { current: usize, max_size: usize },

        #[display("invalid operation: {operation}")]
        InvalidOperation { operation: String },

        #[display("node not found: {node}")]
        NotFound { node: String },
    }
}
