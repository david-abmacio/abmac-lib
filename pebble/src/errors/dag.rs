//! DAG-related error types.

verdict::display_error! {
    #[derive(Clone, PartialEq, Eq)]
    pub enum DAGError {
        #[display("missing dependency: {dep_id}")]
        MissingDependency { dep_id: alloc::string::String },

        #[display("self-dependency: {node_id}")]
        SelfDependency { node_id: alloc::string::String },

        #[display("node exists: {node_id}")]
        NodeExists { node_id: alloc::string::String },

        #[display("cycle detected: {node_id}")]
        CycleDetected { node_id: alloc::string::String },
    }
}
