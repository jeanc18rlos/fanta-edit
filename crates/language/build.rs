fn main() {
    if let Ok(bundled) = std::env::var("FANTA_BUNDLE") {
        println!("cargo:rustc-env=FANTA_BUNDLE={}", bundled);
    }
}
