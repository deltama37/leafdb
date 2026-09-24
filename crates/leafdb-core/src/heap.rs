//! Table heap: stores rows across a chain of slotted pages.
//!
//! This is an append-oriented heap. New rows are written to the last page in
//! the chain, allocating a new page when the current one is full. Rows are
//! addressed by a [`RowLocation`] (page id + slot index) so the executor can
//! update or delete matched rows.
//!
//! A B+Tree primary-key index is intentionally out of the initial scope
//! (ADR-0001/0002): primary-key uniqueness is currently enforced with a heap
//! scan. The page-based layout keeps the door open for adding an index later.

use crate::catalog::TableSchema;
use crate::error::Result;
use crate::page::{ensure_fits, SlottedPage};
use crate::record::{decode_row, encode_row};
use crate::storage::{PageId, Storage, PAGE_SIZE};
use crate::value::Value;

/// Physical address of a row within a table heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowLocation {
    pub page_id: PageId,
    pub slot: usize,
}

/// A row together with its physical location.
pub struct LocatedRow {
    pub location: RowLocation,
    pub values: Vec<Value>,
}

fn read_page(storage: &dyn Storage, page_id: PageId) -> Result<[u8; PAGE_SIZE]> {
    let mut buf = [0u8; PAGE_SIZE];
    storage.read_page(page_id, &mut buf)?;
    Ok(buf)
}

/// Appends a row to the table's heap, allocating pages as needed.
pub fn insert_row(
    storage: &mut dyn Storage,
    table: &mut TableSchema,
    values: &[Value],
) -> Result<RowLocation> {
    let cell = encode_row(values);
    ensure_fits(cell.len())?;

    if table.first_page == 0 {
        let page_id = storage.allocate_page()?;
        let mut buf = [0u8; PAGE_SIZE];
        let mut page = SlottedPage::format(&mut buf);
        let slot = page.insert(&cell).expect("fresh page must fit one row");
        storage.write_page(page_id, &buf)?;
        table.first_page = page_id;
        table.last_page = page_id;
        return Ok(RowLocation { page_id, slot });
    }

    let last = table.last_page;
    let mut buf = read_page(storage, last)?;
    {
        let mut page = SlottedPage::new(&mut buf);
        if let Some(slot) = page.insert(&cell) {
            storage.write_page(last, &buf)?;
            return Ok(RowLocation {
                page_id: last,
                slot,
            });
        }
    }

    // No room in the last page: allocate and link a new one.
    let new_id = storage.allocate_page()?;
    let mut new_buf = [0u8; PAGE_SIZE];
    let slot = {
        let mut page = SlottedPage::format(&mut new_buf);
        page.insert(&cell).expect("fresh page must fit one row")
    };
    storage.write_page(new_id, &new_buf)?;

    // Update the previous last page's forward pointer.
    let mut prev = read_page(storage, last)?;
    {
        let mut page = SlottedPage::new(&mut prev);
        page.set_next(new_id);
    }
    storage.write_page(last, &prev)?;
    table.last_page = new_id;

    Ok(RowLocation {
        page_id: new_id,
        slot,
    })
}

/// Scans every live row in a table, in physical order.
pub fn scan(storage: &dyn Storage, table: &TableSchema) -> Result<Vec<LocatedRow>> {
    let mut out = Vec::new();
    let mut page_id = table.first_page;
    while page_id != 0 {
        let mut buf = read_page(storage, page_id)?;
        let (num_slots, next) = {
            let page = SlottedPage::new(&mut buf);
            (page.num_slots(), page.next())
        };
        for slot in 0..num_slots {
            let page = SlottedPage::new(&mut buf);
            if let Some(cell) = page.cell(slot) {
                let values = decode_row(cell)?;
                out.push(LocatedRow {
                    location: RowLocation { page_id, slot },
                    values,
                });
            }
        }
        page_id = next;
    }
    Ok(out)
}

/// Replaces the row at `location` with new values (may relocate within page).
pub fn update_row(
    storage: &mut dyn Storage,
    location: RowLocation,
    values: &[Value],
) -> Result<()> {
    let cell = encode_row(values);
    ensure_fits(cell.len())?;
    let mut buf = read_page(storage, location.page_id)?;
    {
        let mut page = SlottedPage::new(&mut buf);
        page.update(location.slot, &cell);
    }
    storage.write_page(location.page_id, &buf)
}

/// Deletes the row at `location` (tombstone).
pub fn delete_row(storage: &mut dyn Storage, location: RowLocation) -> Result<()> {
    let mut buf = read_page(storage, location.page_id)?;
    {
        let mut page = SlottedPage::new(&mut buf);
        page.delete(location.slot);
    }
    storage.write_page(location.page_id, &buf)
}
