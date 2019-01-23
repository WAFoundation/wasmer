#[cfg(test)]
#[macro_use]
extern crate field_offset;

#[macro_use]
mod macros;
#[doc(hidden)]
pub mod backend;
mod backing;
pub mod error;
pub mod export;
pub mod import;
pub mod instance;
pub mod memory;
pub mod module;
mod sig_registry;
pub mod structures;
mod sys;
pub mod table;
pub mod types;
pub mod vm;
#[doc(hidden)]
pub mod vmcalls;

use self::error::CompileResult;
#[doc(inline)]
pub use self::error::Result;
#[doc(inline)]
pub use self::instance::Instance;
#[doc(inline)]
pub use self::module::Module;
use std::rc::Rc;

pub mod prelude {
    pub use crate::import::{ImportObject, Namespace};
    pub use crate::types::{
        FuncIndex, GlobalIndex, ImportedFuncIndex, ImportedGlobalIndex, ImportedMemoryIndex,
        ImportedTableIndex, LocalFuncIndex, LocalGlobalIndex, LocalMemoryIndex, LocalTableIndex,
        MemoryIndex, TableIndex, Type, Value,
    };
    pub use crate::vm;
    pub use crate::{export_func, imports};
}

/// Compile a webassembly module using the provided compiler.
pub fn compile_with(
    wasm: &[u8],
    compiler: &dyn backend::Compiler,
) -> CompileResult<module::Module> {
    let token = backend::Token::generate();
    compiler
        .compile(wasm, token)
        .map(|inner| module::Module::new(Rc::new(inner)))
}

/// This function performs validation as defined by the
/// WebAssembly specification and returns true if validation
/// succeeded, false if validation failed.
pub fn validate(wasm: &[u8]) -> bool {
    use wasmparser::WasmDecoder;
    let mut parser = wasmparser::ValidatingParser::new(wasm, None);
    loop {
        let state = parser.read();
        match *state {
            wasmparser::ParserState::EndWasm => break true,
            wasmparser::ParserState::Error(_) => break false,
            _ => {}
        }
    }
}

/// The current version of this crate
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
