//! Compiles simdjson's amalgamation with the shim and its kernels (one translation unit: the kernels use
//! simdjson's internal stage 1 and scalar checks), optimized in every profile: simdjson is a
//! dependency's code, as the `[profile.dev.package."*"]` setting has the others (a workspace
//! member is not covered by it).
use std::ffi::OsString;
use std::process::Stdio;

fn main() {
    let vendor = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/simdjson-5.0.5");
    let shim = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shim.cpp");
    let kernel = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/kernel.inc");
    for file in [vendor.join("simdjson.cpp"), vendor.join("simdjson.h"), shim.clone(), kernel] {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    let mut build = cc::Build::new();
    // MSVC's own compiler takes none of it (GCC and Clang target regions and attributes): Visual Studio's clang-cl
    // compiles it there, to objects MSVC's linker links.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") && std::env::var_os("CXX").is_none() {
        build.compiler(clang_cl());
    }
    build
        .cpp(true)
        .std("c++17")
        .opt_level(3)
        .debug(false)
        .define("NDEBUG", None)
        // The shim reads errors as codes; nothing throws.
        .define("SIMDJSON_EXCEPTIONS", "0")
        .include(&vendor)
        .file(&shim)
        .warnings(false);
    println!("cargo:rerun-if-env-changed=TARGET_SYSROOT");
    if let Some(sysroot) = linux_gnu_sysroot() {
        for_sysroot(&mut build, sysroot);
    }
    build.compile("rerail_simdjson");
}

/// `TARGET_SYSROOT`, when it is set (and not empty) and the target is Linux with glibc: the one target `for_sysroot` is for.
fn linux_gnu_sysroot() -> Option<OsString> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") || std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
        return None;
    }
    std::env::var_os("TARGET_SYSROOT").filter(|sysroot| !sysroot.is_empty())
}

/// The released Linux engine (`napi build --use-napi-cross`, package.yml) links against the sysroot of napi-rs's cross
/// toolchain, `TARGET_SYSROOT`: glibc 2.17 and GCC 4.8.5's C++ runtime, libstdc++ 6.0.19, which the engine then loads
/// on any distribution with those. That GCC has no C++17, so the compiler `cc` picks (the runner's, GCC 13) compiles
/// simdjson, and every symbol it leaves for the link must be one those two libraries define:
/// - glibc's headers are the sysroot's, not the compiler's machine's: newer headers inline references to newer glibc
///   (`__libc_single_threaded` 2.32 in libstdc++'s reference counts, `__isoc23_strtol` 2.38). `--sysroot` also hides
///   the compiler's own C++ headers, which come back first on the include path, as they are without it;
/// - libstdc++'s old string ABI and no sized deallocation: `std::__cxx11::basic_string` and the sized `operator delete`
///   are libstdc++ 6.0.21's (GLIBCXX_3.4.21);
/// - no `<iostream>`: GCC 13's references `std::ios_base_library_init` (GLIBCXX_3.4.32) to start the standard streams.
///   simdjson includes it and uses none of it (`std::ostream` is `<ostream>`'s), so its guard is defined in advance.
fn for_sysroot(build: &mut cc::Build, sysroot: OsString) {
    for dir in cxx_include_dirs(build) {
        build.flag("-isystem").flag(dir);
    }
    let mut flag = OsString::from("--sysroot=");
    flag.push(sysroot);
    build
        .flag(flag)
        .define("_GLIBCXX_USE_CXX11_ABI", "0")
        .flag("-fno-sized-deallocation")
        .define("_GLIBCXX_IOSTREAM", None);
}

/// The compiler's C++ standard library headers: the directories of its `#include <...>` search that hold `c++`, in
/// order (`/usr/include/c++/13`, its target's `c++/13`, `backward`).
fn cxx_include_dirs(build: &cc::Build) -> Vec<String> {
    let mut command = build.get_compiler().to_command();
    command.args(["-E", "-x", "c++", "-v", "-"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let output = command.output().unwrap_or_else(|error| panic!("running the C++ compiler for its include directories: {error}"));
    let text = String::from_utf8_lossy(&output.stderr);
    let dirs: Vec<String> = text
        .lines()
        .skip_while(|line| !line.starts_with("#include <...> search starts here:"))
        .skip(1)
        .take_while(|line| !line.starts_with("End of search list."))
        .map(str::trim)
        .filter(|dir| dir.split('/').any(|part| part == "c++"))
        .map(str::to_owned)
        .collect();
    assert!(!dirs.is_empty(), "the C++ compiler lists no C++ include directory:\n{text}");
    dirs
}

/// Visual Studio's clang-cl (its "C++ Clang tools for Windows" component), as its installer's `vswhere` finds it, for
/// the machine this build runs on (it compiles for the target it is told).
fn clang_cl() -> std::path::PathBuf {
    let host = if std::env::var("HOST").is_ok_and(|h| h.starts_with("aarch64")) { "ARM64" } else { "x64" };
    let find = format!("VC\\Tools\\Llvm\\{host}\\bin\\clang-cl.exe");
    let found = std::env::var("ProgramFiles(x86)").ok().and_then(|programs| {
        let vswhere = format!("{programs}\\Microsoft Visual Studio\\Installer\\vswhere.exe");
        let o = std::process::Command::new(vswhere).args(["-latest", "-products", "*", "-find", &find]).output().ok()?;
        String::from_utf8_lossy(&o.stdout).lines().next().map(str::trim).filter(|l| !l.is_empty()).map(std::path::PathBuf::from)
    });
    found.unwrap_or_else(|| {
        panic!("simdjson needs clang on MSVC: install Visual Studio's \"C++ Clang tools for Windows\" component, or set CXX to a clang-cl")
    })
}
