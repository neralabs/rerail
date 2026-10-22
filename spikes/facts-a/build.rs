fn main() {
    // rerail-engine is a napi addon; its napi_* imports resolve only inside node. The spike binary
    // never calls them, so leave them to dynamic lookup (macOS).
    println!("cargo:rustc-link-arg-bins=-undefined");
    println!("cargo:rustc-link-arg-bins=dynamic_lookup");
}
