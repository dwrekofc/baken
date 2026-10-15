//! Raw MP3 frame locations through pinned Symphonia header/metadata readers.
//! The imported MPL-2.0 files keep their license and exact source provenance.
#[allow(dead_code)]
mod common;
#[allow(dead_code)]
mod header;
use anyhow::{bail, Context, Result};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
use symphonia::core::{
    io::{MediaSourceStream, ReadBytes},
    meta::{MetadataOptions, MetadataReader},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Absolute file position; metadata frames such as Xing/Info stay included.
    pub offset: u64,
    pub bytes: u32,
    pub bitrate: u32,
    pub header: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameIndex {
    pub data_start: u64,
    pub data_end: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples_per_frame: u32,
    pub first_bytes: Vec<u8>,
    pub frames: Vec<Frame>,
}
fn compatible(a: u32, b: u32) -> bool {
    (a ^ b) & 0xfffe_0ccf == 0
}
fn parse(word: u32) -> Result<common::FrameHeader> {
    if !header::is_frame_header_word_synced(word) {
        bail!("MP3 frame is not synchronized");
    }
    let value = header::parse_frame_header(word)?;
    if value.layer != common::MpegLayer::Layer3 || !(24..=1441).contains(&(value.frame_size + 4)) {
        bail!("MP3 index requires a supported Layer 3 frame");
    }
    Ok(value)
}
/// Read contiguous Layer 3 frames after leading ID3v2 metadata and before
/// trailing ID3v1. Other discontinuities refuse rather than invent locations.
pub fn frame_index(path: &Path) -> Result<FrameIndex> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let mut end = length;
    if length > 128 {
        file.seek(SeekFrom::End(-128))?;
        let mut tag = [0; 3];
        file.read_exact(&mut tag)?;
        if &tag == b"TAG" {
            end -= 128;
        }
    }
    file.seek(SeekFrom::Start(0))?;
    let mut reader = MediaSourceStream::new(Box::new(file), Default::default());
    loop {
        let at = reader.pos();
        let mut tag = [0; 3];
        ReadBytes::read_buf_exact(&mut reader, &mut tag)?;
        reader.seek(SeekFrom::Start(at))?;
        if &tag != b"ID3" {
            break;
        }
        let mut metadata = Box::new(symphonia::default::meta::Id3v2Reader::try_new(
            reader,
            MetadataOptions::default(),
        )?);
        metadata.read_all()?;
        reader = metadata.into_inner();
    }
    let scan_start = reader.pos();
    // The original also requires three compatible consecutive initial frames.
    let (start, first) = loop {
        let word = header::sync_frame(&mut reader)?;
        let start = reader.pos() - 4;
        if start.saturating_sub(scan_start) > 0x80000 {
            bail!("MP3 synchronization exceeds the search bound");
        }
        let Ok(first) = parse(word) else { continue };
        let mut next = start + 4 + first.frame_size as u64;
        let mut valid = true;
        for _ in 0..2 {
            if next + 4 > end {
                valid = false;
                break;
            }
            reader.seek(SeekFrom::Start(next))?;
            let candidate = reader.read_be_u32()?;
            match parse(candidate) {
                Ok(frame) if compatible(word, candidate) => next += 4 + frame.frame_size as u64,
                _ => {
                    valid = false;
                    break;
                }
            }
        }
        if valid && next <= end {
            break (start, word);
        }
        reader.seek(SeekFrom::Start(start + 1))?;
    };
    let initial = parse(first)?;
    reader.seek(SeekFrom::Start(start))?;
    let mut frames = Vec::new();
    while reader.pos() < end {
        let offset = reader.pos();
        if end - offset < 4 {
            bail!("MP3 has unsupported trailing bytes");
        }
        let word = reader.read_be_u32()?;
        let h = parse(word)?;
        if !compatible(first, word) {
            bail!("MP3 frame profile changed");
        }
        let bytes = u32::try_from(h.frame_size + 4).context("MP3 frame size overflow")?;
        if offset + u64::from(bytes) > end {
            bail!("MP3 frame is truncated");
        }
        frames.push(Frame {
            offset,
            bytes,
            bitrate: h.bitrate,
            header: word,
        });
        reader.ignore_bytes(h.frame_size as u64)?;
    }
    reader.seek(SeekFrom::Start(start))?;
    let mut prefix = vec![0; (end - start).min(64) as usize];
    ReadBytes::read_buf_exact(&mut reader, &mut prefix)?;
    Ok(FrameIndex {
        data_start: start,
        data_end: end,
        sample_rate: initial.sample_rate,
        channels: initial.n_channels() as u16,
        samples_per_frame: u32::from(initial.num_frames()),
        first_bytes: prefix,
        frames,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn bytes(words: &[u32]) -> Vec<u8> {
        let mut out = b"ID3\x03\0\0\0\0\0\0".to_vec();
        for &word in words {
            let len = if word & 0xf000 == 0x9000 { 417 } else { 522 };
            out.extend(word.to_be_bytes());
            out.resize(out.len() + len - 4, 0);
        }
        out.extend(b"TAG");
        out.extend([0; 125]);
        out
    }
    #[test]
    fn imported_headers_preserve_all_metadata_frames_offsets_and_variable_bitrates() {
        let path =
            std::env::temp_dir().join(format!("baken-mpeg-index-{}.mp3", std::process::id()));
        std::fs::write(&path, bytes(&[0xfffb9000, 0xfffba000, 0xfffb9000])).unwrap();
        let index = frame_index(&path).unwrap();
        assert_eq!(
            (
                index.data_start,
                index.data_end,
                index.sample_rate,
                index.channels,
                index.samples_per_frame
            ),
            (10, 1366, 44100, 2, 1152)
        );
        assert_eq!(
            index
                .frames
                .iter()
                .map(|f| (f.offset, f.bytes, f.bitrate))
                .collect::<Vec<_>>(),
            [(10, 417, 128000), (427, 522, 160000), (949, 417, 128000)]
        );
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn incomplete_or_incompatible_initial_frames_refuse() {
        let path = std::env::temp_dir().join(format!("baken-mpeg-bad-{}.mp3", std::process::id()));
        for words in [
            &[0xfffb9000, 0xfffb9000][..],
            &[0xfffb9000, 0xfffb9004, 0xfffb9000][..],
        ] {
            std::fs::write(&path, bytes(words)).unwrap();
            assert!(frame_index(&path).is_err());
        }
        std::fs::remove_file(path).unwrap();
    }
}
