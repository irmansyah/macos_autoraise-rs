fn main() {
    // Tell Cargo to link against the required macOS frameworks.
    // This is equivalent to `-framework ApplicationServices -framework AppKit`
    // used in the original AutoRaise Makefile.
    println!("cargo:rustc-link-lib=framework=ApplicationServices");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=Carbon");
    println!("cargo:rustc-link-lib=framework=CoreGraphics");
}
