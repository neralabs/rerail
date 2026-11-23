//! Where a thread's run may place strata (the CPU only, or the device too): the run's binding, which the planner's
//! placement (`plan::placement`) prices within, and the executor (`lanes::executor`) and the rows' blocks
//! (`block_device`) follow.

/// Where a thread's run may place strata: the CPU only, the device where it is the cheaper (the Metal binding), or
/// the device wherever it has every form (`Always`: what the device can run, apart from what it should; the
/// executor coverage gate's, and `RERAIL_DEVICE=always`'s).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    Cpu,
    Metal,
    Always,
}

/// The run's binding (`typing::file_local::Binding::Metal` is `Metal`), in the run's context: its jobs' too.
#[derive(Clone, Copy)]
struct Device(Binding);
impl Default for Device {
    fn default() -> Self {
        Device(Binding::Cpu)
    }
}

/// Whether this thread's run may use the device.
pub fn device() -> bool {
    binding() != Binding::Cpu
}

/// Whether this run may place strata on the device, until the guard drops. `RERAIL_DEVICE=always` makes a Metal
/// binding `Always`, and so does a caller's `Always` (a test's, `set_binding`).
pub fn set_device(on: bool) -> rerail_prof::code_rule_dsl::logic::profile::Bound {
    let b = match on {
        false => Binding::Cpu,
        true if always_env() || binding() == Binding::Always => Binding::Always,
        true => Binding::Metal,
    };
    set_binding(b)
}

/// What a block this run makes for the device to read asks of the block policy (decision 0342): the device may
/// read it when the run is bound to the device; a CPU run's never.
pub fn block_device() -> rerail_blocks::alloc::Device {
    if device() { rerail_blocks::alloc::Device::May } else { rerail_blocks::alloc::Device::No }
}

/// The run's binding.
pub fn binding() -> Binding {
    rerail_prof::code_rule_dsl::logic::profile::bound::<Device>().0
}

/// The run's binding, until the guard drops.
pub fn set_binding(b: Binding) -> rerail_prof::code_rule_dsl::logic::profile::Bound {
    rerail_prof::code_rule_dsl::logic::profile::bind(Device(b))
}

fn always_env() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_DEVICE").is_ok_and(|v| v == "always"))
}
