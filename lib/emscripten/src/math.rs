use wasmer_runtime_core::vm::Ctx;

/// emscripten: _llvm_log10_f64
pub extern "C" fn _llvm_log10_f64(value: f64, vmctx: &mut Ctx) -> f64 {
    debug!("emscripten::_llvm_log10_f64");
    value.log10()
}

/// emscripten: _llvm_log2_f64
pub extern "C" fn _llvm_log2_f64(value: f64, vmctx: &mut Ctx) -> f64 {
    debug!("emscripten::_llvm_log2_f64");
    value.log2()
}

/// emscripten: _llvm_log10_f32
pub extern "C" fn _llvm_log10_f32(value: f32, vmctx: &mut Ctx) -> f32 {
    debug!("emscripten::_llvm_log2_f32");
    value.log10()
}

/// emscripten: _llvm_log2_f32
pub extern "C" fn _llvm_log2_f32(value: f32, vmctx: &mut Ctx) -> f32 {
    debug!("emscripten::_llvm_log2_f32");
    value.log2()
}

// emscripten: f64-rem
pub extern "C" fn f64_rem(x: f64, y: f64, vmctx: &mut Ctx) -> f64 {
    debug!("emscripten::f64-rem");
    x % y
}
