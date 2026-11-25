//! The cost statistics and the learned models (decision 0100 §14, `code_rule_dsl::logic::costs`): every job the
//! executor runs kept as a sample across runs, the prices' rates fitted from them, and the models trained by upkeep,
//! saved as kept facts and put in place. The lanes start it (rerail-run); the planner and the executor read its
//! prices. It stands on the executor's prices and models (rerail-accel), the store's interface and prof alone, below
//! the logic language's core.
//!
//! Its sources are in its own `src/`, at their module path in the engine's tree (the crate split, 10e).
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
pub use rerail_observe::content_store;
extern crate self as rerail_costs;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    #[path = "logic"]
    pub mod logic {
        pub use rerail_prof::code_rule_dsl::logic::profile;
        pub mod costs;
    }
}

/// This crate's own test binary, which the engine does not load: its learned models registered as it loads, as boot
/// registers them (rerail-accel-gpu), so the cost model's tests train on the device.
#[cfg(test)]
mod test_load {
    extern "C" fn load() {
        rerail_accel_gpu::install();
        // The learned model's file (`costs::models`) is the store's kept facts, as boot registers it.
        rerail_store::content_store::observer_store::register();
    }
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}
