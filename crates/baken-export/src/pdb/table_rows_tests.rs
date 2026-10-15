use super::*;
use crate::pdb::tables::WriteError;

fn model() -> Export {
    Export {
        tracks: vec![TrackRow {
            id: 7,
            title: "Track 夜".into(),
            file_path: "/Contents/track.flac".into(),
            ..Default::default()
        }],
        artists: vec![(3, "Artist café".into())],
        artworks: vec![(11, "/PIONEER/Artwork/image.jpg".into())],
        playlists: vec![ExportPlaylist {
            id: 19,
            parent: 0,
            sort_order: 1,
            is_folder: false,
            name: "Playlist".into(),
            track_ids: vec![7, 7],
        }],
        ..Default::default()
    }
}
fn addition(ty: u32) -> TableRows<'static> {
    TableRows {
        ty,
        rows: vec![Box::new(|_| Ok(vec![1; 16]))],
    }
}
#[test]
fn no_additions_are_byte_exact_and_existing_model_rows_cannot_be_replaced() {
    let model = model();
    assert_eq!(
        write_with_empty_table_rows(&model, vec![]).unwrap(),
        write(&model)
    );
    for ty in [0, 2, 7, 8, 13, 16, 17, 18, 19] {
        assert_eq!(
            write_with_empty_table_rows(&model, vec![addition(ty)]),
            Err(WriteError::PopulatedTable(ty))
        );
    }
    assert_eq!(
        write_with_empty_table_rows(&model, vec![addition(99)]),
        Err(WriteError::UnknownTable(99))
    );
    assert_eq!(
        write_with_empty_table_rows(&model, vec![addition(9), addition(9)]),
        Err(WriteError::DuplicateTable)
    );
}
#[test]
fn added_rows_keep_twenty_tables_and_use_existing_fallible_page_rollover() {
    let model = model();
    let mut rows = addition(15);
    rows.rows.clear();
    for _ in 0..200 {
        rows.rows.push(Box::new(|position| {
            let mut row = vec![0; 128];
            row[..4].copy_from_slice(&(position as u32).to_le_bytes());
            Ok(row)
        }));
    }
    let output = write_with_empty_table_rows(&model, vec![rows]).unwrap();
    assert_eq!(
        u32::from_le_bytes(output[8..12].try_into().unwrap()),
        NUM_TABLES
    );
    let mut count = 0;
    for page in output.chunks(4096) {
        if page[8..12] != 15u32.to_le_bytes() || page[27] != 0x24 {
            continue;
        }
        let n = usize::from(page[24]) | (usize::from(page[25] & 0x1f) << 8);
        for position in 0..n {
            let at = 40 + position * 128;
            assert_eq!(&page[at..at + 4], &(position as u32).to_le_bytes());
        }
        count += n;
    }
    assert_eq!(count, 200);
    let oversized = TableRows {
        ty: 15,
        rows: vec![Box::new(|_| Ok(vec![0; 4096]))],
    };
    assert_eq!(
        write_with_empty_table_rows(&model, vec![oversized]),
        Err(WriteError::RowTooLarge)
    );
    let failed = TableRows {
        ty: 15,
        rows: vec![Box::new(|_| Err(WriteError::Encode("cancelled".into())))],
    };
    assert_eq!(
        write_with_empty_table_rows(&model, vec![failed]),
        Err(WriteError::Encode("cancelled".into()))
    );
}
