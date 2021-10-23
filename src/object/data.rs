use std::fmt;
use std::marker::PhantomData;
use std::ops::{Deref, Not};

/// The inner data type that hold the value for an object variant. This type
/// should be no larger then 56 bits. The lowest bit of data is used to encode
/// the mutability flag: 1 if immutable, 0 if mutable. This should be stored in
/// the alignment bits that bottom of the pointer.
#[derive(Copy, Clone)]
pub(crate) struct Data<T> {
    data: [u8; 7],
    marker: PhantomData<T>,
}
pub(super) const UNUSED: Data<()> = Data::from_raw(0);

/// A trait to access the inner value of a [`Data`]
pub(crate) trait Inner<T> {
    fn inner(self) -> T;
}

// We still need to determine when this is sound. Sending `Data<T>` across threads
// is not safe unless the values are copied with it. Maybe there is a better way
// to encode that in the type system.
unsafe impl<T> Send for Data<T> {}

impl<T> Data<T> {
    #[inline(always)]
    const fn into_raw(self) -> i64 {
        let data = self.data;
        // This operation will take the 56 bit data and left shift it so that
        // the bottom byte is zeroed.
        let whole = [
            0, data[0], data[1], data[2], data[3], data[4], data[5], data[6],
        ];
        // We shift it back down so that original value is reconstructed.
        i64::from_le_bytes(whole) >> 8
    }

    #[inline(always)]
    const fn from_raw(data: i64) -> Self {
        let bytes = data.to_le_bytes();
        // Notice bytes[7] is missing. That is the top byte that is removed.
        Data {
            data: [
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6],
            ],
            marker: PhantomData,
        }
    }
}

impl<'a, T> Data<&'a T> {
    /// mark the pointer as immutable. This is done by shifting
    /// the value and setting the LSB to 1.
    fn immut_bit_pattern(ptr: *const T) -> i64 {
        ((ptr as i64) << 1) | 0x1
    }

    fn mut_bit_pattern(ptr: *const T) -> i64 {
        (ptr as i64) << 1
    }

    #[inline(always)]
    pub(super) fn from_ref(rf: &'a T) -> Self {
        let ptr: *const T = rf;
        let bits = Self::immut_bit_pattern(ptr);
        Self::from_raw(bits)
    }

    #[inline(always)]
    pub(super) fn from_mut_ref(rf: &'a mut T) -> Self {
        let ptr: *mut T = rf;
        let bits = Self::mut_bit_pattern(ptr);
        Self::from_raw(bits)
    }

    #[inline(always)]
    pub(crate) fn inner_mut(self) -> Option<&'a mut T> {
        let bits = self.into_raw();
        let mutable = (bits & 0x1) == 0;
        if mutable {
            let ptr = (bits >> 1) as *mut T;
            unsafe { Some(&mut *ptr) }
        } else {
            None
        }
    }

    pub(super) fn make_read_only(&mut self) {
        self.data[0] |= 0x1;
    }
}

impl<'a, T> Inner<&'a T> for Data<&'a T> {
    #[inline(always)]
    fn inner(self) -> &'a T {
        // shift by 1 to remove the mutability flag
        let bits = self.into_raw() >> 1;
        let ptr = bits as *const T;
        unsafe { &*ptr }
    }
}

impl Inner<i64> for Data<i64> {
    #[inline(always)]
    fn inner(self) -> i64 {
        self.into_raw()
    }
}

impl Data<i64> {
    pub(super) fn from_int(data: i64) -> Self {
        Data::from_raw(data)
    }
}

impl<T> Not for Data<T>
where
    Data<T>: Inner<T>,
{
    type Output = T;

    #[inline(always)]
    fn not(self) -> Self::Output {
        self.inner()
    }
}

impl<T> PartialEq for Data<T>
where
    T: PartialEq + Copy,
    Data<T>: Inner<T>,
{
    fn eq(&self, other: &Self) -> bool {
        self.inner() == other.inner()
    }
}

impl PartialEq for Data<()> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T> fmt::Display for Data<T>
where
    T: fmt::Display + Copy,
    Data<T>: Inner<T>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner(), f)
    }
}

impl<T> fmt::Debug for Data<T>
where
    T: fmt::Debug + Copy,
    Data<T>: Inner<T>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner(), f)
    }
}

impl<'a, T> Deref for Data<&'a T> {
    type Target = T;

    #[inline(always)]
    fn deref(&'_ self) -> &'a Self::Target {
        self.inner()
    }
}

impl<'a, T> AsRef<T> for Data<&'a T> {
    #[inline(always)]
    fn as_ref<'b>(&'b self) -> &'a T {
        self.inner()
    }
}