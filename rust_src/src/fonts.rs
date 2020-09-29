//! font support

use std::ffi::CString;
use std::ptr;

use remacs_macros::lisp_fn;

use crate::{
    data, fns,
    frame::{LispFrameLiveOrSelected, LispFrameRef},
    lisp::{ExternalPtr, LispObject},
    lists::{LispCons, LispConsCircularChecks, LispConsEndChecks},
    obarray::intern,
    remacs_sys::font_match_p as c_font_match_p,
    remacs_sys::font_property_index::FONT_TYPE_INDEX,
    remacs_sys::{font_add_log, font_at, font_list_entities, font_make_spec, font_sort_entities},
    remacs_sys::{
        pvec_type, Lisp_Font_Object, Lisp_Font_Spec, Lisp_Type, FONT_ENTITY_MAX, FONT_OBJECT_MAX,
        FONT_SPEC_MAX,
    },
    remacs_sys::{EmacsInt, Qfont, Qfont_entity, Qfont_object, Qfont_spec, Qnil},
    threads::ThreadState,
    vectors::LispVectorlikeRef,
    windows::{LispWindowLiveOrSelected, LispWindowRef},
};

// A font is not a type in and of itself, it's just a group of three kinds of
// pseudovector. This newtype allows us to define methods that yield the actual
// font types: Spec, Entity, and Object.
#[repr(transparent)]
pub struct LispFontRef(LispVectorlikeRef);

impl LispFontRef {
    pub const fn from_vectorlike(v: LispVectorlikeRef) -> Self {
        Self(v)
    }

    pub fn is_font_spec(&self) -> bool {
        self.0.pseudovector_size() == EmacsInt::from(FONT_SPEC_MAX)
    }

    pub fn is_font_entity(&self) -> bool {
        self.0.pseudovector_size() == EmacsInt::from(FONT_ENTITY_MAX)
    }

    pub fn is_font_object(&self) -> bool {
        self.0.pseudovector_size() == EmacsInt::from(FONT_OBJECT_MAX)
    }
}

impl LispObject {
    pub fn is_font(self) -> bool {
        self.as_vectorlike()
            .map_or(false, |v| v.is_pseudovector(pvec_type::PVEC_FONT))
    }

    pub fn as_font(self) -> Option<LispFontRef> {
        self.as_vectorlike().and_then(|v| {
            if v.is_pseudovector(pvec_type::PVEC_FONT) {
                Some(LispFontRef::from_vectorlike(v))
            } else {
                None
            }
        })
    }

    pub fn is_font_entity(self) -> bool {
        self.is_font()
            && self.as_vectorlike().map_or(false, |vec| {
                vec.pseudovector_size() == EmacsInt::from(FONT_ENTITY_MAX)
            })
    }

    pub fn is_font_object(self) -> bool {
        self.is_font()
            && self.as_vectorlike().map_or(false, |vec| {
                vec.pseudovector_size() == EmacsInt::from(FONT_OBJECT_MAX)
            })
    }

    pub fn is_font_spec(self) -> bool {
        self.is_font()
            && self.as_vectorlike().map_or(false, |vec| {
                vec.pseudovector_size() == EmacsInt::from(FONT_SPEC_MAX)
            })
    }
}

pub enum FontExtraType {
    Spec,
    Entity,
    Object,
}

impl FontExtraType {
    // Needed for wrong_type! that is using a safe predicate. This may change in the future.
    #[allow(unused_unsafe)]
    pub fn from_symbol_or_error(extra_type: LispObject) -> Self {
        if extra_type.eq(unsafe { Qfont_spec }) {
            FontExtraType::Spec
        } else if extra_type.eq(unsafe { Qfont_entity }) {
            FontExtraType::Entity
        } else if extra_type.eq(unsafe { Qfont_object }) {
            FontExtraType::Object
        } else {
            wrong_type!(intern("font-extra-type"), extra_type);
        }
    }
}

pub type LispFontObjectRef = ExternalPtr<Lisp_Font_Object>;

impl LispFontObjectRef {
    pub fn add_log(self, action: &str, result: impl Into<LispObject>) {
        let c_str = CString::new(action).unwrap();
        unsafe { font_add_log(c_str.as_ptr(), self.into(), result.into()) }
    }

    pub fn close(mut self, mut _frame: LispFrameRef) {
        if data::aref(self.into(), FONT_TYPE_INDEX.into()).is_nil() {
            // Already closed
            return;
        }
        self.add_log("close", false);
        unsafe {
            if let Some(f) = (*self.driver).close {
                f(self.as_mut())
            }
            #[cfg(feature = "window-system")]
            {
                #[cfg(feature = "window-system-x11")]
                let mut display_info = &mut *(*_frame.output_data.x).display_info;
                #[cfg(feature = "window-system-nextstep")]
                let mut display_info = &mut *(*_frame.output_data.ns).display_info;
                #[cfg(feature = "window-system-w32")]
                let mut display_info = &mut *(*_frame.output_data.w32).display_info;
                debug_assert!(display_info.n_fonts > 0);
                display_info.n_fonts -= 1;
            }
        }
    }
}

impl From<LispFontObjectRef> for LispObject {
    fn from(f: LispFontObjectRef) -> Self {
        Self::tag_ptr(f, Lisp_Type::Lisp_Vectorlike)
    }
}

impl From<LispObject> for LispFontObjectRef {
    fn from(o: LispObject) -> Self {
        match o.into() {
            Some(font) => font,
            None => wrong_type!(Qfont_object, o),
        }
    }
}

impl From<LispObject> for Option<LispFontObjectRef> {
    fn from(o: LispObject) -> Self {
        o.as_vectorlike().and_then(|v| {
            if v.is_pseudovector(pvec_type::PVEC_FONT) && o.is_font_object() {
                Some(v.cast())
            } else {
                None
            }
        })
    }
}

pub type LispFontSpecRef = ExternalPtr<Lisp_Font_Spec>;

impl From<LispFontSpecRef> for LispObject {
    fn from(f: LispFontSpecRef) -> Self {
        Self::tag_ptr(f, Lisp_Type::Lisp_Vectorlike)
    }
}

impl From<LispObject> for LispFontSpecRef {
    fn from(o: LispObject) -> Self {
        match o.into() {
            Some(spec) => spec,
            None => wrong_type!(Qfont_spec, o),
        }
    }
}

impl From<LispObject> for Option<LispFontSpecRef> {
    fn from(o: LispObject) -> Self {
        o.as_vectorlike().and_then(|v| {
            if v.is_pseudovector(pvec_type::PVEC_FONT) && o.is_font_spec() {
                Some(v.cast())
            } else {
                None
            }
        })
    }
}

#[no_mangle]
pub extern "C" fn font_vconcat_entity_vectors(list: LispObject) -> LispObject {
    vconcat_entity_vectors(list.into())
}

/// Concatenate all elements of LIST into one vector. LIST is a list of font-entity vectors.
fn vconcat_entity_vectors(list: LispCons) -> LispObject {
    let mut args: Vec<LispObject> = list
        .iter_cars(LispConsEndChecks::off, LispConsCircularChecks::off)
        .collect();
    fns::vconcat(&mut args)
}

/// Return t if OBJECT is a font-spec, font-entity, or font-object.
/// Return nil otherwise.
/// Optional 2nd argument EXTRA-TYPE, if non-nil, specifies to check
/// which kind of font it is.  It must be one of `font-spec', `font-entity',
/// `font-object'.
#[lisp_fn(min = "1")]
pub fn fontp(object: LispObject, extra_type: LispObject) -> bool {
    // For compatibility with the C version, checking that object is a font
    // takes priority over checking that extra_type is well-formed.
    object.as_font().map_or(false, |f| {
        if extra_type.is_nil() {
            true
        } else {
            match FontExtraType::from_symbol_or_error(extra_type) {
                FontExtraType::Spec => f.is_font_spec(),
                FontExtraType::Entity => f.is_font_entity(),
                FontExtraType::Object => f.is_font_object(),
            }
        }
    })
}

/// Return t if and only if font-spec SPEC matches with FONT.  FONT is a font-spec, font-entity,
/// or font-object.
#[lisp_fn]
pub fn font_match_p(spec: LispObject, font: LispObject) -> bool {
    if !spec.is_font_spec() {
        wrong_type!(Qfont_spec, spec)
    }
    if !font.is_font() {
        wrong_type!(Qfont, font)
    }
    unsafe { c_font_match_p(spec, font) }
}

/// Return a font-entity matching with FONT-SPEC on the current frame.
/// Optional 2nd argument FRAME, if non-nil, specifies the target frame.
#[lisp_fn(min = "1")]
pub fn find_font(spec: LispFontSpecRef, frame: LispFrameLiveOrSelected) -> LispObject {
    let val = list_fonts(spec, frame, Some(1.into()), None);
    match val.into() {
        Some((a, _)) => a,
        None => val,
    }
}

/// Close FONT-OBJECT
#[lisp_fn(min = "1")]
pub fn close_font(font_object: LispFontObjectRef, frame: LispFrameLiveOrSelected) {
    let frame: LispFrameRef = frame.into();
    font_object.close(frame)
}

/// Return a font-object for displaying a character at POSITION.
/// Optional second arg WINDOW, if non-nil, is a window displaying
/// the current buffer.  It defaults to the currently selected window.
/// Optional third arg STRING, if non-nil, is a string containing the target
/// character at index specified by POSITION.
#[lisp_fn(min = "1", c_name = "font_at", name = "font-at")]
pub fn font_at_lisp(
    position: LispObject,
    window: LispWindowLiveOrSelected,
    string: LispObject,
) -> LispObject {
    let mut w: LispWindowRef = window.into();
    let cur_buf = ThreadState::current_buffer_unchecked();

    let pos = match string.as_string() {
        Some(s) => {
            let pos = EmacsInt::from(position) as isize;
            if !(0 <= pos && pos < s.len_bytes()) {
                args_out_of_range!(string, position);
            }
            pos
        }

        _ => {
            if w.contents != cur_buf.into() {
                error!("Specified window is not displaying the current buffer");
            }
            position.as_number_coerce_marker_or_error();
            let pos = EmacsInt::from(position) as isize;

            let begv = cur_buf.begv;
            let zv = cur_buf.zv;
            if !(begv <= pos && pos < zv) {
                args_out_of_range!(
                    position,
                    LispObject::from(begv as EmacsInt),
                    LispObject::from(zv as EmacsInt)
                );
            }
            pos
        }
    };
    unsafe { font_at(-1, pos, ptr::null_mut(), w.as_mut(), string) }
}

/// List available fonts matching FONT-SPEC on the current frame.
/// Optional 2nd argument FRAME specifies the target frame.
/// Optional 3rd argument NUM, if non-nil, limits the number of returned fonts.
/// Optional 4th argument PREFER, if non-nil, is a font-spec to
/// control the order of the returned list.  Fonts are sorted by
/// how close they are to PREFER.
#[lisp_fn(min = "1")]
pub fn list_fonts(
    font_spec: LispFontSpecRef,
    frame: LispFrameLiveOrSelected,
    num: Option<EmacsInt>,
    prefer: Option<LispFontSpecRef>,
) -> LispObject {
    let mut frame: LispFrameRef = frame.into();

    let n = match num {
        Some(n) if n <= 0 => return Qnil,
        Some(n) => n,
        None => 0,
    } as usize;

    let list = unsafe { font_list_entities(frame.as_mut(), font_spec.into()) };

    match list.into() {
        Some((car, cdr)) if cdr.is_nil() => match car.as_vector() {
            Some(vec) if vec.len() == 1 => return list!(vec.get(0)),
            _ => (),
        },
        Some(_) => (),
        None => return Qnil,
    }

    let vec = match prefer {
        None => vconcat_entity_vectors(list.into()),
        Some(f) => unsafe { font_sort_entities(list, f.into(), frame.as_mut(), 0) },
    }
    .force_vector();

    if n == 0 || n >= vec.len() {
        fns::append(&mut [vec.into(), Qnil])
    } else {
        (0..=n)
            .rev()
            .fold(Qnil, |list, n| (vec.get(n), list).into())
    }
}

/// Return FRAME's font cache.  Mainly used for debugging.
/// If FRAME is omitted or nil, use the selected frame.
#[lisp_fn(min = "0")]
pub fn frame_font_cache(_frame: LispFrameLiveOrSelected) -> LispObject {
    #[cfg(feature = "window-system")]
    {
        let frame: LispFrameRef = _frame.into();
        if frame.is_gui_window() {
            unsafe {
                #[cfg(feature = "window-system-x11")]
                let display_info = (*frame.output_data.x).display_info;
                #[cfg(feature = "window-system-nextstep")]
                let display_info = (*frame.output_data.ns).display_info;
                #[cfg(feature = "window-system-w32")]
                let display_info = (*frame.output_data.w32).display_info;
                (*display_info).name_list_element
            }
        } else {
            Qnil
        }
    }
    #[cfg(not(feature = "window-system"))]
    {
        Qnil
    }
}

#[no_mangle]
pub extern "C" fn test_font_spec() {
    let spec1 = unsafe { font_make_spec() };
    let spec_ref: LispFontSpecRef = spec1.into();
    let spec2 = spec_ref.into();

    assert_eq!(spec1, spec2);
}

include!(concat!(env!("OUT_DIR"), "/fonts_exports.rs"));
