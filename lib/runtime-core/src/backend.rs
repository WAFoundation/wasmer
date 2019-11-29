use crate::{
    error::CompileResult,
    module::ModuleInner,
    state::ModuleStateMap,
    typed_func::Wasm,
    types::{LocalFuncIndex, SigIndex},
    vm,
};

use crate::{
    cache::{Artifact, Error as CacheError},
    codegen::BreakpointMap,
    module::ModuleInfo,
    sys::Memory,
};
use std::{any::Any, ptr::NonNull};

use std::collections::HashMap;

pub mod sys {
    pub use crate::sys::*;
}
pub use crate::sig_registry::SigRegistry;

/// Enum used to select which compiler should be used to generate code.
#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub enum Backend {
    Cranelift,
    Singlepass,
    LLVM,
    Auto,
}

impl Backend {
    /// Get a list of the currently enabled (via feature flag) backends.
    pub fn variants() -> &'static [&'static str] {
        &[
            #[cfg(feature = "backend-cranelift")]
            "cranelift",
            #[cfg(feature = "backend-singlepass")]
            "singlepass",
            #[cfg(feature = "backend-llvm")]
            "llvm",
            "auto",
        ]
    }

    /// Stable string representation of the backend.
    /// It can be used as part of a cache key, for example.
    pub fn to_string(&self) -> &'static str {
        match self {
            Backend::Cranelift => "cranelift",
            Backend::Singlepass => "singlepass",
            Backend::LLVM => "llvm",
            Backend::Auto => "auto",
        }
    }
}

impl Default for Backend {
    fn default() -> Self {
        Backend::Cranelift
    }
}

impl std::str::FromStr for Backend {
    type Err = String;
    fn from_str(s: &str) -> Result<Backend, String> {
        match s.to_lowercase().as_str() {
            "singlepass" => Ok(Backend::Singlepass),
            "cranelift" => Ok(Backend::Cranelift),
            "llvm" => Ok(Backend::LLVM),
            "auto" => Ok(Backend::Auto),
            _ => Err(format!("The backend {} doesn't exist", s)),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Architecture {
    X64,
    Aarch64,
}

#[repr(u8)]
#[derive(Copy, Clone, Debug)]
pub enum InlineBreakpointType {
    Trace,
    Middleware,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct InlineBreakpoint {
    pub size: usize,
    pub ty: InlineBreakpointType,
}

pub fn get_inline_breakpoint_size(arch: Architecture, backend: Backend) -> Option<usize> {
    match (arch, backend) {
        (Architecture::X64, Backend::Singlepass) => Some(7),
        (Architecture::Aarch64, Backend::Singlepass) => Some(12),
        _ => None,
    }
}

pub fn read_inline_breakpoint(
    arch: Architecture,
    backend: Backend,
    code: &[u8],
) -> Option<InlineBreakpoint> {
    match arch {
        Architecture::X64 => match backend {
            Backend::Singlepass => {
                if code.len() < 7 {
                    None
                } else if &code[..6] == &[0x0f, 0x0b, 0x0f, 0xb9, 0xcd, 0xff] {
                    // ud2 ud (int 0xff) code
                    Some(InlineBreakpoint {
                        size: 7,
                        ty: match code[6] {
                            0 => InlineBreakpointType::Trace,
                            1 => InlineBreakpointType::Middleware,
                            _ => InlineBreakpointType::Unknown,
                        },
                    })
                } else {
                    None
                }
            }
            _ => None,
        },
        Architecture::Aarch64 => match backend {
            Backend::Singlepass => {
                if code.len() < 12 {
                    None
                } else if &code[..8] == &[0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff] {
                    Some(InlineBreakpoint {
                        size: 12,
                        ty: match code[8] {
                            0 => InlineBreakpointType::Trace,
                            1 => InlineBreakpointType::Middleware,
                            _ => InlineBreakpointType::Unknown,
                        },
                    })
                } else {
                    None
                }
            }
            _ => None,
        },
    }
}

#[cfg(test)]
mod backend_test {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn str_repr_matches() {
        // if this test breaks, think hard about why it's breaking
        // can we avoid having these be different?

        for &backend in &[Backend::Cranelift, Backend::LLVM, Backend::Singlepass] {
            assert_eq!(backend, Backend::from_str(backend.to_string()).unwrap());
        }
    }
}

/// This type cannot be constructed from
/// outside the runtime crate.
pub struct Token {
    _private: (),
}

impl Token {
    pub(crate) fn generate() -> Self {
        Self { _private: () }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum MemoryBoundCheckMode {
    Default,
    Enable,
    Disable,
}

impl Default for MemoryBoundCheckMode {
    fn default() -> MemoryBoundCheckMode {
        MemoryBoundCheckMode::Default
    }
}

/// Controls which experimental features will be enabled.
#[derive(Debug, Default)]
pub struct Features {
    pub simd: bool,
    pub threads: bool,
}

/// Configuration data for the compiler
#[derive(Debug, Default)]
pub struct CompilerConfig {
    /// Symbol information generated from emscripten; used for more detailed debug messages
    pub symbol_map: Option<HashMap<u32, String>>,
    pub memory_bound_check_mode: MemoryBoundCheckMode,
    pub enforce_stack_check: bool,
    pub track_state: bool,
    pub features: Features,

    // target info used by LLVM
    pub triple: Option<String>,
    pub cpu_name: Option<String>,
    pub cpu_features: Option<String>,
}

pub trait Compiler {
    /// Compiles a `Module` from WebAssembly binary format.
    /// The `CompileToken` parameter ensures that this can only
    /// be called from inside the runtime.
    fn compile(
        &self,
        wasm: &[u8],
        comp_conf: CompilerConfig,
        _: Token,
    ) -> CompileResult<ModuleInner>;

    unsafe fn from_cache(&self, cache: Artifact, _: Token) -> Result<ModuleInner, CacheError>;
}

pub trait RunnableModule: Send + Sync {
    /// This returns a pointer to the function designated by the `local_func_index`
    /// parameter.
    fn get_func(
        &self,
        info: &ModuleInfo,
        local_func_index: LocalFuncIndex,
    ) -> Option<NonNull<vm::Func>>;

    fn get_module_state_map(&self) -> Option<ModuleStateMap> {
        None
    }

    fn get_breakpoints(&self) -> Option<BreakpointMap> {
        None
    }

    unsafe fn patch_local_function(&self, _idx: usize, _target_address: usize) -> bool {
        false
    }

    /// A wasm trampoline contains the necessary data to dynamically call an exported wasm function.
    /// Given a particular signature index, we are returned a trampoline that is matched with that
    /// signature and an invoke function that can call the trampoline.
    fn get_trampoline(&self, info: &ModuleInfo, sig_index: SigIndex) -> Option<Wasm>;

    unsafe fn do_early_trap(&self, data: Box<dyn Any>) -> !;

    /// Returns the machine code associated with this module.
    fn get_code(&self) -> Option<&[u8]> {
        None
    }

    /// Returns the beginning offsets of all functions, including import trampolines.
    fn get_offsets(&self) -> Option<Vec<usize>> {
        None
    }

    /// Returns the beginning offsets of all local functions.
    fn get_local_function_offsets(&self) -> Option<Vec<usize>> {
        None
    }
}

pub trait CacheGen: Send + Sync {
    fn generate_cache(&self) -> Result<(Box<[u8]>, Memory), CacheError>;
}
