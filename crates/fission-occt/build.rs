// Builds src/shim.cpp: our own OCCT calls (shape healing, validity checks,
// exception-safe booleans) against the OCCT that opencascade-sys bundles.
fn main() {
    let occt = occt_sys::occt_path();
    cxx_build::bridge("src/shim.rs")
        .file("src/shim.cpp")
        .include(occt.join("include"))
        .std("c++17")
        .flag_if_supported("-w")
        .compile("fission_occt_shim");
    println!("cargo:rerun-if-changed=src/shim.rs");
    println!("cargo:rerun-if-changed=src/shim.cpp");
    println!("cargo:rerun-if-changed=src/shim.hxx");
}
