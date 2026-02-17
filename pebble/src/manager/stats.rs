//! Statistics and theoretical validation.

/// Checkpoint management statistics.
#[derive(Debug, Clone)]
#[must_use]
pub struct PebbleStats {
    checkpoints_added: u64,
    red_pebble_count: usize,
    blue_pebble_count: usize,
    warm_count: usize,
    write_buffer_count: usize,
    io_operations: u64,
    warm_hits: u64,
    cold_loads: u64,
    hot_utilization: f64,
    theoretical_min_io: u64,
    io_optimality_ratio: f64,
    space_complexity_ratio: f64,
}

impl PebbleStats {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        checkpoints_added: u64,
        red_pebble_count: usize,
        blue_pebble_count: usize,
        warm_count: usize,
        write_buffer_count: usize,
        io_operations: u64,
        warm_hits: u64,
        cold_loads: u64,
        hot_utilization: f64,
        theoretical_min_io: u64,
        io_optimality_ratio: f64,
        space_complexity_ratio: f64,
    ) -> Self {
        Self {
            checkpoints_added,
            red_pebble_count,
            blue_pebble_count,
            warm_count,
            write_buffer_count,
            io_operations,
            warm_hits,
            cold_loads,
            hot_utilization,
            theoretical_min_io,
            io_optimality_ratio,
            space_complexity_ratio,
        }
    }

    #[inline]
    pub fn checkpoints_added(&self) -> u64 {
        self.checkpoints_added
    }

    #[inline]
    pub fn red_pebble_count(&self) -> usize {
        self.red_pebble_count
    }

    #[inline]
    pub fn blue_pebble_count(&self) -> usize {
        self.blue_pebble_count
    }

    #[inline]
    pub fn warm_count(&self) -> usize {
        self.warm_count
    }

    #[inline]
    pub fn write_buffer_count(&self) -> usize {
        self.write_buffer_count
    }

    #[inline]
    pub fn io_operations(&self) -> u64 {
        self.io_operations
    }

    #[inline]
    pub fn warm_hits(&self) -> u64 {
        self.warm_hits
    }

    #[inline]
    pub fn cold_loads(&self) -> u64 {
        self.cold_loads
    }

    #[inline]
    pub fn hot_utilization(&self) -> f64 {
        self.hot_utilization
    }

    #[inline]
    pub fn theoretical_min_io(&self) -> u64 {
        self.theoretical_min_io
    }

    #[inline]
    pub fn io_optimality_ratio(&self) -> f64 {
        self.io_optimality_ratio
    }

    #[inline]
    pub fn space_complexity_ratio(&self) -> f64 {
        self.space_complexity_ratio
    }
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
