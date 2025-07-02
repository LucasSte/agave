/// This is how a slice is represented in the VM.
/// It should be merged with VmSlice in a future refactor.
#[repr(C)]
pub(crate) struct GuestSliceReference {
    pub(crate) pointer: u64,
    pub(crate) length: u64,
}