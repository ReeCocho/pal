# pal

A hardware abstraction layer for graphics.

## Build Instructions

1. Install the [Rust toolchain](https://rustup.rs/).

2. Install the [Vulkan SDK](https://vulkan.lunarg.com/).

3. Clone the repository to your system.

4. Run `cargo run --example EXAMPLE_NAME` in the root directory of the repository where
`EXAMPLE_NAME` is one of the examples located in `examples/`.

## Goals

- No manual synchronization.
- Ability to access the underlying API when needed.
- Easy to debug.
