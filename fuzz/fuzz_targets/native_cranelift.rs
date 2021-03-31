#![no_main]

use libfuzzer_sys::{arbitrary, arbitrary::Arbitrary, fuzz_target};
use wasm_smith::{Config, ConfiguredModule};
use wasmer::{imports, Instance, Module, Store};
use wasmer_compiler_cranelift::Cranelift;
use wasmer_engine_native::Native;

#[derive(Arbitrary, Debug, Default, Copy, Clone)]
struct NoImportsConfig;
impl Config for NoImportsConfig {
    fn max_imports(&self) -> usize {
        0
    }
    fn max_memory_pages(&self) -> u32 {
        // https://github.com/wasmerio/wasmer/issues/2187
        65535
    }
    fn allow_start_export(&self) -> bool {
        false
    }
}

fuzz_target!(|module: ConfiguredModule<NoImportsConfig>| {
    let serialized = {
        let wasm_bytes = module.to_bytes();
        let compiler = Cranelift::default();
        let store = Store::new(&Native::new(compiler).engine());
        let module = Module::new(&store, &wasm_bytes).unwrap();
        module.serialize().unwrap()
    };

    let engine = Native::headless().engine();
    let store = Store::new(&engine);
    let module = unsafe { Module::deserialize(&store, serialized.as_slice()) }.unwrap();
    match Instance::new(&module, &imports! {}) {
        Ok(_) => {}
        Err(e) => {
            let error_message = format!("{}", e);
            if error_message
                .contains("RuntimeError: memory out of bounds: data segment does not fit")
                || error_message
                    .contains("RuntimeError: table out of bounds: elements segment does not fit")
            {
                return;
            }
            panic!("{}", e);
        }
    }
});
