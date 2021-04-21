//! Text property support

use std::ptr;

use remacs_macros::lisp_fn;

use crate::{
    lisp::LispObject,
    numbers::LispNumber,
    remacs_sys::{
        get_char_property_and_overlay, set_text_properties, textget, validate_interval_range,
    },
    remacs_sys::{EmacsInt, Lisp_Interval, Qnil, Qt},
    threads::ThreadState,
};

/// Return the value of POSITION's property PROP, in OBJECT.
/// Both overlay properties and text properties are checked.
/// OBJECT is optional and defaults to the current buffer.
/// If POSITION is at the end of OBJECT, the value is nil.
/// If OBJECT is a buffer, then overlay properties are considered as well as
/// text properties.
/// If OBJECT is a window, then that window's buffer is used, but window-specific
/// overlays are considered only if they are associated with OBJECT.
#[lisp_fn(min = "2")]
pub fn get_char_property(position: LispNumber, prop: LispObject, object: LispObject) -> LispObject {
    unsafe { get_char_property_and_overlay(position.into(), prop, object, ptr::null_mut()) }
}

/// Return the value of POSITION's property PROP, in OBJECT.
/// OBJECT should be a buffer or a string; if omitted or nil, it defaults
/// to the current buffer.
/// If POSITION is at the end of OBJECT, the value is nil.
#[lisp_fn(min = "2")]
pub fn get_text_property(position: LispNumber, prop: LispObject, object: LispObject) -> LispObject {
    unsafe { textget(text_properties_at(position.into(), object), prop) }
}

/// Return the list of properties of the character at POSITION in OBJECT.
/// If the optional second argument OBJECT is a buffer (or nil, which means
/// the current buffer), POSITION is a buffer position (integer or marker).
/// If OBJECT is a string, POSITION is a 0-based index into it.
/// If POSITION is at the end of OBJECT, the value is nil.
#[lisp_fn(min = "1")]
pub fn text_properties_at(mut position: LispObject, mut object: LispObject) -> LispObject {
    if object.is_nil() {
        object = ThreadState::current_buffer().into();
    }

    let i: *mut Lisp_Interval = unsafe {
        validate_interval_range(
            object,
            &mut position as *mut LispObject,
            &mut position as *mut LispObject,
            false,
        )
    };

    if i.is_null() {
        return Qnil;
    }

    unsafe {
        // If POSITION is at the end of the interval,
        // it means it's the end of OBJECT.
        // There are no properties at the very end,
        // since no character follows.
        let interval_length = total_length(i) - total_length((*i).right) - total_length((*i).left);
        if EmacsInt::from(position) == (interval_length + (*i).position) as EmacsInt {
            Qnil
        } else {
            (*i).plist
        }
    }
}

// A temporary replacement for the TOTAL_LENGTH macro from intervals.h
// Ideally we would implement a LispIntervalRef type to house this
// logic, but this function is here for the moment to keep the scope
// of these changes manageable.
unsafe fn total_length(interval: *mut Lisp_Interval) -> isize {
    if interval.is_null() {
        0
    } else {
        (*interval).total_length
    }
}

/// Completely replace properties of the text from START to END
/// The third argument PROPERTIES is the new property list.
/// If the optional fourth argument OBJECT is a buffer (or nil, which
/// means the current buffer), START and END are buffer positions (integers
/// or markers). If OBJECT is a string, START and END are 0-based indices into
/// it. If PROPERTIES is nil, the effect is to remove all properties from the
/// designated part of OBJECT
#[lisp_fn(
    c_name = "set_text_properties",
    name = "set-text-properties",
    min = "3"
)]
pub fn set_text_properties_lisp(
    start: LispObject,
    end: LispObject,
    properties: LispObject,
    object: LispObject,
) -> LispObject {
    unsafe { set_text_properties(start, end, properties, object, Qt) }
}

include!(concat!(env!("OUT_DIR"), "/textprop_exports.rs"));
