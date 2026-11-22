//! Registries of an abstraction's providers.
//!
//! A central crate declares an abstraction (a trait, or a function type) and, beside it, a registry. A provider
//! crate implements the abstraction and registers itself from its `register()`. The composition root
//! (`rerail-boot`) calls every provider's `register()` in one fixed order as the program loads, then seals the
//! registries. The core asks the registry and never names a provider. This is MLDB's pattern
//! (`ComputeRuntime::registerRuntime`, `forest/src/mldb/block/compute_kernel.cc`), with an explicit call in place
//! of a static constructor: a Rust provider that nothing names can be dropped at link time, and constructors'
//! order across crates is unspecified.
//!
//! A registry holds code (references to `'static` providers, function pointers), registered at load. Once sealed it
//! never changes, so it is not run state: two runs at once read the same providers and cannot reach each other
//! through it (the engine's no-global-state rule allows code registered at load).
//!
//! - A second registration of a key is refused, naming both providers. Boot registers each provider once
//!   (`rerail_boot::install` runs once a process); a test binary's load hook calls it rather than registering
//!   providers of its own.
//! - Registration after `seal` is refused.
//! - Asking for a key nobody registered is an error that names the registry and lists what it holds: never a
//!   silent default.
//! - `all` lists the providers in registration order, which is meaningful where order is (parse order, check
//!   order).

use std::fmt;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// One registered provider.
#[derive(Clone, Copy)]
pub struct Entry<K, V> {
    pub key: K,
    /// Who registered it, for messages (a crate or module name).
    pub provider: &'static str,
    pub value: V,
}

/// The providers of one abstraction, by key.
pub struct Registry<K: 'static, V: 'static> {
    what: &'static str,
    entries: RwLock<Vec<Entry<K, V>>>,
    sealed: AtomicBool,
}

/// A key the registry has no provider for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub registry: &'static str,
    pub key: String,
    pub registered: Vec<String>,
}

impl fmt::Display for Missing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.registered.is_empty() {
            write!(f, "{}: nothing is registered for {} (nothing is registered at all: was the program booted?)", self.registry, self.key)
        } else {
            write!(f, "{}: nothing is registered for {}; registered: {}", self.registry, self.key, self.registered.join(", "))
        }
    }
}

impl std::error::Error for Missing {}

impl<K: Copy + Eq + fmt::Debug + Send + Sync, V: Copy + Send + Sync> Registry<K, V> {
    /// An empty registry; `what` names it in messages.
    pub const fn new(what: &'static str) -> Self {
        Self { what, entries: RwLock::new(Vec::new()), sealed: AtomicBool::new(false) }
    }

    /// The registry's name.
    pub fn what(&self) -> &'static str {
        self.what
    }

    /// Registers `value` as the provider of `key`.
    ///
    /// # Panics
    /// When the registry is sealed, or when `key` already has another provider: both are bugs in the program's
    /// composition, not conditions a run can meet.
    pub fn register(&self, key: K, provider: &'static str, value: V) {
        let mut entries = self.entries.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(held) = entries.iter().find(|entry| entry.key == key) {
            panic!("{}: {key:?} is registered twice, by {} and by {provider}", self.what, held.provider);
        }
        if self.sealed.load(Ordering::Acquire) {
            panic!("{}: {provider} registers {key:?} after the registry was sealed", self.what);
        }
        entries.push(Entry { key, provider, value });
    }

    /// Refuses any later registration.
    pub fn seal(&self) {
        self.sealed.store(true, Ordering::Release);
    }

    /// Whether the registry is sealed.
    pub fn is_sealed(&self) -> bool {
        self.sealed.load(Ordering::Acquire)
    }

    /// The provider of `key`.
    pub fn get(&self, key: K) -> Result<V, Missing> {
        self.try_get(key).ok_or_else(|| Missing { registry: self.what, key: format!("{key:?}"), registered: self.keys() })
    }

    /// The provider of `key`, if one is registered.
    pub fn try_get(&self, key: K) -> Option<V> {
        self.entries.read().unwrap_or_else(|poisoned| poisoned.into_inner()).iter().find(|entry| entry.key == key).map(|entry| entry.value)
    }

    /// Every provider, in registration order.
    pub fn all(&self) -> Vec<Entry<K, V>> {
        self.entries.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// Every provider's value, in registration order.
    pub fn values(&self) -> Vec<V> {
        self.entries.read().unwrap_or_else(|poisoned| poisoned.into_inner()).iter().map(|entry| entry.value).collect()
    }

    /// The registered keys, as messages print them.
    pub fn keys(&self) -> Vec<String> {
        self.entries.read().unwrap_or_else(|poisoned| poisoned.into_inner()).iter().map(|entry| format!("{:?} ({})", entry.key, entry.provider)).collect()
    }

    /// What the registry holds, one provider per line.
    pub fn describe(&self) -> String {
        let mut out = format!("{}{}:\n", self.what, if self.is_sealed() { " (sealed)" } else { "" });
        for key in self.keys() {
            out.push_str("  ");
            out.push_str(&key);
            out.push('\n');
        }
        out
    }
}

/// The one provider of an abstraction that has exactly one (a formatter, a reader of one kind of file).
pub struct Slot<V: 'static> {
    inner: Registry<(), V>,
}

impl<V: Copy + Send + Sync> Slot<V> {
    pub const fn new(what: &'static str) -> Self {
        Self { inner: Registry::new(what) }
    }

    /// Registers the provider. Panics as `Registry::register` does.
    pub fn register(&self, provider: &'static str, value: V) {
        self.inner.register((), provider, value);
    }

    pub fn seal(&self) {
        self.inner.seal();
    }

    /// The provider.
    pub fn get(&self) -> Result<V, Missing> {
        self.inner.try_get(()).ok_or_else(|| Missing { registry: self.inner.what, key: "its provider".to_owned(), registered: Vec::new() })
    }

    /// The provider, if one is registered.
    pub fn try_get(&self) -> Option<V> {
        self.inner.try_get(())
    }

    /// Who registered the provider.
    pub fn provider(&self) -> Option<&'static str> {
        self.inner.all().first().map(|entry| entry.provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    trait Device: Sync {
        fn name(&self) -> &'static str;
    }
    struct Cpu;
    struct Gpu;
    impl Device for Cpu {
        fn name(&self) -> &'static str {
            "cpu"
        }
    }
    impl Device for Gpu {
        fn name(&self) -> &'static str {
            "gpu"
        }
    }
    static CPU: Cpu = Cpu;
    static GPU: Gpu = Gpu;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Kind {
        Cpu,
        Gpu,
    }

    #[test]
    fn a_registered_provider_is_found_by_its_key_and_listed_in_order() {
        let devices: Registry<Kind, &'static dyn Device> = Registry::new("devices");
        devices.register(Kind::Gpu, "gpu crate", &GPU);
        devices.register(Kind::Cpu, "cpu crate", &CPU);
        assert_eq!(devices.get(Kind::Cpu).unwrap().name(), "cpu");
        let order: Vec<_> = devices.values().iter().map(|device| device.name()).collect();
        assert_eq!(order, ["gpu", "cpu"]);
    }

    #[test]
    fn a_missing_key_names_the_registry_and_what_it_holds() {
        let devices: Registry<Kind, &'static dyn Device> = Registry::new("devices");
        devices.register(Kind::Cpu, "cpu crate", &CPU);
        let missing = devices.get(Kind::Gpu).err().unwrap();
        assert_eq!(missing.to_string(), "devices: nothing is registered for Gpu; registered: Cpu (cpu crate)");
        let empty: Registry<Kind, &'static dyn Device> = Registry::new("devices");
        assert!(empty.get(Kind::Cpu).err().unwrap().to_string().contains("was the program booted?"));
    }

    #[test]
    #[should_panic(expected = "devices: Cpu is registered twice, by cpu crate and by gpu crate")]
    fn another_provider_for_a_key_is_refused() {
        let devices: Registry<Kind, &'static dyn Device> = Registry::new("devices");
        devices.register(Kind::Cpu, "cpu crate", &CPU);
        devices.register(Kind::Cpu, "gpu crate", &GPU);
    }

    #[test]
    #[should_panic(expected = "after the registry was sealed")]
    fn a_registration_after_the_seal_is_refused() {
        let devices: Registry<Kind, &'static dyn Device> = Registry::new("devices");
        devices.register(Kind::Cpu, "cpu crate", &CPU);
        devices.seal();
        devices.register(Kind::Gpu, "gpu crate", &GPU);
    }

    #[test]
    fn a_slot_holds_one_function() {
        static FORMAT: Slot<fn(f64) -> String> = Slot::new("number formatter");
        assert!(FORMAT.get().err().unwrap().to_string().starts_with("number formatter: nothing is registered"));
        FORMAT.register("ecma", |x| format!("{x:.1}"));
        assert_eq!((FORMAT.get().unwrap())(1.25), "1.2");
        assert_eq!(FORMAT.provider(), Some("ecma"));
    }
}
