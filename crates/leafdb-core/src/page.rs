//! Slotted-page layout used for table heap pages.
//!
//! Each data page holds a small header, a slot directory that grows forward
//! from the header, and variable-length cells (serialized rows) that grow
//! backward from the end of the page. This is the classic layout used by many
//! RDBMS storage engines and supports variable-length records and free-space
//! management within a page (ADR-0002).
//!
//! ```text
//! 0        8                              free_end            PAGE_SIZE
//! ┌────────┬───────────────┬──────────────┬────────────────────────────┐
//! │ header │ slot dir  →   │  free space  │   ←  cells (rows)           │
//! └────────┴───────────────┴──────────────┴────────────────────────────┘
//! ```

use crate::error::{Error, Result};
use crate::storage::{PageId, PAGE_SIZE};

const HEADER_SIZE: usize = 8;
const SLOT_SIZE: usize = 4;

/// A view over a single page buffer providing slotted-page operations.
pub struct SlottedPage<'a> {
    buf: &'a mut [u8],
}

impl<'a> SlottedPage<'a> {
    /// Wraps an existing page buffer without modifying it.
    pub fn new(buf: &'a mut [u8]) -> Self {
        debug_assert_eq!(buf.len(), PAGE_SIZE);
        Self { buf }
    }

    /// Initializes a fresh, empty page in place.
    pub fn format(buf: &'a mut [u8]) -> Self {
        debug_assert_eq!(buf.len(), PAGE_SIZE);
        for b in buf.iter_mut() {
            *b = 0;
        }
        let mut page = Self { buf };
        page.set_next(0);
        page.set_num_slots(0);
        page.set_free_end(PAGE_SIZE as u16);
        page
    }

    fn read_u16(&self, off: usize) -> u16 {
        u16::from_le_bytes([self.buf[off], self.buf[off + 1]])
    }

    fn read_u32(&self, off: usize) -> u32 {
        u32::from_le_bytes([
            self.buf[off],
            self.buf[off + 1],
            self.buf[off + 2],
            self.buf[off + 3],
        ])
    }

    fn write_u16(&mut self, off: usize, v: u16) {
        self.buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
    }

    fn write_u32(&mut self, off: usize, v: u32) {
        self.buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// Id of the next page in this table's chain (0 == none).
    pub fn next(&self) -> PageId {
        self.read_u32(0)
    }

    pub fn set_next(&mut self, id: PageId) {
        self.write_u32(0, id);
    }

    pub fn num_slots(&self) -> usize {
        self.read_u16(4) as usize
    }

    fn set_num_slots(&mut self, n: u16) {
        self.write_u16(4, n);
    }

    fn free_end(&self) -> usize {
        self.read_u16(6) as usize
    }

    fn set_free_end(&mut self, v: u16) {
        self.write_u16(6, v);
    }

    fn slot_dir_end(&self) -> usize {
        HEADER_SIZE + self.num_slots() * SLOT_SIZE
    }

    /// Bytes available for one more cell (accounting for its slot entry).
    pub fn free_space(&self) -> usize {
        self.free_end().saturating_sub(self.slot_dir_end())
    }

    fn slot(&self, idx: usize) -> (usize, usize) {
        let base = HEADER_SIZE + idx * SLOT_SIZE;
        let off = self.read_u16(base) as usize;
        let len = self.read_u16(base + 2) as usize;
        (off, len)
    }

    fn set_slot(&mut self, idx: usize, off: u16, len: u16) {
        let base = HEADER_SIZE + idx * SLOT_SIZE;
        self.write_u16(base, off);
        self.write_u16(base + 2, len);
    }

    /// Attempts to append a cell. Returns the slot index on success, or `None`
    /// if the page does not have enough free space.
    pub fn insert(&mut self, cell: &[u8]) -> Option<usize> {
        let need = cell.len() + SLOT_SIZE;
        if need > self.free_space() {
            return None;
        }
        let new_free_end = self.free_end() - cell.len();
        self.buf[new_free_end..new_free_end + cell.len()].copy_from_slice(cell);
        let idx = self.num_slots();
        self.set_slot(idx, new_free_end as u16, cell.len() as u16);
        self.set_num_slots((idx + 1) as u16);
        self.set_free_end(new_free_end as u16);
        Some(idx)
    }

    /// Returns the bytes of the cell in `idx`, or `None` if it was deleted.
    pub fn cell(&self, idx: usize) -> Option<&[u8]> {
        if idx >= self.num_slots() {
            return None;
        }
        let (off, len) = self.slot(idx);
        if off == 0 {
            return None; // tombstone
        }
        Some(&self.buf[off..off + len])
    }

    /// Overwrites a cell in place when the new bytes are the same length,
    /// otherwise marks the old cell deleted and appends the new one.
    ///
    /// Returns the (possibly new) slot index, or `None` if there was no room to
    /// relocate a larger record.
    pub fn update(&mut self, idx: usize, cell: &[u8]) -> Option<usize> {
        let (off, len) = self.slot(idx);
        if off != 0 && len == cell.len() {
            self.buf[off..off + len].copy_from_slice(cell);
            return Some(idx);
        }
        self.delete(idx);
        self.insert(cell)
    }

    /// Marks a slot as deleted (tombstone). Space is not compacted; it is
    /// reclaimed only when the page is rewritten.
    pub fn delete(&mut self, idx: usize) {
        if idx < self.num_slots() {
            self.set_slot(idx, 0, 0);
        }
    }
}

/// Guard that a serialized row fits within a page's usable area.
pub fn max_cell_size() -> usize {
    PAGE_SIZE - HEADER_SIZE - SLOT_SIZE
}

/// Ensures `len` can ever fit in a page, returning a descriptive error if not.
pub fn ensure_fits(len: usize) -> Result<()> {
    if len > max_cell_size() {
        return Err(Error::Storage(format!(
            "record of {len} bytes exceeds maximum cell size {}",
            max_cell_size()
        )));
    }
    Ok(())
}
