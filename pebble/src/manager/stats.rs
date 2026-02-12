//! Statistics and theoretical validation.

/// Checkpoint management statistics.
#[derive(Debug, Clone)]
#[must_use]
pub struct PebbleStats {
    pub checkpoints_added: u64,
    pub red_pebble_count: usize,
    pub blue_pebble_count: usize,
    pub warm_count: usize,
    pub write_buffer_count: usize,
    pub io_operations: u64,
    pub hot_utilization: f64,
    pub theoretical_min_io: u64,
    pub io_optimality_ratio: f64,
    pub space_complexity_ratio: f64,
}

/// Red-Blue Pebble Game theoretical bounds validation.
///
/// Space: O(sqrt(T)). I/O: 2-approximation for trees (Gleinig & Hoefler 2022).
#[derive(Debug, Clone)]
#[must_use]
pub struct TheoreticalValidation {
    space_bound_satisfied: bool,
    io_bound_satisfied: bool,
    current_space_ratio: f64,
    current_io_ratio: f64,
    expected_max_space: usize,
    total_nodes: usize,
}

impl TheoreticalValidation {
    pub(crate) fn new(
        space_bound_satisfied: bool,
        io_bound_satisfied: bool,
        current_space_ratio: f64,
        current_io_ratio: f64,
        expected_max_space: usize,
        total_nodes: usize,
    ) -> Self {
        Self {
            space_bound_satisfied,
            io_bound_satisfied,
            current_space_ratio,
            current_io_ratio,
            expected_max_space,
            total_nodes,
        }
    }

    #[inline]
    pub fn space_bound_satisfied(&self) -> bool {
        self.space_bound_satisfied
    }

    #[inline]
    pub fn io_bound_satisfied(&self) -> bool {
        self.io_bound_satisfied
    }

    #[inline]
    pub fn current_space_ratio(&self) -> f64 {
        self.current_space_ratio
    }

    #[inline]
    pub fn current_io_ratio(&self) -> f64 {
        self.current_io_ratio
    }

    #[inline]
    pub fn expected_max_space(&self) -> usize {
        self.expected_max_space
    }

    #[inline]
    pub fn total_nodes(&self) -> usize {
        self.total_nodes
    }

    #[inline]
    pub fn all_bounds_satisfied(&self) -> bool {
        self.space_bound_satisfied && self.io_bound_satisfied
    }
}
