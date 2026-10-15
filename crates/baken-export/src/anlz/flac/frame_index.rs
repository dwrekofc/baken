//! Complete frame locations using the existing CRC/header scanner, not a second parser.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FramePoint {
    pub sample: u64,
    /// Byte offset relative to the first audio frame, excluding metadata.
    pub offset: u64,
    pub block_size: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameIndex {
    pub sample_rate: u32,
    pub total_samples: u64,
    pub declared_total_samples: u64,
    pub audio_start: u64,
    pub frames: Vec<FramePoint>,
}
/// Read every frame location. Unknown total sample counts refuse explicitly,
/// as in seek_table; callers must also validate decoded audio independently.
/// This API exposes frame headers/locations, not a new audio decoder.
pub fn frame_index(path: &Path) -> Result<FrameIndex> {
    read_index(path, None)
}
/// Support STREAMINFO's unknown count using an independently decoded count.
/// A known count must match it; zero or mismatching measured counts refuse.
pub fn frame_index_with_samples(path: &Path, decoded_samples: u64) -> Result<FrameIndex> {
    read_index(path, Some(decoded_samples))
}
fn read_index(path: &Path, decoded_samples: Option<u64>) -> Result<FrameIndex> {
    let mut file = File::open(path)?;
    let mut stream = stream(&mut file)?;
    let declared_total_samples = stream.total_samples;
    if let Some(samples) = decoded_samples {
        if samples == 0 || (stream.total_samples != 0 && stream.total_samples != samples) {
            bail!("independently decoded sample count differs from STREAMINFO");
        }
        stream.total_samples = samples;
    }
    if stream.sample_rate == 0 || stream.total_samples == 0 || stream.block_size == 0 {
        bail!("STREAMINFO has no supported rate/sample/block count");
    }
    file.seek(SeekFrom::Start(stream.audio_start))?;
    let mut reader = Frames {
        file: &mut file,
        buf: Vec::new(),
        base: 0,
        eof: false,
    };
    let mut frame = header(reader.bytes_at(0)?.0).context("no frame where metadata ends")?;
    let (mut offset, mut sample, mut ordinal) = (0, 0u64, 0);
    let mut out = Vec::new();
    loop {
        let end = sample
            .checked_add(frame.block_size)
            .context("sample count overflow")?;
        if frame.first_sample(&stream) != sample || end > stream.total_samples {
            bail!("FLAC frame sample positions differ from STREAMINFO");
        }
        out.push(FramePoint {
            sample,
            offset,
            block_size: frame.block_size as u32,
        });
        if end == stream.total_samples {
            break;
        }
        sample = end;
        ordinal += 1;
        let want = if frame.variable { sample } else { ordinal };
        (offset, frame) = reader
            .next(offset, &frame, want)?
            .context("frames end before the STREAMINFO sample count")?;
    }
    Ok(FrameIndex {
        sample_rate: stream.sample_rate,
        total_samples: stream.total_samples,
        declared_total_samples,
        audio_start: stream.audio_start,
        frames: out,
    })
}
