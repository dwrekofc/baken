//! Reusable DeviceSQL table assembly extracted from the established classic writer.
//! Row encoders receive their final position on each page, including after rollover.
use super::page::{header_page, index_page, DataPage, HeaderStyle, PAGE_LEN};

/// Fallible row encoder, called again at position zero if a page rolls over.
pub type Row<'a> = Box<dyn Fn(usize) -> Result<Vec<u8>, WriteError> + 'a>;

/// One typed table; format-specific row meanings belong to the caller.
pub struct Table<'a> {
    pub ty: u32,
    pub style: HeaderStyle,
    /// Fixed sequence when required by a table, otherwise allocated monotonically.
    pub seq: Option<u32>,
    pub rows: Vec<Row<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    #[error("unknown DeviceSQL table {0}")]
    UnknownTable(u32),
    #[error("DeviceSQL table {0} is already populated")]
    PopulatedTable(u32),
    #[error("table inventory exceeds the header page")]
    TooManyTables,
    #[error("duplicate table type")]
    DuplicateTable,
    #[error("row larger than a DeviceSQL page")]
    RowTooLarge,
    #[error("DeviceSQL page index limit exceeded")]
    TooManyPages,
    #[error("DeviceSQL sequence overflow")]
    SequenceOverflow,
    #[error("row encoding failed: {0}")]
    Encode(String),
}

/// Build a complete header, index/data chains and empty candidates.
pub fn write_tables(tables: &[Table<'_>]) -> Result<Vec<u8>, WriteError> {
    if tables.len() > (PAGE_LEN - 0x1c) / 16 {
        return Err(WriteError::TooManyTables);
    }
    let mut types = std::collections::BTreeSet::new();
    if tables.iter().any(|t| !types.insert(t.ty)) {
        return Err(WriteError::DuplicateTable);
    }
    let num_tables = tables.len() as u32;
    let mut pages: Vec<Vec<u8>> = vec![Vec::new()]; // page 0 filled last
    let mut pointers = Vec::new();
    let mut next_seq = 10u32;
    let mut max_seq = 3u32;
    // (table index, index page position, data pages) resolved once empties are known
    let mut pending: Vec<(u32, usize, Vec<DataPage>)> = Vec::new();

    for t in tables {
        let index_pos = pages.len();
        pages.push(Vec::new());
        let mut data: Vec<DataPage> = Vec::new();
        let mut page = DataPage::new(pages.len() as u32, t.ty, 0, t.style);
        for make in &t.rows {
            let row = make(page.rows())?;
            if !page.try_push(&row) {
                data.push(page);
                page = DataPage::new((pages.len() + data.len()) as u32, t.ty, 0, t.style);
                let row = make(0)?;
                if !page.try_push(&row) {
                    return Err(WriteError::RowTooLarge);
                }
            }
        }
        if !page.is_empty() {
            data.push(page);
        }
        if pages
            .len()
            .checked_add(data.len())
            .and_then(|n| n.checked_add(tables.len()))
            .is_none_or(|n| n >= 0x03ff_ffff)
        {
            return Err(WriteError::TooManyPages);
        }
        for p in &mut data {
            p.seq = match t.seq {
                Some(s) => s,
                None => {
                    next_seq = next_seq
                        .checked_add(1)
                        .ok_or(WriteError::SequenceOverflow)?;
                    next_seq
                }
            };
            max_seq = max_seq.max(p.seq);
            pages.push(Vec::new());
        }
        pending.push((t.ty, index_pos, data));
    }

    let first_empty = pages.len() as u32;
    for (i, (ty, index_pos, mut data)) in pending.into_iter().enumerate() {
        let empty = first_empty + i as u32;
        let first_data = data.first().map(|p| p.index);
        let last = data.last().map(|p| p.index).unwrap_or(index_pos as u32);
        pointers.push((ty, empty, index_pos as u32, last));
        pages[index_pos] = index_page(
            index_pos as u32,
            ty,
            first_data.unwrap_or(empty),
            first_data,
        );
        let n = data.len();
        for (k, p) in data.iter_mut().enumerate() {
            p.next = if k + 1 < n { p.index + 1 } else { empty };
            pages[p.index as usize] = p.to_bytes();
        }
    }
    for _ in 0..num_tables {
        pages.push(vec![0u8; PAGE_LEN]);
    }
    pages[0] = header_page(
        num_tables,
        pages.len() as u32,
        max_seq.checked_add(1).ok_or(WriteError::SequenceOverflow)?,
        &pointers,
    );
    Ok(pages.concat())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn table(ty: u32) -> Table<'static> {
        Table {
            ty,
            style: HeaderStyle::Normal,
            seq: None,
            rows: vec![],
        }
    }
    #[test]
    fn invalid_tables_and_rows_return_errors() {
        assert_eq!(
            write_tables(&[table(3), table(3)]),
            Err(WriteError::DuplicateTable)
        );
        let many = (0..255).map(table).collect::<Vec<_>>();
        assert_eq!(write_tables(&many), Err(WriteError::TooManyTables));
        let mut large = table(3);
        large.rows.push(Box::new(|_| Ok(vec![0; 4096])));
        assert_eq!(write_tables(&[large]), Err(WriteError::RowTooLarge));
        let mut error = table(3);
        error
            .rows
            .push(Box::new(|_| Err(WriteError::Encode("invalid".into()))));
        assert_eq!(
            write_tables(&[error]),
            Err(WriteError::Encode("invalid".into()))
        );
        let mut seq = table(3);
        seq.seq = Some(u32::MAX);
        seq.rows.push(Box::new(|_| Ok(vec![0; 16])));
        assert_eq!(write_tables(&[seq]), Err(WriteError::SequenceOverflow));
    }
    #[test]
    fn rollover_reencodes_positions_and_preserves_fixed_style() {
        let mut t = table(3);
        t.style = HeaderStyle::Fixed;
        for _ in 0..100 {
            t.rows.push(Box::new(|position| {
                let mut row = vec![0; 128];
                row[..4].copy_from_slice(&(position as u32).to_le_bytes());
                Ok(row)
            }));
        }
        let output = write_tables(&[t]).unwrap();
        let mut total = 0;
        let mut data_pages = 0;
        for page in output.chunks(PAGE_LEN).skip(2) {
            if page[0x1b] != 0x24 {
                continue;
            }
            data_pages += 1;
            let count = u16::from_le_bytes(page[0x20..0x22].try_into().unwrap()) as usize;
            assert_eq!(&page[0x22..0x24], &[0, 0]);
            for position in 0..count {
                let offset = 0x28 + 128 * position;
                assert_eq!(
                    u32::from_le_bytes(page[offset..offset + 4].try_into().unwrap()),
                    position as u32
                );
            }
            total += count;
        }
        assert_eq!(total, 100);
        assert!(data_pages > 1);
    }
}
