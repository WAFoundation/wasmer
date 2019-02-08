#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use self::unix::*;

#[cfg(windows)]
pub use self::windows::*;

use crate::{allocate_on_stack, EmscriptenData};
use std::os::raw::c_int;
use wasmer_runtime_core::vm::Ctx;

pub fn _getaddrinfo(_one: i32, _two: i32, _three: i32, _four: i32, _ctx: &mut Ctx) -> i32 {
    debug!("emscripten::_getaddrinfo");
    -1
}

pub fn call_malloc(size: u32, ctx: &mut Ctx) -> u32 {
    get_emscripten_data(ctx).malloc.call(size).unwrap()
}

pub fn call_memalign(alignment: u32, size: u32, ctx: &mut Ctx) -> u32 {
    if let Some(memalign) = &get_emscripten_data(ctx).memalign {
        memalign.call(alignment, size).unwrap()
    } else {
        panic!("Memalign is set to None");
    }
}

pub fn call_memset(pointer: u32, value: u32, size: u32, ctx: &mut Ctx) -> u32 {
    get_emscripten_data(ctx)
        .memset
        .call(pointer, value, size)
        .unwrap()
}

pub(crate) fn get_emscripten_data(ctx: &mut Ctx) -> &mut EmscriptenData {
    unsafe { &mut *(ctx.data as *mut EmscriptenData) }
}

pub fn _getpagesize(_ctx: &mut Ctx) -> u32 {
    debug!("emscripten::_getpagesize");
    16384
}

#[allow(clippy::cast_ptr_alignment)]
pub fn ___build_environment(environ: c_int, ctx: &mut Ctx) {
    debug!("emscripten::___build_environment {}", environ);
    const MAX_ENV_VALUES: u32 = 64;
    const TOTAL_ENV_SIZE: u32 = 1024;
    let environment = emscripten_memory_pointer!(ctx.memory(0), environ) as *mut c_int;
    unsafe {
        let (pool_offset, _pool_slice): (u32, &mut [u8]) =
            allocate_on_stack(TOTAL_ENV_SIZE as u32, ctx);
        let (env_offset, _env_slice): (u32, &mut [u8]) =
            allocate_on_stack((MAX_ENV_VALUES * 4) as u32, ctx);
        let env_ptr = emscripten_memory_pointer!(ctx.memory(0), env_offset) as *mut c_int;
        let mut _pool_ptr = emscripten_memory_pointer!(ctx.memory(0), pool_offset) as *mut c_int;
        *env_ptr = pool_offset as i32;
        *environment = env_offset as i32;

        // *env_ptr = 0;
    };
    // unsafe {
    //     *env_ptr = 0;
    // };
}

pub fn ___assert_fail(a: c_int, b: c_int, c: c_int, d: c_int, _ctx: &mut Ctx) {
    debug!("emscripten::___assert_fail {} {} {} {}", a, b, c, d);
    // TODO: Implement like emscripten expects regarding memory/page size
    // TODO raise an error
}
