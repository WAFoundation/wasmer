mod call;
mod func_env;
mod libcalls;
mod module;
mod module_env;
mod relocation;
mod resolver;
mod trampoline;

use cranelift_codegen::{
    isa,
    settings::{self, Configurable},
};
use target_lexicon::Triple;
use wasmer_runtime_core::{
    backend::{Compiler, Token},
    error::{CompileError, CompileResult},
    module::ModuleInner,
};
use wasmparser::{self, WasmDecoder};

pub struct CraneliftCompiler {}

impl CraneliftCompiler {
    pub fn new() -> Self {
        Self {}
    }
}

impl Compiler for CraneliftCompiler {
    // Compiles wasm binary to a wasmer module.
    fn compile(&self, wasm: &[u8], _: Token) -> CompileResult<ModuleInner> {
        validate(wasm)?;

        let isa = get_isa();

        let mut module = module::Module::empty();
        let module_env = module_env::ModuleEnv::new(&mut module, &*isa);

        let func_bodies = module_env.translate(wasm)?;

        module.compile(&*isa, func_bodies)
    }
}

fn get_isa() -> Box<isa::TargetIsa> {
    let flags = {
        let mut builder = settings::builder();
        builder.set("opt_level", "best").unwrap();

        if cfg!(not(test)) {
            builder.set("enable_verifier", "false").unwrap();
        }

        let flags = settings::Flags::new(builder);
        debug_assert_eq!(flags.opt_level(), settings::OptLevel::Best);
        flags
    };
    isa::lookup(Triple::host()).unwrap().finish(flags)
}

fn validate(bytes: &[u8]) -> CompileResult<()> {
    let mut parser = wasmparser::ValidatingParser::new(bytes, None);
    loop {
        let state = parser.read();
        match *state {
            wasmparser::ParserState::EndWasm => break Ok(()),
            wasmparser::ParserState::Error(err) => Err(CompileError::ValidationError {
                msg: err.message.to_string(),
            })?,
            _ => {}
        }
    }
}

/// The current version of this crate
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
