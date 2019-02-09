use super::utils::{copy_cstr_into_wasm, write_to_buf};
use libc::{c_char, c_int};
use std::mem;
use std::time::SystemTime;

#[cfg(not(target_os = "windows"))]
use libc::{clockid_t, time as libc_time};

#[cfg(target_os = "windows")]
use libc::time_t;

#[cfg(target_os = "windows")]
type clockid_t = c_int;

#[cfg(target_os = "windows")]
extern "C" {
    #[link_name = "time"]
    pub fn libc_time(s: *const time_t) -> time_t;
}

use time;

use super::env;
use wasmer_runtime_core::vm::Ctx;

#[cfg(target_os = "linux")]
use libc::{CLOCK_MONOTONIC, CLOCK_MONOTONIC_COARSE, CLOCK_REALTIME};

#[cfg(target_os = "macos")]
use libc::{CLOCK_MONOTONIC, CLOCK_REALTIME};
#[cfg(target_os = "macos")]
const CLOCK_MONOTONIC_COARSE: clockid_t = 6;

// some assumptions about the constants when targeting windows
#[cfg(target_os = "windows")]
const CLOCK_REALTIME: clockid_t = 0;
#[cfg(target_os = "windows")]
const CLOCK_MONOTONIC: clockid_t = 1;
#[cfg(target_os = "windows")]
const CLOCK_MONOTONIC_COARSE: clockid_t = 6;

/// emscripten: _gettimeofday
#[allow(clippy::cast_ptr_alignment)]
pub fn _gettimeofday(tp: c_int, tz: c_int, ctx: &mut Ctx) -> c_int {
    debug!("emscripten::_gettimeofday {} {}", tp, tz);
    #[repr(C)]
    struct GuestTimeVal {
        tv_sec: i32,
        tv_usec: i32,
    }

    assert!(
        tz == 0,
        "the timezone argument of `_gettimeofday` must be null"
    );
    unsafe {
        let now = SystemTime::now();
        let since_epoch = now.duration_since(SystemTime::UNIX_EPOCH).unwrap();
        let timeval_struct_ptr = emscripten_memory_pointer!(ctx.memory(0), tp) as *mut GuestTimeVal;

        (*timeval_struct_ptr).tv_sec = since_epoch.as_secs() as _;
        (*timeval_struct_ptr).tv_usec = since_epoch.subsec_nanos() as _;
    }
    0
}

/// emscripten: _clock_gettime
#[allow(clippy::cast_ptr_alignment)]
pub fn _clock_gettime(clk_id: clockid_t, tp: c_int, ctx: &mut Ctx) -> c_int {
    debug!("emscripten::_clock_gettime {} {}", clk_id, tp);
    // debug!("Memory {:?}", ctx.memory(0)[..]);
    #[repr(C)]
    struct GuestTimeSpec {
        tv_sec: i32,
        tv_nsec: i32,
    }

    let timespec = match clk_id {
        CLOCK_REALTIME => time::get_time(),
        CLOCK_MONOTONIC | CLOCK_MONOTONIC_COARSE => {
            let precise_ns = time::precise_time_ns();
            time::Timespec::new(
                (precise_ns / 1000000000) as i64,
                (precise_ns % 1000000000) as i32,
            )
        }
        _ => panic!("Clock with id \"{}\" is not supported.", clk_id),
    };

    unsafe {
        let timespec_struct_ptr =
            emscripten_memory_pointer!(ctx.memory(0), tp) as *mut GuestTimeSpec;
        (*timespec_struct_ptr).tv_sec = timespec.sec as _;
        (*timespec_struct_ptr).tv_nsec = timespec.nsec as _;
    }
    0
}

/// emscripten: ___clock_gettime
pub fn ___clock_gettime(clk_id: clockid_t, tp: c_int, ctx: &mut Ctx) -> c_int {
    debug!("emscripten::___clock_gettime {} {}", clk_id, tp);
    _clock_gettime(clk_id, tp, ctx)
}

/// emscripten: _clock
pub fn _clock(_ctx: &mut Ctx) -> c_int {
    debug!("emscripten::_clock");
    0 // TODO: unimplemented
}

/// emscripten: _difftime
pub fn _difftime(t0: u32, t1: u32, _ctx: &mut Ctx) -> f64 {
    debug!("emscripten::_difftime");
    (t0 - t1) as _
}

pub fn _gmtime_r(_one: i32, _two: i32, _ctx: &mut Ctx) -> i32 {
    debug!("emscripten::_gmtime_r");
    -1
}

pub fn _mktime(_one: i32, _ctx: &mut Ctx) -> i32 {
    debug!("emscripten::_mktime");
    -1
}

pub fn _gmtime(_one: i32, _ctx: &mut Ctx) -> i32 {
    debug!("emscripten::_gmtime");
    -1
}

#[repr(C)]
struct guest_tm {
    pub tm_sec: c_int,    // 0
    pub tm_min: c_int,    // 4
    pub tm_hour: c_int,   // 8
    pub tm_mday: c_int,   // 12
    pub tm_mon: c_int,    // 16
    pub tm_year: c_int,   // 20
    pub tm_wday: c_int,   // 24
    pub tm_yday: c_int,   // 28
    pub tm_isdst: c_int,  // 32
    pub tm_gmtoff: c_int, // 36
    pub tm_zone: c_int,   // 40
}

/// emscripten: _tvset
pub fn _tvset(_ctx: &mut Ctx) {
    debug!("emscripten::_tvset UNIMPLEMENTED");
}

/// formats time as a C string
#[allow(clippy::cast_ptr_alignment)]
unsafe fn fmt_time(time: u32, ctx: &mut Ctx) -> *const c_char {
    let date = &*(emscripten_memory_pointer!(ctx.memory(0), time) as *mut guest_tm);

    let days = vec!["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let months = vec![
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let year = 1900 + date.tm_year;

    let time_str = format!(
        // NOTE: TODO: Hack! The 14 accompanying chars are needed for some reason
        "{} {} {:2} {:02}:{:02}:{:02} {:4}\n\0\0\0\0\0\0\0\0\0\0\0\0\0",
        days[date.tm_wday as usize],
        months[date.tm_mon as usize],
        date.tm_mday,
        date.tm_hour,
        date.tm_min,
        date.tm_sec,
        year
    );

    time_str[0..26].as_ptr() as _
}

/// emscripten: _asctime
pub fn _asctime(time: u32, ctx: &mut Ctx) -> u32 {
    debug!("emscripten::_asctime {}", time);

    unsafe {
        let time_str_ptr = fmt_time(time, ctx);
        copy_cstr_into_wasm(ctx, time_str_ptr)

        // let c_str = emscripten_memory_pointer!(ctx.memory(0), res) as *mut i8;
        // use std::ffi::CStr;
        // debug!("#### cstr = {:?}", CStr::from_ptr(c_str));
    }
}

/// emscripten: _asctime_r
pub fn _asctime_r(time: u32, buf: u32, ctx: &mut Ctx) -> u32 {
    debug!("emscripten::_asctime_r {}, {}", time, buf);

    unsafe {
        // NOTE: asctime_r is specced to behave in an undefined manner if the algorithm would attempt
        //      to write out more than 26 bytes (including the null terminator).
        //      See http://pubs.opengroup.org/onlinepubs/9699919799/functions/asctime.html
        //      Our undefined behavior is to truncate the write to at most 26 bytes, including null terminator.
        let time_str_ptr = fmt_time(time, ctx);
        write_to_buf(time_str_ptr, buf, 26, ctx)

        // let c_str = emscripten_memory_pointer!(ctx.memory(0), res) as *mut i8;
        // use std::ffi::CStr;
        // debug!("#### cstr = {:?}", CStr::from_ptr(c_str));
    }
}

/// emscripten: _localtime
#[allow(clippy::cast_ptr_alignment)]
pub fn _localtime(time_p: u32, ctx: &mut Ctx) -> c_int {
    debug!("emscripten::_localtime {}", time_p);
    // NOTE: emscripten seems to want tzset() called in this function
    //      https://stackoverflow.com/questions/19170721/real-time-awareness-of-timezone-change-in-localtime-vs-localtime-r

    let timespec = unsafe {
        let time_p_addr = emscripten_memory_pointer!(ctx.memory(0), time_p) as *mut i64;
        let seconds = *time_p_addr.clone();
        time::Timespec::new(seconds, 0)
    };
    let result_tm = time::at(timespec);

    unsafe {
        let tm_struct_offset = env::call_malloc(mem::size_of::<guest_tm>() as _, ctx);
        let tm_struct_ptr =
            emscripten_memory_pointer!(ctx.memory(0), tm_struct_offset) as *mut guest_tm;
        // debug!(
        //     ">>>>>>> time = {}, {}, {}, {}, {}, {}, {}, {}",
        //     result_tm.tm_sec, result_tm.tm_min, result_tm.tm_hour, result_tm.tm_mday,
        //     result_tm.tm_mon, result_tm.tm_year, result_tm.tm_wday, result_tm.tm_yday,
        // );
        (*tm_struct_ptr).tm_sec = result_tm.tm_sec;
        (*tm_struct_ptr).tm_min = result_tm.tm_min;
        (*tm_struct_ptr).tm_hour = result_tm.tm_hour;
        (*tm_struct_ptr).tm_mday = result_tm.tm_mday;
        (*tm_struct_ptr).tm_mon = result_tm.tm_mon;
        (*tm_struct_ptr).tm_year = result_tm.tm_year;
        (*tm_struct_ptr).tm_wday = result_tm.tm_wday;
        (*tm_struct_ptr).tm_yday = result_tm.tm_yday;
        (*tm_struct_ptr).tm_isdst = result_tm.tm_isdst;
        (*tm_struct_ptr).tm_gmtoff = 0;
        (*tm_struct_ptr).tm_zone = 0;

        tm_struct_offset as _
    }
}
/// emscripten: _localtime_r
#[allow(clippy::cast_ptr_alignment)]
pub fn _localtime_r(time_p: u32, result: u32, ctx: &mut Ctx) -> c_int {
    debug!("emscripten::_localtime_r {}", time_p);

    // NOTE: emscripten seems to want tzset() called in this function
    //      https://stackoverflow.com/questions/19170721/real-time-awareness-of-timezone-change-in-localtime-vs-localtime-r

    unsafe {
        let seconds = emscripten_memory_pointer!(ctx.memory(0), time_p) as *const i32;
        let timespec = time::Timespec::new(*seconds as _, 0);
        let result_tm = time::at(timespec);

        // debug!(
        //     ">>>>>>> time = {}, {}, {}, {}, {}, {}, {}, {}",
        //     result_tm.tm_sec, result_tm.tm_min, result_tm.tm_hour, result_tm.tm_mday,
        //     result_tm.tm_mon, result_tm.tm_year, result_tm.tm_wday, result_tm.tm_yday,
        // );

        let result_addr = emscripten_memory_pointer!(ctx.memory(0), result) as *mut guest_tm;

        (*result_addr).tm_sec = result_tm.tm_sec;
        (*result_addr).tm_min = result_tm.tm_min;
        (*result_addr).tm_hour = result_tm.tm_hour;
        (*result_addr).tm_mday = result_tm.tm_mday;
        (*result_addr).tm_mon = result_tm.tm_mon;
        (*result_addr).tm_year = result_tm.tm_year;
        (*result_addr).tm_wday = result_tm.tm_wday;
        (*result_addr).tm_yday = result_tm.tm_yday;
        (*result_addr).tm_isdst = result_tm.tm_isdst;
        (*result_addr).tm_gmtoff = 0;
        (*result_addr).tm_zone = 0;

        result as _
    }
}

/// emscripten: _time
#[allow(clippy::cast_ptr_alignment)]
pub fn _time(time_p: u32, ctx: &mut Ctx) -> i32 {
    debug!("emscripten::_time {}", time_p);

    unsafe {
        let time_p_addr = emscripten_memory_pointer!(ctx.memory(0), time_p) as *mut i64;
        libc_time(time_p_addr) as i32 // TODO review i64
    }
}

/// emscripten: _strftime
pub fn _strftime(
    s_ptr: c_int,
    maxsize: u32,
    format_ptr: c_int,
    tm_ptr: c_int,
    _ctx: &mut Ctx,
) -> i32 {
    debug!(
        "emscripten::_strftime {} {} {} {}",
        s_ptr, maxsize, format_ptr, tm_ptr
    );
    0
}
