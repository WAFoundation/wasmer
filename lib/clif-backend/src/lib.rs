//! The Wasmer Cranelift Backend crate is used to compile wasm binary code via parse events from the
//! Wasmer runtime common parser code into machine code.
//!

#![deny(
    dead_code,
    missing_docs,
    nonstandard_style,
    unused_imports,
    unused_mut,
    unused_variables,
    unused_unsafe,
    unreachable_patterns
)]
#![doc(html_favicon_url = "https://wasmer.io/static/icons/favicon.ico")]
#![doc(html_logo_url = "https://avatars3.githubusercontent.com/u/44205449?s=200&v=4")]

mod cache;
mod code;
mod libcalls;
mod module;
mod relocation;
mod resolver;
mod signal;
mod trampoline;

use cranelift_codegen::{
    isa,
    settings::{self, Configurable},
};
use target_lexicon::Triple;
use wasmer_runtime_core::{backend::CompilerConfig, codegen::SimpleStreamingCompilerGen};

#[macro_use]
extern crate serde_derive;

extern crate rayon;
extern crate serde;

fn get_isa(config: Option<&CompilerConfig>) -> Box<dyn isa::TargetIsa> {
    let flags = {
        let mut builder = settings::builder();
        builder.set("opt_level", "speed_and_size").unwrap();
        builder.set("enable_jump_tables", "false").unwrap();

        let enable_verifier: bool;

        if let Some(config) = config {
            if config.nan_canonicalization {
                builder.set("enable_nan_canonicalization", "true").unwrap();
            }
            enable_verifier = config.enable_verification;
        } else {
            // Set defaults if no config found.
            // NOTE: cfg(test) probably does nothing when not running `cargo test`
            //       on this crate
            enable_verifier = cfg!(test) || cfg!(debug_assertions);
        }

        builder
            .set(
                "enable_verifier",
                if enable_verifier { "true" } else { "false" },
            )
            .unwrap();

        let flags = settings::Flags::new(builder);
        debug_assert_eq!(flags.opt_level(), settings::OptLevel::SpeedAndSize);
        flags
    };
    isa::lookup(Triple::host()).unwrap().finish(flags)
}

/// The current version of this crate
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Streaming compiler implementation for the Cranelift backed. Compiles web assembly binary into
/// machine code.
pub type CraneliftCompiler = SimpleStreamingCompilerGen<
    code::CraneliftModuleCodeGenerator,
    code::CraneliftFunctionCodeGenerator,
    signal::Caller,
    code::CodegenError,
>;
