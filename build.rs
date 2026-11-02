fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rustc-env=RUSTXNG_BUILD_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
