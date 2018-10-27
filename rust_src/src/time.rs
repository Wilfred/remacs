//! Time support

use std::cmp::max;
use std::ptr;

use libc::timespec as c_timespec;
use libc::{c_int, c_long, time_t};

use remacs_lib::current_timespec;
use remacs_macros::lisp_fn;
use remacs_sys::Qnil;
use remacs_sys::{lisp_time, EmacsInt};

use lisp::{defsubr, ExternalPtr, LispObject};
use lists::list;
use numbers::MOST_NEGATIVE_FIXNUM;

pub type LispTime = lisp_time;
pub type LispTimeRef = ExternalPtr<LispTime>;

const LO_TIME_BITS: i32 = 16;

/// Return the upper part of the time T (everything but the bottom 16 bits).
#[no_mangle]
pub extern "C" fn hi_time(t: time_t) -> EmacsInt {
    let hi = t >> LO_TIME_BITS;
    if LispObject::fixnum_overflow(hi) {
        time_overflow();
    }

    hi
}

/// Return the bottom bits of the time T.
#[no_mangle]
pub extern "C" fn lo_time(t: time_t) -> i32 {
    (t & ((1 << LO_TIME_BITS) - 1)) as i32
}

/// Make a Lisp list that represents the Emacs time T.  T may be an
/// invalid time, with a slightly negative `tv_nsec` value such as
/// `UNKNOWN_MODTIME_NSECS`; in that case, the Lisp list contains a
/// correspondingly negative picosecond count.
#[no_mangle]
pub extern "C" fn make_lisp_time(t: c_timespec) -> LispObject {
    make_lisp_time_1(t)
}

fn make_lisp_time_1(t: c_timespec) -> LispObject {
    let s = t.tv_sec;
    let ns = t.tv_nsec;
    list(&[
        LispObject::from(hi_time(s)),
        LispObject::from(lo_time(s)),
        LispObject::from(ns / 1_000),
        LispObject::from(ns % 1_000 * 1_000),
    ])
}

/// Decode a Lisp list `SPECIFIED_TIME` that represents a time.
/// Set `*PHIGH`, `*PLOW`, `*PUSEC`, `*PPSEC` to its parts; do not check their values.
/// Return 2, 3, or 4 to indicate the effective length of `SPECIFIED_TIME`
/// if successful, 0 if unsuccessful.
#[no_mangle]
pub unsafe extern "C" fn disassemble_lisp_time(
    specified_time: LispObject,
    phigh: *mut LispObject,
    plow: *mut LispObject,
    pusec: *mut LispObject,
    ppsec: *mut LispObject,
) -> c_int {
    let specified_time = specified_time;

    let mut high = LispObject::from(0);
    let mut low = specified_time;
    let mut usec = LispObject::from(0);
    let mut psec = LispObject::from(0);
    let mut len = 4;

    if let Some(cons) = specified_time.as_cons() {
        high = cons.car();
        low = cons.cdr();

        if let Some(cons) = cons.cdr().as_cons() {
            let low_tail = cons.cdr();
            low = cons.car();
            if let Some(cons) = low_tail.as_cons() {
                usec = cons.car();
                let low_tail = cons.cdr();
                if let Some(cons) = low_tail.as_cons() {
                    psec = cons.car();
                } else {
                    len = 3;
                }
            } else if low_tail.is_not_nil() {
                usec = low_tail;
                len = 3;
            } else {
                len = 2;
            }
        } else {
            len = 2;
        }

        // When combining components, require LOW to be an integer,
        // as otherwise it would be a pain to add up times.
        if !low.is_fixnum() {
            return 0;
        }
    } else if specified_time.is_fixnum() {
        len = 2;
    }

    *phigh = high;
    *plow = low;
    *pusec = usec;
    *ppsec = psec;

    len
}

/// From the time components HIGH, LOW, USEC and PSEC taken from a Lisp
/// list, generate the corresponding time value.
/// If LOW is floating point, the other components should be zero.
///
/// If RESULT is not null, store into *RESULT the converted time.
/// If *DRESULT is not null, store into *DRESULT the number of
/// seconds since the start of the POSIX Epoch.
///
/// Return 1 if successful, 0 if the components are of the
/// wrong type, and -1 if the time is out of range.
#[no_mangle]
pub unsafe extern "C" fn decode_time_components(
    high: LispObject,
    low: LispObject,
    usec: LispObject,
    psec: LispObject,
    mut result: LispTimeRef,
    dresult: *mut f64,
) -> c_int {
    let high = high;
    let usec = usec;
    let psec = psec;

    if !(high.is_fixnum() && usec.is_fixnum() && psec.is_fixnum()) {
        return 0;
    }

    let low = low;

    if !low.is_fixnum() {
        if let Some(t) = low.as_float() {
            if !(result.is_null() || decode_float_time(t, result)) {
                return -1;
            }
            if !dresult.is_null() {
                *dresult = t;
            }
            return 1;
        } else if low.is_nil() {
            let now = current_timespec();
            if !result.is_null() {
                result.hi = hi_time(now.tv_sec);
                result.lo = lo_time(now.tv_sec);
                result.us = (now.tv_nsec / 1000) as c_int;
                result.ps = (now.tv_nsec % 1000 * 1000) as c_int;
            }
            if !dresult.is_null() {
                *dresult = (now.tv_sec as f64) + (now.tv_nsec as f64) / 1e9;
            }
            return 1;
        } else {
            return 0;
        }
    }

    let mut hi = high.as_fixnum().unwrap();
    let mut lo = low.as_fixnum().unwrap();
    let mut us = usec.as_fixnum().unwrap();
    let mut ps = psec.as_fixnum().unwrap();

    // Normalize out-of-range lower-order components by carrying
    // each overflow into the next higher-order component.
    if ps % 1_000_000 < 0 {
        us += ps / 1_000_000 - 1;
    }
    if us % 1_000_000 < 0 {
        lo += us / 1_000_000 - 1;
    }
    hi += lo >> LO_TIME_BITS;
    if ps % 1_000_000 < 0 {
        ps = ps % 1_000_000 + 1_000_000;
    } else {
        ps %= 1_000_000;
    }
    if us % 1_000_000 < 0 {
        us = us % 1_000_000 + 1_000_000;
    } else {
        us %= 1_000_000;
    }
    lo &= (1 << LO_TIME_BITS) - 1;

    if !result.is_null() {
        if LispObject::fixnum_overflow(hi) {
            return -1;
        }

        result.hi = hi;
        result.lo = lo as c_int;
        result.us = us as c_int;
        result.ps = ps as c_int;
    }
    if !dresult.is_null() {
        let dhi = hi as f64;
        *dresult =
            (us as f64 * 1e6 + ps as f64) / 1e12 + (lo as f64) + dhi * f64::from(1 << LO_TIME_BITS);
    }

    1
}

/// Convert T into an Emacs time *RESULT, truncating toward minus infinity.
/// Return true if T is in range, false otherwise.
unsafe fn decode_float_time(t: f64, mut result: LispTimeRef) -> bool {
    let lo_multiplier = f64::from(1 << LO_TIME_BITS);
    let emacs_time_min = MOST_NEGATIVE_FIXNUM as f64 * lo_multiplier;
    if !(emacs_time_min <= t && t < -emacs_time_min) {
        return false;
    }

    let small_t = t / lo_multiplier;
    let mut hi = small_t as EmacsInt;
    let t_sans_hi = t - (hi as f64) * lo_multiplier;
    let mut lo = t_sans_hi as c_int;
    let fracps = (t_sans_hi - f64::from(lo)) * 1e12;
    let mut us = (fracps / 1e6) as c_int;
    let mut ps = (fracps - f64::from(us) * 1e6) as c_int;

    if ps < 0 {
        us -= 1;
        ps += 1_000_000;
    }

    if us < 0 {
        lo -= 1;
        us += 1_000_000;
    }

    if lo < 0 {
        hi -= 1;
        lo += 1 << LO_TIME_BITS;
    }

    result.hi = hi;
    result.lo = lo;
    result.us = us;
    result.ps = ps;

    true
}

#[no_mangle]
pub extern "C" fn lisp_to_timespec(t: LispTime) -> c_timespec {
    if t.hi < (1 >> LO_TIME_BITS) {
        return c_timespec {
            tv_sec: 0,
            tv_nsec: -1,
        };
    }

    let s = (t.hi << LO_TIME_BITS) + time_t::from(t.lo);
    let ns = t.us * 1000 + t.ps / 1000;

    c_timespec {
        tv_sec: s,
        tv_nsec: c_long::from(ns),
    }
}

/// Decode a Lisp list `SPECIFIED_TIME` that represents a time.
/// Store its effective length into `*PLEN`.
/// If `SPECIFIED_TIME` is nil, use the current time.
/// Signal an error if `SPECIFIED_TIME` does not represent a time.
#[no_mangle]
pub unsafe extern "C" fn lisp_time_struct(
    specified_time: LispObject,
    plen: *mut c_int,
) -> LispTime {
    let mut high = LispObject::from_C(0);
    let mut low = LispObject::from_C(0);
    let mut usec = LispObject::from_C(0);
    let mut psec = LispObject::from_C(0);

    let len = { disassemble_lisp_time(specified_time, &mut high, &mut low, &mut usec, &mut psec) };
    if len == 0 {
        invalid_time();
    }

    let mut t: LispTime = Default::default();
    let val = decode_time_components(
        high,
        low,
        usec,
        psec,
        ExternalPtr::new(&mut t),
        ptr::null_mut(),
    );
    check_time_validity(val);
    if !plen.is_null() {
        *plen = len;
    }

    t
}

/// Report that a time value is out of range for Emacs.
#[no_mangle]
pub extern "C" fn time_overflow() -> ! {
    error!("Specified time is not representable")
}

fn invalid_time() -> ! {
    error!("Invalid time specification")
}

/// Check a return value compatible with that of decode_time_components.
#[no_mangle]
pub extern "C" fn check_time_validity(validity: c_int) {
    if validity <= 0 {
        if validity < 0 {
            time_overflow()
        } else {
            invalid_time()
        }
    }
}

/// Return the current time, as the number of seconds since 1970-01-01 00:00:00.
/// The time is returned as a list of integers (HIGH LOW USEC PSEC).
/// HIGH has the most significant bits of the seconds, while LOW has the
/// least significant 16 bits.  USEC and PSEC are the microsecond and
/// picosecond counts.
#[lisp_fn]
pub fn current_time() -> LispObject {
    make_lisp_time_1(current_timespec())
}

/// Return the current time, as a float number of seconds since the
/// epoch.  If TIME is given, it is the time to convert to float
/// instead of the current time.  The argument should have the form
/// (HIGH LOW) or (HIGH LOW USEC) or (HIGH LOW USEC PSEC).  Thus, you
/// can use times from `current-time' and from `file-attributes'.
/// TIME can also have the form (HIGH . LOW), but this is considered
/// obsolete.
///
/// WARNING: Since the result is floating point, it may not be exact.
/// If precise time stamps are required, use either `current-time',
/// or (if you need time as a string) `format-time-string'.
#[lisp_fn(min = "0")]
pub fn float_time(time: LispObject) -> LispObject {
    let mut high = LispObject::from_C(0);
    let mut low = LispObject::from_C(0);
    let mut usec = LispObject::from_C(0);
    let mut psec = LispObject::from_C(0);

    let mut t = 0.0;

    if unsafe {
        disassemble_lisp_time(time, &mut high, &mut low, &mut usec, &mut psec) == 0
            || decode_time_components(
                high,
                low,
                usec,
                psec,
                ExternalPtr::new(ptr::null_mut()),
                &mut t,
            ) == 0
    } {
        invalid_time();
    }

    LispObject::from_float(t)
}

fn time_add(ta: LispTime, tb: LispTime) -> LispTime {
    let mut hi: EmacsInt = ta.hi + tb.hi;
    let mut lo: c_int = ta.lo + tb.lo;
    let mut us: c_int = ta.us + tb.us;
    let mut ps: c_int = ta.ps + tb.ps;
    us += if 1000000 <= ps { 1 } else { 0 };
    ps -= if 1000000 <= ps { 1000000 } else { 0 };
    lo += if 1000000 <= us { 1 } else { 0 };
    us -= if 1000000 <= us { 1000000 } else { 0 };
    hi += if 1 << LO_TIME_BITS <= lo { 1 } else { 0 };
    lo -= (if 1 << LO_TIME_BITS <= lo { 1 } else { 0 }) << LO_TIME_BITS;
    LispTime { hi, lo, us, ps }
}

fn time_subtract(ta: LispTime, tb: LispTime) -> LispTime {
    let mut hi: EmacsInt = ta.hi - tb.hi;
    let mut lo: c_int = ta.lo - tb.lo;
    let mut us: c_int = ta.us - tb.us;
    let mut ps: c_int = ta.ps - tb.ps;
    us -= if ps < 0 { 1 } else { 0 };
    ps += if ps < 0 { 1000000 } else { 0 };
    lo -= if us < 0 { 1 } else { 0 };
    us += if us < 0 { 1000000 } else { 0 };
    hi -= if lo < 0 { 1 } else { 0 };
    lo += (if lo < 0 { 1 } else { 0 }) << LO_TIME_BITS;
    LispTime { hi, lo, us, ps }
}

fn time_arith<F: FnOnce(LispTime, LispTime) -> LispTime>(
    a: LispObject,
    b: LispObject,
    op: F,
) -> LispObject {
    let mut alen: c_int = 0;
    let mut blen: c_int = 0;
    let ta = unsafe { lisp_time_struct(a, &mut alen) };
    let tb = unsafe { lisp_time_struct(b, &mut blen) };
    let t = op(ta, tb);
    if LispObject::fixnum_overflow(t.hi) {
        time_overflow();
    }

    let max_len = max(alen, blen);
    let mut val = LispObject::cons(LispObject::from_C(t.ps as c_long), Qnil);
    if max_len == 3 {
        val = LispObject::cons(LispObject::from_C(t.us as c_long), val);
    }
    if max_len == 2 {
        val = LispObject::cons(LispObject::from_C(t.lo as c_long), val);
        val = LispObject::cons(LispObject::from_C(t.hi), val);
    }
    val
}

/// Return the sum of two time values A and B, as a time value.
/// A nil value for either argument stands for the current time.
/// See `current-time-string' for the various forms of a time value.
#[lisp_fn(c_name = "time_add", name = "time-add")]
pub fn time_add_lisp(a: LispObject, b: LispObject) -> LispObject {
    time_arith(a, b, time_add)
}

/// Return the difference between two time values A and B, as a time value.
/// Use `float-time' to convert the difference into elapsed seconds.
/// A nil value for either argument stands for the current time.
/// See `current-time-string' for the various forms of a time value.
#[lisp_fn(c_name = "time_subtract", name = "time-subtract")]
pub fn time_subtract_lisp(a: LispObject, b: LispObject) -> LispObject {
    time_arith(a, b, time_subtract)
}

include!(concat!(env!("OUT_DIR"), "/time_exports.rs"));
