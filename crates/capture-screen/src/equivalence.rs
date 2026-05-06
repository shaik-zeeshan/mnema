use std::path::Path;

pub const CAPTURED_FRAME_EQUIVALENCE_VERSION: i64 = 1;

const EQUIVALENCE_HINT_GRID_SIZE: usize = 8;
const EQUIVALENCE_PROOF_GRID_SIZE: usize = 16;
const EQUIVALENCE_TILE_SAMPLE_GRID_SIZE: usize = 3;
const EQUIVALENCE_TILE_QUANTIZATION_STEP: u64 = 32;
const EQUIVALENCE_HASH_SEED: u64 = 0x9E37_79B9_7F4A_7C15;
const EQUIVALENCE_MAX_CHANGED_TILES: usize = 4;
const EQUIVALENCE_MAX_TILE_CHANNEL_DELTA: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFrameEquivalence {
    pub hint: String,
    pub proof: Vec<u8>,
    pub version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapturedFrameEquivalenceOutcome {
    Ready(CapturedFrameEquivalence),
    Quarantined(String),
}

impl CapturedFrameEquivalenceOutcome {
    pub fn ready(equivalence: CapturedFrameEquivalence) -> Self {
        Self::Ready(equivalence)
    }

    pub fn quarantined(error: impl Into<String>) -> Self {
        Self::Quarantined(error.into())
    }
}

pub fn captured_frame_equivalence_from_image_path(path: &Path) -> CapturedFrameEquivalenceOutcome {
    let image = match image::ImageReader::open(path) {
        Ok(reader) => match reader.decode() {
            Ok(image) => image.into_rgba8(),
            Err(error) => {
                return CapturedFrameEquivalenceOutcome::quarantined(format!(
                    "failed to decode frame artifact {}: {error}",
                    path.display()
                ));
            }
        },
        Err(error) => {
            return CapturedFrameEquivalenceOutcome::quarantined(format!(
                "failed to open frame artifact {}: {error}",
                path.display()
            ));
        }
    };

    let width = image.width() as usize;
    let height = image.height() as usize;
    let bytes = image.as_raw();
    let bytes_per_row = width.saturating_mul(4);

    match captured_frame_equivalence_from_interleaved_bytes(
        bytes,
        bytes_per_row,
        width,
        height,
        [0, 1, 2, 3],
    ) {
        Some(equivalence) => CapturedFrameEquivalenceOutcome::ready(equivalence),
        None => CapturedFrameEquivalenceOutcome::quarantined(format!(
            "failed to derive captured frame equivalence from frame artifact {}",
            path.display()
        )),
    }
}

pub fn captured_frame_equivalence_proofs_match(
    version: i64,
    left_proof: &[u8],
    right_proof: &[u8],
) -> bool {
    if version != CAPTURED_FRAME_EQUIVALENCE_VERSION {
        return false;
    }

    let expected_len = EQUIVALENCE_PROOF_GRID_SIZE * EQUIVALENCE_PROOF_GRID_SIZE * 4;
    if left_proof.len() != expected_len || right_proof.len() != expected_len {
        return false;
    }

    let mut changed_tiles = 0_usize;

    for (left_tile, right_tile) in left_proof.chunks_exact(4).zip(right_proof.chunks_exact(4)) {
        let mut tile_changed = false;

        for (left_channel, right_channel) in left_tile.iter().zip(right_tile.iter()) {
            let delta = left_channel.abs_diff(*right_channel);
            if delta > EQUIVALENCE_MAX_TILE_CHANNEL_DELTA {
                return false;
            }
            tile_changed |= delta != 0;
        }

        if tile_changed {
            changed_tiles = changed_tiles.saturating_add(1);
            if changed_tiles > EQUIVALENCE_MAX_CHANGED_TILES {
                return false;
            }
        }
    }

    true
}

pub fn captured_frame_equivalence_from_interleaved_bytes(
    bytes: &[u8],
    bytes_per_row: usize,
    width: usize,
    height: usize,
    channel_order: [usize; 4],
) -> Option<CapturedFrameEquivalence> {
    let hint_tiles = normalized_tile_summaries(
        bytes,
        bytes_per_row,
        width,
        height,
        EQUIVALENCE_HINT_GRID_SIZE,
        channel_order,
    )?;
    let proof_tiles = normalized_tile_summaries(
        bytes,
        bytes_per_row,
        width,
        height,
        EQUIVALENCE_PROOF_GRID_SIZE,
        channel_order,
    )?;

    Some(CapturedFrameEquivalence {
        hint: equivalence_hint_string(&hint_tiles),
        proof: encode_proof_tiles(&proof_tiles),
        version: CAPTURED_FRAME_EQUIVALENCE_VERSION,
    })
}

fn normalized_tile_summaries(
    bytes: &[u8],
    bytes_per_row: usize,
    width: usize,
    height: usize,
    grid_size: usize,
    channel_order: [usize; 4],
) -> Option<Vec<[u8; 4]>> {
    if bytes.is_empty() || bytes_per_row == 0 || width == 0 || height == 0 {
        return None;
    }

    let tile_rows = height.min(grid_size).max(1);
    let tile_cols = width.min(grid_size).max(1);
    let mut tiles = Vec::with_capacity(tile_rows.saturating_mul(tile_cols));

    for tile_row in 0..tile_rows {
        let row_start = tile_row.saturating_mul(height) / tile_rows;
        let row_end = ((tile_row.saturating_add(1)).saturating_mul(height) / tile_rows)
            .max(row_start.saturating_add(1))
            .min(height);

        for tile_col in 0..tile_cols {
            let col_start = tile_col.saturating_mul(width) / tile_cols;
            let col_end = ((tile_col.saturating_add(1)).saturating_mul(width) / tile_cols)
                .max(col_start.saturating_add(1))
                .min(width);

            let tile = normalized_tile_summary(
                bytes,
                bytes_per_row,
                row_start,
                row_end,
                col_start,
                col_end,
                channel_order,
            )?;
            tiles.push(tile);
        }
    }

    Some(tiles)
}

fn normalized_tile_summary(
    bytes: &[u8],
    bytes_per_row: usize,
    row_start: usize,
    row_end: usize,
    col_start: usize,
    col_end: usize,
    channel_order: [usize; 4],
) -> Option<[u8; 4]> {
    if row_start >= row_end || col_start >= col_end {
        return None;
    }

    let sample_rows = (row_end - row_start)
        .min(EQUIVALENCE_TILE_SAMPLE_GRID_SIZE)
        .max(1);
    let sample_cols = (col_end - col_start)
        .min(EQUIVALENCE_TILE_SAMPLE_GRID_SIZE)
        .max(1);
    let mut sum_channels = [0_u64; 4];
    let mut sample_count = 0_u64;

    for sample_row in 0..sample_rows {
        let row = row_start
            + (((sample_row.saturating_mul(2)).saturating_add(1))
                .saturating_mul(row_end - row_start)
                / sample_rows.saturating_mul(2))
                .min(row_end - row_start - 1);

        for sample_col in 0..sample_cols {
            let col = col_start
                + (((sample_col.saturating_mul(2)).saturating_add(1))
                    .saturating_mul(col_end - col_start)
                    / sample_cols.saturating_mul(2))
                    .min(col_end - col_start - 1);
            let pixel_offset = row
                .checked_mul(bytes_per_row)?
                .checked_add(col.checked_mul(4)?)?;
            let pixel = bytes.get(pixel_offset..pixel_offset + 4)?;

            for (channel_index, source_index) in channel_order.iter().copied().enumerate() {
                sum_channels[channel_index] += pixel[source_index] as u64;
            }
            sample_count += 1;
        }
    }

    if sample_count == 0 {
        return None;
    }

    Some(sum_channels.map(|sum| {
        ((sum / sample_count) / EQUIVALENCE_TILE_QUANTIZATION_STEP)
            .min(u8::MAX as u64) as u8
    }))
}

fn equivalence_hint_string(tiles: &[[u8; 4]]) -> String {
    let mut hash = EQUIVALENCE_HASH_SEED;

    for (index, tile) in tiles.iter().enumerate() {
        mix_equivalence_hash(&mut hash, index as u64);
        for channel in tile {
            mix_equivalence_hash(&mut hash, *channel as u64);
        }
    }

    format!("{:016x}", finalize_equivalence_hash(hash))
}

fn encode_proof_tiles(tiles: &[[u8; 4]]) -> Vec<u8> {
    let mut proof = Vec::with_capacity(tiles.len().saturating_mul(4));
    for tile in tiles {
        proof.extend_from_slice(tile);
    }
    proof
}

fn mix_equivalence_hash(hash: &mut u64, value: u64) {
    *hash ^= value.wrapping_add(EQUIVALENCE_HASH_SEED).rotate_left(25);
    *hash = hash.rotate_left(27).wrapping_mul(0x94D0_49BB_1331_11EB);
}

fn finalize_equivalence_hash(mut hash: u64) -> u64 {
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    hash ^= hash >> 33;

    if hash == 0 {
        EQUIVALENCE_HASH_SEED
    } else {
        hash
    }
}

#[cfg(test)]
mod tests {
    use super::{
        captured_frame_equivalence_from_interleaved_bytes,
        captured_frame_equivalence_proofs_match,
    };

    fn test_rgba(width: usize, height: usize, fill: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(width.saturating_mul(height).saturating_mul(4));
        for _ in 0..width.saturating_mul(height) {
            bytes.extend_from_slice(&fill);
        }
        bytes
    }

    fn set_pixel(bytes: &mut [u8], width: usize, x: usize, y: usize, rgba: [u8; 4]) {
        let offset = (y.saturating_mul(width).saturating_add(x)).saturating_mul(4);
        bytes[offset..offset + 4].copy_from_slice(&rgba);
    }

    #[test]
    fn small_localized_noise_keeps_equivalence() {
        let width = 32;
        let height = 32;
        let bytes_per_row = width * 4;
        let baseline = test_rgba(width, height, [64, 64, 64, 255]);
        let mut noisy = baseline.clone();
        set_pixel(&mut noisy, width, 10, 10, [72, 64, 64, 255]);
        set_pixel(&mut noisy, width, 11, 10, [72, 64, 64, 255]);

        let baseline = captured_frame_equivalence_from_interleaved_bytes(
            &baseline,
            bytes_per_row,
            width,
            height,
            [0, 1, 2, 3],
        )
        .expect("baseline equivalence should compute");
        let noisy = captured_frame_equivalence_from_interleaved_bytes(
            &noisy,
            bytes_per_row,
            width,
            height,
            [0, 1, 2, 3],
        )
        .expect("noisy equivalence should compute");

        assert_eq!(baseline.hint, noisy.hint);
        assert!(captured_frame_equivalence_proofs_match(
            baseline.version,
            &baseline.proof,
            &noisy.proof,
        ));
    }

    #[test]
    fn larger_change_breaks_equivalence() {
        let width = 32;
        let height = 32;
        let bytes_per_row = width * 4;
        let baseline = test_rgba(width, height, [64, 64, 64, 255]);
        let mut changed = baseline.clone();

        for y in 8..20 {
            for x in 8..20 {
                set_pixel(&mut changed, width, x, y, [240, 240, 240, 255]);
            }
        }

        let baseline = captured_frame_equivalence_from_interleaved_bytes(
            &baseline,
            bytes_per_row,
            width,
            height,
            [0, 1, 2, 3],
        )
        .expect("baseline equivalence should compute");
        let changed = captured_frame_equivalence_from_interleaved_bytes(
            &changed,
            bytes_per_row,
            width,
            height,
            [0, 1, 2, 3],
        )
        .expect("changed equivalence should compute");

        assert!(
            !captured_frame_equivalence_proofs_match(
                baseline.version,
                &baseline.proof,
                &changed.proof,
            ),
            "larger OCR-relevant changes must not compare as equivalent"
        );
    }

    #[test]
    fn byte_width_passed_as_pixel_width_fails_equivalence_derivation() {
        let width = 32;
        let height = 32;
        let bytes_per_row = width * 4;
        let bytes = test_rgba(width, height, [64, 64, 64, 255]);

        assert!(
            captured_frame_equivalence_from_interleaved_bytes(
                &bytes,
                bytes_per_row,
                width,
                height,
                [0, 1, 2, 3],
            )
            .is_some(),
            "pixel width should derive equivalence"
        );

        assert!(
            captured_frame_equivalence_from_interleaved_bytes(
                &bytes,
                bytes_per_row,
                bytes_per_row,
                height,
                [0, 1, 2, 3],
            )
            .is_none(),
            "byte width misread as pixel width should fail derivation"
        );
    }
}
