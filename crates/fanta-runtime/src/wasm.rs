#[cfg(not(target_os = "emscripten"))]
fn main() {}

#[cfg(target_os = "emscripten")]
mod bridge;

#[cfg(target_os = "emscripten")]
fn main() {}
