#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]
#![cfg_attr(feature = "strict", deny(warnings))]

extern crate errno;
#[macro_use]
extern crate lazy_static;
extern crate libc;
extern crate rand;
extern crate regex;
extern crate remacs_util;
extern crate syn;
extern crate time as time_crate;

mod docfile;
mod files;
mod math;
mod time;

// Used by make-docfile
pub use crate::docfile::scan_rust_file;

// Used for creating temporary files in emacs
pub use crate::files::rust_make_temp;

pub use crate::math::rust_count_one_bits;
pub use crate::math::rust_count_trailing_zero_bits;

pub use crate::time::current_timespec;

#[cfg(all(not(test), target_os = "windows"))]
#[no_mangle]
pub unsafe extern "C" fn __chkstk() {}
