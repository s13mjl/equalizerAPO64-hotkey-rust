//! Port of `helpers/MemoryHelper.{h,cpp}`.
//!
//! The C++ helper allocates `size + 16` bytes, returns a 16 byte aligned
//! pointer and stores the offset to the real allocation in the byte directly
//! before the returned pointer.  This port uses `HeapAlloc`/`HeapFree` on the
//! process heap, which is the non-WINDDK branch of the original and avoids a
//! dependency on the C runtime allocator.

use std::ffi::c_void;

use windows::Win32::System::Memory::{GetProcessHeap, HeapAlloc, HeapFree, HEAP_FLAGS};

/// Alignment used by the original implementation.
pub const MEMORY_ALIGNMENT: usize = 16;

/// Port of `MemoryHelper::alloc`.  Returns null on failure.
pub fn alloc(size: usize) -> *mut u8 {
    let total = size + MEMORY_ALIGNMENT;
    let Ok(heap) = (unsafe { GetProcessHeap() }) else {
        crate::log_helper::log_error_static("GetProcessHeap failed.");
        return std::ptr::null_mut();
    };

    let memory = unsafe { HeapAlloc(heap, HEAP_FLAGS(0), total) } as *mut u8;
    if memory.is_null() {
        crate::log_helper::log_error_static(&format!("Allocation of {} bytes failed.", total));
        return std::ptr::null_mut();
    }

    // Same arithmetic as the C++ code: an already aligned block gets an
    // offset of 16, leaving one spare byte for the bookkeeping byte.
    let offset = MEMORY_ALIGNMENT - (memory as usize % MEMORY_ALIGNMENT);
    unsafe {
        let ptr = memory.add(offset);
        *ptr.sub(1) = offset as u8;
        ptr
    }
}

/// Port of `MemoryHelper::free`.
///
/// # Safety
/// `ptr` must have been returned by [`alloc`] and must not have been freed
/// before.
pub unsafe fn free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }

    let offset = unsafe { *ptr.sub(1) as usize };
    let memory = unsafe { ptr.sub(offset) };
    if let Ok(heap) = unsafe { GetProcessHeap() } {
        unsafe {
            let _ = HeapFree(heap, HEAP_FLAGS(0), Some(memory as *const c_void));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_is_16_byte_aligned() {
        for size in [1usize, 7, 16, 17, 64, 1000, 4097] {
            let ptr = alloc(size);
            assert!(!ptr.is_null());
            assert_eq!(ptr as usize % MEMORY_ALIGNMENT, 0, "size {}", size);
            unsafe {
                for i in 0..size {
                    *ptr.add(i) = (i & 0xFF) as u8;
                }
                for i in 0..size {
                    assert_eq!(*ptr.add(i), (i & 0xFF) as u8);
                }
                free(ptr);
            }
        }
    }

    #[test]
    fn null_free_is_a_no_op() {
        unsafe { free(std::ptr::null_mut()) };
    }
}
