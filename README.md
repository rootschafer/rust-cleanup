# rust-cleanup

A simple tool to save space on your computer by cleaning up build files for rust projects. Supports regular Rust projects and Dioxus projects. 


# Install

I choose to not publish this crate on crates.io because it's not properly documented yet.

1. Clone the github repo somewhere

2. Install with:

```rust
cargo install --path .
```

# Usage:

Output of `rust-cleanup --help`:

``` 
Usage: rust-cleanup [OPTIONS]

Options:
  -p, --path <PATH>  Sets the starting directory for the search
      --yes-cargo    Automatically clean non-Dioxus Rust projects without prompting
      --yes-dioxus   Automatically clean Dioxus projects without prompting
  -y, --yes-all      Automatically clean all projects without prompting for a yes or a no
  -h, --help         Print help
```
