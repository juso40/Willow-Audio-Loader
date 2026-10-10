//! Generates `blaudio/_blaudio.pyi` from the pyo3-stub-gen inventory.
//!
//! Gets invoked by `scripts/build_windows.sh` automatically.
//!
//! The stub describes the `blaudio._blaudio` module. It renders the
//! inventory gathered by [`_blaudio::stub_info`] directly to the explicit
//! output path.

use _blaudio::stub_info;

/// Dotted module name the stub is generated for.
const MODULE: &str = "blaudio._blaudio";

fn main() {
    let stub = stub_info().expect("failed to gather stub info");
    let module = stub
        .modules
        .get(MODULE)
        .unwrap_or_else(|| panic!("{MODULE} missing from stub info"));
    let text = module.to_string();
    // `<root>/native/crates/blaudio` -> `<root>/blaudio/_blaudio.pyi`.
    let out = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../blaudio/_blaudio.pyi");
    std::fs::write(&out, &text).expect("failed to write _blaudio.pyi");
    println!("wrote {} ({} bytes)", out.display(), text.len());
}
