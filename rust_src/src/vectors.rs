//! Functions operating on vector(like)s, and general sequences.

use std::cmp::Ordering;
use std::fmt;
use std::fmt::{Debug, Formatter};
use std::mem;
use std::ptr;

use libc::ptrdiff_t;

use remacs_macros::lisp_fn;

use crate::{
    buffers::LispBufferRef,
    chartable::{LispCharTableRef, LispSubCharTableAsciiRef, LispSubCharTableRef},
    data::aref,
    frame::LispFrameRef,
    hashtable::LispHashTableRef,
    lisp::{ExternalPtr, LispObject, LispStructuralEqual, LispSubrRef},
    lists::{inorder, nth, sort_list},
    multibyte::MAX_CHAR,
    process::LispProcessRef,
    remacs_sys::Fmake_vector,
    remacs_sys::{
        equal_kind, pvec_type, EmacsInt, Lisp_Bool_Vector, Lisp_Type, Lisp_Vector, Lisp_Vectorlike,
        Lisp_Vectorlike_With_Slots, More_Lisp_Bits, BITS_PER_BITS_WORD, BOOL_VECTOR_BITS_PER_CHAR,
        PSEUDOVECTOR_FLAG,
    },
    remacs_sys::{Qarrayp, Qsequencep, Qvectorp},
    threads::ThreadStateRef,
    window_configuration::SaveWindowDataRef,
    windows::LispWindowRef,
};

pub type LispVectorlikeRef = ExternalPtr<Lisp_Vectorlike>;
pub type LispVectorRef = ExternalPtr<Lisp_Vector>;
pub type LispBoolVecRef = ExternalPtr<Lisp_Bool_Vector>;
pub type LispVectorlikeSlotsRef = ExternalPtr<Lisp_Vectorlike_With_Slots>;

// Vectorlike support (LispType == 5)

impl LispObject {
    pub fn is_vectorlike(self) -> bool {
        self.get_type() == Lisp_Type::Lisp_Vectorlike
    }

    pub fn is_vector(self) -> bool {
        self.as_vectorlike()
            .map_or(false, LispVectorlikeRef::is_vector)
    }

    pub fn force_vectorlike(self) -> LispVectorlikeRef {
        unsafe { self.as_vectorlike_unchecked() }
    }

    pub fn as_vectorlike(self) -> Option<LispVectorlikeRef> {
        if self.is_vectorlike() {
            Some(unsafe { self.as_vectorlike_unchecked() })
        } else {
            None
        }
    }

    /*
    pub fn as_vectorlike_or_error(self) -> LispVectorlikeRef {
        if self.is_vectorlike() {
            LispVectorlikeRef::new(unsafe { mem::transmute(self.get_untaggedptr()) })
        } else {
            wrong_type!(Qvectorp, self)
        }
    }
    */

    pub unsafe fn as_vectorlike_unchecked(self) -> LispVectorlikeRef {
        LispVectorlikeRef::new(self.get_untaggedptr() as *mut Lisp_Vectorlike)
    }

    pub unsafe fn as_vectorlike_slots_unchecked(self) -> LispVectorlikeSlotsRef {
        LispVectorlikeSlotsRef::new(self.get_untaggedptr() as *mut Lisp_Vectorlike_With_Slots)
    }

    pub fn force_vectorlike_slots(self) -> LispVectorlikeSlotsRef {
        unsafe { self.as_vectorlike_slots_unchecked() }
    }

    pub fn as_vector(self) -> Option<LispVectorRef> {
        self.as_vectorlike().and_then(LispVectorlikeRef::as_vector)
    }

    pub fn as_vector_or_error(self) -> LispVectorRef {
        self.as_vector()
            .unwrap_or_else(|| wrong_type!(Qvectorp, self))
    }

    pub unsafe fn as_vector_unchecked(self) -> LispVectorRef {
        self.as_vectorlike_unchecked().as_vector_unchecked()
    }

    pub fn force_vector(self) -> LispVectorRef {
        unsafe { self.as_vector_unchecked() }
    }

    pub unsafe fn as_bool_vector_unchecked(self) -> LispBoolVecRef {
        LispBoolVecRef::new(self.get_untaggedptr() as *mut Lisp_Bool_Vector)
    }

    pub fn force_bool_vector(self) -> LispBoolVecRef {
        unsafe { self.as_bool_vector_unchecked() }
    }

    pub fn as_vector_or_string_length(self) -> isize {
        if let Some(s) = self.as_string() {
            return s.len_chars();
        } else if let Some(vl) = self.as_vectorlike() {
            if let Some(v) = vl.as_vector() {
                return v.len() as isize;
            }
        };

        wrong_type!(Qarrayp, self);
    }
}

impl Debug for LispVectorlikeRef {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        match self.as_vector() {
            Some(v) => {
                write!(f, "[")?;
                match v.as_slice() {
                    [] => {}
                    [first, rest @ ..] => {
                        write!(f, "{:?}", first)?;
                        for elt in rest {
                            write!(f, " {:?}", elt)?;
                        }
                    }
                }
                write!(f, "]")
            }
            None => write!(
                f,
                "#<VECTOR-LIKE @ {:p}: VAL({:#X})>",
                self.as_ptr(),
                LispObject::tag_ptr(*self, Lisp_Type::Lisp_Vectorlike).to_C()
            ),
        }
    }
}

impl LispVectorlikeRef {
    pub fn is_vector(self) -> bool {
        unsafe { self.header.size & (PSEUDOVECTOR_FLAG as isize) == 0 }
    }

    pub fn as_vector(self) -> Option<LispVectorRef> {
        if self.is_vector() {
            Some(self.cast())
        } else {
            None
        }
    }

    pub unsafe fn as_vector_unchecked(self) -> LispVectorRef {
        self.cast()
    }

    pub fn pseudovector_type(self) -> pvec_type {
        unsafe {
            mem::transmute(
                ((self.header.size & (More_Lisp_Bits::PVEC_TYPE_MASK as isize))
                    >> More_Lisp_Bits::PSEUDOVECTOR_AREA_BITS) as i32,
            )
        }
    }

    pub fn is_pseudovector(self, tp: pvec_type) -> bool {
        unsafe {
            self.header.size
                & (PSEUDOVECTOR_FLAG | More_Lisp_Bits::PVEC_TYPE_MASK as usize) as isize
                == (PSEUDOVECTOR_FLAG | ((tp as usize) << More_Lisp_Bits::PSEUDOVECTOR_AREA_BITS))
                    as isize
        }
    }

    pub fn pseudovector_size(self) -> EmacsInt {
        (unsafe { self.header.size } & (More_Lisp_Bits::PSEUDOVECTOR_SIZE_MASK as isize))
            as EmacsInt
    }

    pub fn as_bool_vector(self) -> Option<LispBoolVecRef> {
        if self.is_pseudovector(pvec_type::PVEC_BOOL_VECTOR) {
            Some(self.cast())
        } else {
            None
        }
    }

    pub fn as_buffer(self) -> Option<LispBufferRef> {
        if self.is_pseudovector(pvec_type::PVEC_BUFFER) {
            Some(self.cast())
        } else {
            None
        }
    }

    pub fn as_subr(self) -> Option<LispSubrRef> {
        if self.is_pseudovector(pvec_type::PVEC_SUBR) {
            Some(self.cast())
        } else {
            None
        }
    }

    pub fn as_window(self) -> Option<LispWindowRef> {
        if self.is_pseudovector(pvec_type::PVEC_WINDOW) {
            Some(self.cast())
        } else {
            None
        }
    }

    pub fn as_window_configuration(self) -> Option<SaveWindowDataRef> {
        if self.is_pseudovector(pvec_type::PVEC_WINDOW_CONFIGURATION) {
            Some(self.cast())
        } else {
            None
        }
    }

    pub fn as_frame(self) -> Option<LispFrameRef> {
        if self.is_pseudovector(pvec_type::PVEC_FRAME) {
            Some(self.cast())
        } else {
            None
        }
    }

    pub fn as_process(self) -> Option<LispProcessRef> {
        if self.is_pseudovector(pvec_type::PVEC_PROCESS) {
            Some(self.cast())
        } else {
            None
        }
    }

    pub fn as_thread(self) -> Option<ThreadStateRef> {
        if self.is_pseudovector(pvec_type::PVEC_THREAD) {
            Some(self.cast())
        } else {
            None
        }
    }

    pub fn as_char_table(self) -> Option<LispCharTableRef> {
        if self.is_pseudovector(pvec_type::PVEC_CHAR_TABLE) {
            Some(self.cast())
        } else {
            None
        }
    }

    pub fn as_sub_char_table(self) -> Option<LispSubCharTableRef> {
        if self.is_pseudovector(pvec_type::PVEC_SUB_CHAR_TABLE) {
            Some(self.cast())
        } else {
            None
        }
    }

    pub fn as_sub_char_table_ascii(self) -> Option<LispSubCharTableAsciiRef> {
        if self.is_pseudovector(pvec_type::PVEC_SUB_CHAR_TABLE) {
            Some(unsafe { mem::transmute(self) })
        } else {
            None
        }
    }

    pub fn as_compiled(self) -> Option<LispVectorlikeSlotsRef> {
        if self.is_pseudovector(pvec_type::PVEC_COMPILED) {
            Some(self.cast())
        } else {
            None
        }
    }

    pub fn as_record(self) -> Option<LispVectorlikeSlotsRef> {
        if self.is_pseudovector(pvec_type::PVEC_RECORD) {
            Some(self.cast())
        } else {
            None
        }
    }

    pub fn as_font(self) -> Option<LispVectorlikeSlotsRef> {
        if self.is_pseudovector(pvec_type::PVEC_FONT) {
            Some(self.cast())
        } else {
            None
        }
    }
}

impl LispStructuralEqual for LispVectorlikeRef {
    fn equal(
        &self,
        other: Self,
        kind: equal_kind::Type,
        depth: i32,
        ht: &mut LispHashTableRef,
    ) -> bool {
        // Pseudovectors have the type encoded in the size field, so this test
        // actually checks that the objects have the same type as well as the
        // same size.
        if unsafe { self.header.size != other.header.size } {
            false
        } else if let (Some(bv1), Some(bv2)) = (self.as_bool_vector(), other.as_bool_vector()) {
            bv1.equal(bv2, kind, depth, ht)
        } else if let (Some(cf1), Some(cf2)) = (
            self.as_window_configuration(),
            other.as_window_configuration(),
        ) {
            assert!(kind != equal_kind::EQUAL_NO_QUIT);
            cf1.equal(cf2, false)
        } else if let (Some(vec1), Some(vec2)) = (self.as_vector(), other.as_vector()) {
            vec1.equal(vec2, kind, depth, ht)
        } else if let (Some(fn1), Some(fn2)) = (self.as_compiled(), other.as_compiled()) {
            fn1.equal(fn2, kind, depth, ht)
        } else if let (Some(rec1), Some(rec2)) = (self.as_record(), other.as_record()) {
            rec1.equal(rec2, kind, depth, ht)
        } else if let (Some(font1), Some(font2)) = (self.as_font(), other.as_font()) {
            font1.equal(font2, kind, depth, ht)
        } else if let (Some(ct1), Some(ct2)) = (self.as_char_table(), other.as_char_table()) {
            ct1.equal(ct2, kind, depth, ht)
        } else if let (Some(ct1), Some(ct2)) = (self.as_sub_char_table(), other.as_sub_char_table())
        {
            ct1.equal(ct2, kind, depth, ht)
        } else if let (Some(ct1), Some(ct2)) = (
            self.as_sub_char_table_ascii(),
            other.as_sub_char_table_ascii(),
        ) {
            ct1.equal(ct2, kind, depth, ht)
        } else {
            // All of the other vector likes are not readily comparable.
            false
        }
    }
}

macro_rules! impl_vectorlike_ref {
    ($type:ident, $itertype:ident, $size_mask:expr) => {
        impl From<$type> for LispObject {
            fn from(v: $type) -> Self {
                Self::tag_ptr(v, Lisp_Type::Lisp_Vectorlike)
            }
        }

        impl $type {
            pub fn len(self) -> usize {
                (unsafe { self.header.size } & ($size_mask as isize)) as usize
            }

            pub fn is_empty(self) -> bool {
                self.len() == 0
            }

            pub fn as_slice(&self) -> &[LispObject] {
                let l = self.len();
                unsafe { self.contents.as_slice(l) }
            }

            pub fn as_mut_slice(&mut self) -> &mut [LispObject] {
                let l = self.len();
                unsafe { self.contents.as_mut_slice(l) }
            }

            pub fn get(self, idx: usize) -> LispObject {
                assert!(idx < self.len());
                unsafe { self.get_unchecked(idx) }
            }

            pub unsafe fn get_unchecked(self, idx: usize) -> LispObject {
                self.as_slice()[idx]
            }

            pub fn set(&mut self, idx: usize, item: LispObject) {
                assert!(idx < self.len());
                unsafe { self.set_unchecked(idx, item) };
            }

            pub fn set_checked(&mut self, idx: usize, item: LispObject) {
                if idx >= self.len() {
                    args_out_of_range!(*self, idx);
                }

                unsafe { self.set_unchecked(idx, item) };
            }

            pub unsafe fn set_unchecked(&mut self, idx: usize, item: LispObject) {
                self.as_mut_slice()[idx] = item
            }

            pub fn iter(&self) -> $itertype {
                $itertype::new(self)
            }
        }

        impl LispStructuralEqual for $type {
            fn equal(
                &self,
                other: Self,
                kind: equal_kind::Type,
                depth: i32,
                ht: &mut LispHashTableRef,
            ) -> bool {
                (0..self.len()).all(|i| {
                    let v1 = self.get(i as usize);
                    let v2 = other.get(i as usize);
                    v1.equal_internal(v2, kind, depth + 1, ht)
                })
            }
        }

        pub struct $itertype<'a> {
            vec: &'a $type,
            cur: usize,
            rev: usize,
        }

        impl<'a> $itertype<'a> {
            pub fn new(vec: &'a $type) -> Self {
                Self {
                    vec,
                    cur: 0,
                    rev: vec.len(),
                }
            }
        }

        impl<'a> Iterator for $itertype<'a> {
            type Item = LispObject;

            fn next(&mut self) -> Option<Self::Item> {
                if self.cur < self.rev {
                    let res = unsafe { self.vec.get_unchecked(self.cur) };
                    self.cur += 1;
                    Some(res)
                } else {
                    None
                }
            }
        }

        impl<'a> DoubleEndedIterator for $itertype<'a> {
            fn next_back(&mut self) -> Option<Self::Item> {
                if self.rev > self.cur {
                    let res = unsafe { self.vec.get_unchecked(self.rev - 1) };
                    self.rev -= 1;
                    Some(res)
                } else {
                    None
                }
            }
        }

        impl<'a> ExactSizeIterator for $itertype<'a> {}
    };
}

impl_vectorlike_ref! { LispVectorRef, LispVecIterator, ptrdiff_t::max_value() }
impl From<LispObject> for LispVectorRef {
    fn from(o: LispObject) -> Self {
        o.as_vector_or_error()
    }
}

impl_vectorlike_ref! { LispVectorlikeSlotsRef, LispVecSlotsIterator,
More_Lisp_Bits::PSEUDOVECTOR_SIZE_MASK as isize }

impl From<LispBoolVecRef> for LispObject {
    fn from(b: LispBoolVecRef) -> Self {
        Self::tag_ptr(b, Lisp_Type::Lisp_Vectorlike)
    }
}

impl LispBoolVecRef {
    pub fn len(self) -> usize {
        self.size as usize
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn len_bytes(self) -> usize {
        (self.len() + BOOL_VECTOR_BITS_PER_CHAR as usize - 1) / BOOL_VECTOR_BITS_PER_CHAR as usize
    }

    pub fn len_words(self) -> usize {
        (self.len() + BITS_PER_BITS_WORD as usize - 1) / BITS_PER_BITS_WORD as usize
    }

    pub fn as_slice(&self) -> &[usize] {
        let l = self.len_words();
        unsafe { self.data.as_slice(l) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [usize] {
        let l = self.len_words();
        unsafe { self.data.as_mut_slice(l) }
    }

    pub fn get(self, idx: usize) -> LispObject {
        assert!(idx < self.len());
        unsafe { self.get_unchecked(idx) }
    }

    pub unsafe fn get_unchecked(self, idx: usize) -> LispObject {
        self.iter().nth(idx).unwrap()
    }

    pub fn set(&mut self, idx: usize, b: bool) {
        assert!(idx < self.len());
        unsafe { self.set_unchecked(idx, b) }
    }

    pub fn set_checked(&mut self, idx: usize, b: bool) {
        if idx >= self.len() {
            args_out_of_range!(*self, idx);
        }

        unsafe { self.set_unchecked(idx, b) }
    }

    pub unsafe fn set_unchecked(&mut self, idx: usize, b: bool) {
        let limb = self
            .as_mut_slice()
            .get_unchecked_mut(idx / BITS_PER_BITS_WORD as usize) as *mut usize;
        if b {
            *limb |= 1 << (idx % BITS_PER_BITS_WORD as usize)
        } else {
            *limb &= !(1 << (idx % BITS_PER_BITS_WORD as usize))
        }
    }

    pub fn iter(&self) -> LispBoolVecIterator {
        LispBoolVecIterator {
            real_len: self.len(),
            bvec_slice: self.as_slice(),
            limb: 0,
            cur: 0,
        }
    }
}

impl LispStructuralEqual for LispBoolVecRef {
    fn equal(
        &self,
        other: Self,
        _kind: equal_kind::Type,
        _depth: i32,
        _ht: &mut LispHashTableRef,
    ) -> bool {
        let bits_per = BOOL_VECTOR_BITS_PER_CHAR as usize;
        // Bool vectors are compared much like strings.
        self.len() == other.len()
            && unsafe {
                libc::memcmp(
                    self.data.as_ptr() as *const libc::c_void,
                    other.data.as_ptr() as *const libc::c_void,
                    (self.len() + bits_per - 1) / bits_per,
                ) == 0
            }
    }
}

impl LispObject {
    pub fn is_bool_vector(self) -> bool {
        self.as_vectorlike()
            .map_or(false, |v| v.is_pseudovector(pvec_type::PVEC_BOOL_VECTOR))
    }

    pub fn as_bool_vector(self) -> Option<LispBoolVecRef> {
        self.as_vectorlike()
            .and_then(LispVectorlikeRef::as_bool_vector)
    }
}

pub struct LispBoolVecIterator<'a> {
    real_len: usize,
    bvec_slice: &'a [usize],
    limb: usize,
    cur: usize,
}

impl<'a> Iterator for LispBoolVecIterator<'a> {
    type Item = LispObject;

    fn next(&mut self) -> Option<LispObject> {
        if self.cur >= self.real_len {
            None
        } else {
            if self.cur % BITS_PER_BITS_WORD as usize == 0 {
                self.limb = self.bvec_slice[self.cur / BITS_PER_BITS_WORD as usize];
            }
            let res = self.limb & (1 << (self.cur % BITS_PER_BITS_WORD as usize)) != 0;

            self.cur += 1;
            Some(res.into())
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.real_len - self.cur;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for LispBoolVecIterator<'a> {}

/// Return the length of vector, list or string SEQUENCE.
/// A byte-code function object is also allowed.
/// If the string contains multibyte characters, this is not necessarily
/// the number of bytes in the string; it is the number of characters.
/// To get the number of bytes, use `string-bytes'.
#[lisp_fn]
pub fn length(sequence: LispObject) -> usize {
    if sequence.is_nil() {
        0
    } else if let Some(s) = sequence.as_string() {
        s.len_chars() as usize
    } else if let Some(vl) = sequence.as_vectorlike() {
        if let Some(v) = vl.as_vector() {
            v.len()
        } else if let Some(bv) = vl.as_bool_vector() {
            bv.len()
        } else if vl.is_pseudovector(pvec_type::PVEC_CHAR_TABLE) {
            MAX_CHAR as usize
        } else if vl.is_pseudovector(pvec_type::PVEC_COMPILED)
            || vl.is_pseudovector(pvec_type::PVEC_RECORD)
        {
            vl.pseudovector_size() as usize
        } else {
            wrong_type!(Qsequencep, sequence);
        }
    } else if let Some(cons) = sequence.as_cons() {
        cons.length()
    } else {
        wrong_type!(Qsequencep, sequence);
    }
}

/// Return element of SEQUENCE at index N.
#[lisp_fn]
pub fn elt(sequence: LispObject, n: EmacsInt) -> LispObject {
    if sequence.is_list() {
        nth(n, sequence)
    } else if sequence.is_array() {
        aref(sequence, n)
    } else {
        wrong_type!(Qsequencep, sequence);
    }
}

/// Sort SEQ, stably, comparing elements using PREDICATE.
/// Returns the sorted sequence.  SEQ should be a list or vector.  SEQ is
/// modified by side effects.  PREDICATE is called with two elements of
/// SEQ, and should return non-nil if the first element should sort before
/// the second.
#[lisp_fn]
pub fn sort(seq: LispObject, predicate: LispObject) -> LispObject {
    if seq.is_cons() {
        sort_list(seq, predicate)
    } else if let Some(mut vec) = seq.as_vectorlike().and_then(LispVectorlikeRef::as_vector) {
        vec.as_mut_slice().sort_by(|&a, &b| {
            // XXX: since the `sort' predicate is a two-outcome comparison
            // Less/!Less, and slice::sort_by() uses Greater/!Greater
            // (which is not guaranteed anyway), this requires two calls
            // instead of one in some cases.
            if !inorder(predicate, a, b) {
                Ordering::Greater
            } else if inorder(predicate, b, a) {
                Ordering::Equal
            } else {
                Ordering::Less
            }
        });
        seq
    } else if seq.is_nil() {
        seq
    } else {
        wrong_type!(Qsequencep, seq)
    }
}

/// Return t if OBJECT is a vector.
#[lisp_fn]
pub fn vectorp(object: LispObject) -> bool {
    object.is_vector()
}

/// Return t if OBJECT is a char-table.
#[lisp_fn]
pub fn char_table_p(object: LispObject) -> bool {
    object.is_char_table()
}

/// Return t if OBJECT is a char-table or vector.
#[lisp_fn]
pub fn vector_or_char_table_p(object: LispObject) -> bool {
    object.is_vector() || object.is_char_table()
}

/// Return t if OBJECT is a bool-vector.
#[lisp_fn]
pub fn bool_vector_p(object: LispObject) -> bool {
    object.is_bool_vector()
}

/// Return t if OBJECT is an array (string or vector).
#[lisp_fn]
pub fn arrayp(object: LispObject) -> bool {
    object.is_array()
}

/// Return t if OBJECT is a sequence (list or array).
#[lisp_fn]
pub fn sequencep(object: LispObject) -> bool {
    object.is_sequence()
}

/// Return t if OBJECT is an editor buffer.
#[lisp_fn]
pub fn bufferp(object: LispObject) -> bool {
    object.is_buffer()
}

/// Return t if OBJECT is a built-in function.
#[lisp_fn]
pub fn subrp(object: LispObject) -> bool {
    object.is_subr()
}

/// Return t if OBJECT is a byte-compiled function object.
#[lisp_fn]
pub fn byte_code_function_p(object: LispObject) -> bool {
    object.is_byte_code_function()
}

/// Return t if OBJECT is a thread.
#[lisp_fn]
pub fn threadp(object: LispObject) -> bool {
    object.is_thread()
}

/// Return t if OBJECT is a mutex.
#[lisp_fn]
pub fn mutexp(object: LispObject) -> bool {
    object.is_mutex()
}

/// Return t if OBJECT is a condition variable.
#[lisp_fn]
pub fn condition_variable_p(object: LispObject) -> bool {
    object.is_condition_variable()
}

/// Return t if OBJECT is a record.
#[lisp_fn]
pub fn recordp(object: LispObject) -> bool {
    object.is_record()
}

lazy_static! {
    pub static ref HEADER_SIZE: usize =
        unsafe { offset_of!(crate::remacs_sys::Lisp_Vector, contents) };
    pub static ref WORD_SIZE: usize = ::std::mem::size_of::<crate::lisp::LispObject>();
}

#[no_mangle]
pub extern "C" fn test_vectors() {
    let vec1 = unsafe { Fmake_vector(10.into(), 3.into()) };
    let vec_ref: LispVectorRef = vec1.into();
    let vec2: LispObject = vec_ref.into();

    assert_eq!(vec1, vec2);
}

include!(concat!(env!("OUT_DIR"), "/vectors_exports.rs"));
