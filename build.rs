fn main() {
    println!(
        "cargo:rustc-env=RUSTXNG_BUILD_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
