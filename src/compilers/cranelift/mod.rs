pub mod codegen;

use crate::runtime::{backend::Compiler, module::Module};

use std::sync::Arc;

use self::codegen::{converter, CraneliftModule};
use crate::webassembly;

pub struct Cranelift {}

impl Compiler for Cranelift {
    // Compiles towasm byte to module
    fn compile(&self, wasm: &[u8]) -> Result<Arc<Module>, String> {
        let isa = webassembly::get_isa();
        // Generate a Cranlift module from wasm binary
        let cranelift_module = CraneliftModule::from_bytes(wasm.to_vec(), isa.frontend_config())
            .map_err(|err| format!("{}", err))?;

        // Convert Cranelift module to wasmer module
        let wasmer_module = converter::generate_wasmer_module(cranelift_module);

        // Return new wasmer module
        Ok(Arc::new(wasmer_module))
    }
}
