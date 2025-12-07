use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let output_file = PathBuf::from(&crate_dir).join("include").join("boxlite.h");

    // Create include directory if it doesn't exist
    std::fs::create_dir_all(output_file.parent().unwrap())
        .expect("Failed to create include directory");

    // Generate C header from Rust code
    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_language(cbindgen::Language::C)
        .with_pragma_once(true)
        .with_include_guard("BOXLITE_H")
        .with_documentation(true)
        .with_cpp_compat(true)
        .generate()
        .expect("Unable to generate C bindings")
        .write_to_file(&output_file);

    println!("cargo:rerun-if-changed=src/");
}
