//! Storage abstraction (ADR-0002).
//!
//! The engine never touches OPFS or the host filesystem directly. Instead it
//! reads and writes fixed-size pages through the [`Storage`] trait. The initial
//! implementation is [`MemoryStorage`], a page buffer held entirely in WASM
//! linear memory. Browser persistence is layered on top by exporting/importing
//! the raw page image to OPFS from JavaScript (see the `web/` demo and
//! [`crate::Database::export_image`]).

use crate::error::{Error, Result};

/// Fixed page size in bytes. 4 KiB is a common default that maps well to OPFS
/// block I/O; the concrete value is intentionally left to measurement in
/// ADR-0002 and can change without affecting callers above the pager.
pub const PAGE_SIZE: usize = 4096;

/// Identifier of a page within a database image.
pub type PageId = u32;

/// A block device abstraction: a linear array of fixed-size pages.
pub trait Storage {
    /// Number of pages currently allocated.
    fn page_count(&self) -> u32;

    /// Reads a full page into `buf` (which must be [`PAGE_SIZE`] long).
    fn read_page(&self, page_id: PageId, buf: &mut [u8]) -> Result<()>;

    /// Writes a full page from `buf` (which must be [`PAGE_SIZE`] long).
    fn write_page(&mut self, page_id: PageId, buf: &[u8]) -> Result<()>;

    /// Allocates a new zeroed page and returns its id.
    fn allocate_page(&mut self) -> Result<PageId>;
}

/// In-memory page store backed by a contiguous byte image.
///
/// The whole database is a `Vec<u8>` whose length is a multiple of
/// [`PAGE_SIZE`]. This makes persistence trivial: the byte image can be handed
/// to OPFS as-is and reloaded later.
#[derive(Clone, Default)]
pub struct MemoryStorage {
    data: Vec<u8>,
}

impl MemoryStorage {
    /// Creates an empty store with zero pages.
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    /// Rebuilds a store from a previously exported image.
    pub fn from_image(image: Vec<u8>) -> Result<Self> {
        if image.len() % PAGE_SIZE != 0 {
            return Err(Error::Storage(format!(
                "image length {} is not a multiple of page size {PAGE_SIZE}",
                image.len()
            )));
        }
        Ok(Self { data: image })
    }

    /// Returns a copy of the raw page image for persistence.
    pub fn image(&self) -> Vec<u8> {
        self.data.clone()
    }

    fn range(&self, page_id: PageId) -> Result<core::ops::Range<usize>> {
        let start = page_id as usize * PAGE_SIZE;
        let end = start + PAGE_SIZE;
        if end > self.data.len() {
            return Err(Error::Storage(format!(
                "page {page_id} out of range (page_count={})",
                self.page_count()
            )));
        }
        Ok(start..end)
    }
}

impl Storage for MemoryStorage {
    fn page_count(&self) -> u32 {
        (self.data.len() / PAGE_SIZE) as u32
    }

    fn read_page(&self, page_id: PageId, buf: &mut [u8]) -> Result<()> {
        if buf.len() != PAGE_SIZE {
            return Err(Error::Storage("read buffer must be PAGE_SIZE".into()));
        }
        let range = self.range(page_id)?;
        buf.copy_from_slice(&self.data[range]);
        Ok(())
    }

    fn write_page(&mut self, page_id: PageId, buf: &[u8]) -> Result<()> {
        if buf.len() != PAGE_SIZE {
            return Err(Error::Storage("write buffer must be PAGE_SIZE".into()));
        }
        let range = self.range(page_id)?;
        self.data[range].copy_from_slice(buf);
        Ok(())
    }

    fn allocate_page(&mut self) -> Result<PageId> {
        let id = self.page_count();
        self.data.extend(core::iter::repeat(0u8).take(PAGE_SIZE));
        Ok(id)
    }
}
