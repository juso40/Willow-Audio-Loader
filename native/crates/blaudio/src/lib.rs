//! blaudio native module: Wwise container, soundbank and sound-engine
//! work, nothing else.
//!
//! The `blaudio` mod's Python extension (`blaudio/_blaudio.pyd`). It
//! re-exports the shared surface ([`py_audio`]) unchanged: archive tools
//! (`pck_probe`, `media_list`, `extract_media`), the add path (`craft_bank`,
//! `to_wem`, `load_bank`, ...) and the raw engine entry shims
//! (`post_event`, `register_game_object`, ...).


use pyo3::prelude::*;

/// Gathers stub metadata for `cargo run --bin stub_gen` (prints to stdout).
/// It reads no config file. The module name is fixed.
pub fn stub_info() -> pyo3_stub_gen::Result<pyo3_stub_gen::StubInfo> {
    pyo3_stub_gen::StubInfo::from_project_root(
        "blaudio._blaudio".to_owned(),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        false,
        Default::default(),
    )
}

#[pymodule]
fn _blaudio(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(py_audio::craft_bank, module)?)?;
    module.add_function(wrap_pyfunction!(py_audio::to_wem, module)?)?;
    module.add_function(wrap_pyfunction!(py_audio::media_list, module)?)?;
    module.add_function(wrap_pyfunction!(py_audio::extract_media, module)?)?;
    module.add_function(wrap_pyfunction!(py_audio::post_event, module)?)?;
    module.add_function(wrap_pyfunction!(py_audio::register_game_object, module)?)?;
    module.add_function(wrap_pyfunction!(py_audio::unregister_game_object, module)?)?;
    module.add_function(wrap_pyfunction!(py_audio::set_active_listeners, module)?)?;
    module.add_function(wrap_pyfunction!(py_audio::get_id_from_string, module)?)?;
    module.add_function(wrap_pyfunction!(py_audio::load_bank, module)?)?;
    module.add_function(wrap_pyfunction!(py_audio::unload_bank, module)?)?;
    module.add_function(wrap_pyfunction!(py_audio::pck_probe, module)?)?;
    Ok(())
}
