#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct StringHandle(u32);

impl StringHandle {
    /// Creates a handle from a raw slot index.
    pub fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the raw slot index represented by this handle.
    pub fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringHeapError {
    InvalidHandle(StringHandle),
    RefCountOverflow(StringHandle),
    InvalidUtf8(StringHandle),
}

impl std::fmt::Display for StringHeapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StringHeapError::InvalidHandle(h) => write!(f, "invalid string handle: {}", h.raw()),
            StringHeapError::RefCountOverflow(h) => {
                write!(f, "string refcount overflow at handle {}", h.raw())
            }
            StringHeapError::InvalidUtf8(h) => {
                write!(f, "string at handle {} is not valid UTF-8", h.raw())
            }
        }
    }
}

impl std::error::Error for StringHeapError {}

#[derive(Debug, Clone)]
struct StringObject {
    ref_count: u32,
    bytes: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct StringHeap {
    slots: Vec<Option<StringObject>>,
    free_list: Vec<u32>,
}

impl StringHeap {
    /// Creates an empty string heap.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocates a UTF-8 string and returns its handle.
    pub fn alloc_str(&mut self, value: &str) -> StringHandle {
        self.alloc_bytes(value.as_bytes().to_vec())
    }

    /// Allocates raw bytes and returns their handle.
    pub fn alloc_bytes(&mut self, bytes: Vec<u8>) -> StringHandle {
        let idx = if let Some(reuse) = self.free_list.pop() {
            self.slots[reuse as usize] = Some(StringObject {
                ref_count: 1,
                bytes,
            });
            reuse
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(Some(StringObject {
                ref_count: 1,
                bytes,
            }));
            idx
        };
        StringHandle::new(idx)
    }

    /// Increments the reference count of a handle.
    pub fn retain(&mut self, handle: StringHandle) -> Result<(), StringHeapError> {
        let obj = self
            .get_slot_mut(handle)
            .ok_or(StringHeapError::InvalidHandle(handle))?;
        if obj.ref_count == u32::MAX {
            return Err(StringHeapError::RefCountOverflow(handle));
        }
        obj.ref_count += 1;
        Ok(())
    }

    /// Decrements the reference count and frees storage at zero.
    pub fn release(&mut self, handle: StringHandle) -> Result<(), StringHeapError> {
        let obj = self
            .get_slot_mut(handle)
            .ok_or(StringHeapError::InvalidHandle(handle))?;

        if obj.ref_count > 1 {
            obj.ref_count -= 1;
            return Ok(());
        }

        self.slots[handle.raw() as usize] = None;
        self.free_list.push(handle.raw());
        Ok(())
    }

    /// Returns the byte length of the referenced string.
    pub fn len(&self, handle: StringHandle) -> Result<usize, StringHeapError> {
        let obj = self
            .get_slot(handle)
            .ok_or(StringHeapError::InvalidHandle(handle))?;
        Ok(obj.bytes.len())
    }

    /// Returns the current reference count of the handle.
    pub fn ref_count(&self, handle: StringHandle) -> Result<u32, StringHeapError> {
        let obj = self
            .get_slot(handle)
            .ok_or(StringHeapError::InvalidHandle(handle))?;
        Ok(obj.ref_count)
    }

    /// Returns the raw bytes for a handle.
    pub fn bytes(&self, handle: StringHandle) -> Result<&[u8], StringHeapError> {
        let obj = self
            .get_slot(handle)
            .ok_or(StringHeapError::InvalidHandle(handle))?;
        Ok(&obj.bytes)
    }

    /// Decodes the referenced bytes as UTF-8.
    pub fn to_utf8_string(&self, handle: StringHandle) -> Result<String, StringHeapError> {
        let obj = self
            .get_slot(handle)
            .ok_or(StringHeapError::InvalidHandle(handle))?;
        let s =
            std::str::from_utf8(&obj.bytes).map_err(|_| StringHeapError::InvalidUtf8(handle))?;
        Ok(s.to_owned())
    }

    /// Concatenates two handles and returns a newly allocated handle.
    pub fn concat(
        &mut self,
        lhs: StringHandle,
        rhs: StringHandle,
    ) -> Result<StringHandle, StringHeapError> {
        let out = {
            let left = self.bytes(lhs)?;
            let right = self.bytes(rhs)?;
            let mut out = Vec::with_capacity(left.len() + right.len());
            out.extend_from_slice(left);
            out.extend_from_slice(right);
            out
        };
        Ok(self.alloc_bytes(out))
    }

    fn get_slot(&self, handle: StringHandle) -> Option<&StringObject> {
        self.slots.get(handle.raw() as usize)?.as_ref()
    }

    fn get_slot_mut(&mut self, handle: StringHandle) -> Option<&mut StringObject> {
        self.slots.get_mut(handle.raw() as usize)?.as_mut()
    }
}
