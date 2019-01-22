use super::env;
use super::process::_abort;
use wasmer_runtime_core::Instance;

/// emscripten: ___cxa_allocate_exception
pub extern "C" fn ___cxa_allocate_exception(size: u32, instance: &mut Instance) -> u32 {
    debug!("emscripten::___cxa_allocate_exception");
    env::call_malloc(size as _, instance)
}

/// emscripten: ___cxa_throw
/// TODO: We don't have support for exceptions yet
pub extern "C" fn ___cxa_throw(_ptr: u32, ty: u32, destructor: u32, instance: &mut Instance) {
    debug!("emscripten::___cxa_throw");
    _abort();
}
