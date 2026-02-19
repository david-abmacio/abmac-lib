//! Branch-related error types.

use alloc::string::String;

use crate::manager::BranchId;

verdict::display_error! {
    /// Error type for branching operations.
    #[derive(Clone, PartialEq, Eq)]
    pub enum BranchError {
        #[display("branching not enabled")]
        BranchingNotEnabled,

        #[display("branch {id:?} not found")]
        BranchNotFound { id: BranchId },

        #[display("branch name already used: {name}")]
        NameAlreadyUsed { name: String },
    }
}
