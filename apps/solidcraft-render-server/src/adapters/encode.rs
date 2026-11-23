//! A frame's pieces encoded (in parallel) into one payload, with the rectangles that locate them;
//! and whole frames as PNG for snapshots. JPEG and PNG come from the `image` crate already in the
//! tree (pure Rust); `turbojpeg` is the first new dependency if M3 asks for it.

use std::io::Cursor;

use image::ImageEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use rayon::prelude::*;

use crate::domain::codec::{JPEG_QUALITY, Piece};
use crate::domain::damage::Rect;
use crate::domain::wire::{Codec, FrameRect};

/// Encode `pieces` of `frame` (tight RGBA rows, `frame_w` px wide): the rectangles for the
/// frame message and the payload they index.
pub fn encode_pieces(frame: &[u8], frame_w: u32, pieces: &[Piece]) -> Result<(Vec<FrameRect>, Vec<u8>), String> {
    let encoded: Vec<Vec<u8>> = pieces.par_iter().map(|p| encode_piece(frame, frame_w, p)).collect::<Result<_, _>>()?;
    let mut payload = Vec::with_capacity(encoded.iter().map(Vec::len).sum());
    let mut rects = Vec::with_capacity(pieces.len());
    for (piece, bytes) in pieces.iter().zip(encoded) {
        let Rect { x, y, w, h } = piece.rect;
        rects.push(FrameRect { x, y, w, h, codec: piece.codec, ew: w, eh: h, offset: payload.len() as u32, length: bytes.len() as u32 });
        payload.extend_from_slice(&bytes);
    }
    Ok((rects, payload))
}

/// A whole frame as a PNG file's bytes.
pub fn png(frame: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut opaque = frame.to_vec();
    force_opaque(&mut opaque);
    png_bytes(&opaque, w, h)
}

fn encode_piece(frame: &[u8], frame_w: u32, piece: &Piece) -> Result<Vec<u8>, String> {
    let r = piece.rect;
    match piece.codec {
        Codec::Rgba => {
            let mut rgba = crop(frame, frame_w, r, 4);
            force_opaque(&mut rgba);
            Ok(rgba)
        }
        Codec::Png => {
            let mut rgba = crop(frame, frame_w, r, 4);
            force_opaque(&mut rgba);
            png_bytes(&rgba, r.w, r.h)
        }
        Codec::Jpeg => {
            let rgb = crop(frame, frame_w, r, 3);
            let mut out = Cursor::new(Vec::with_capacity(rgb.len() / 8));
            // `image-0.25.10/src/codecs/jpeg/encoder.rs:398,479` (always 4:2:2).
            JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
                .encode(&rgb, r.w, r.h, image::ExtendedColorType::Rgb8)
                .map_err(|e| format!("jpeg: {e}"))?;
            Ok(out.into_inner())
        }
    }
}

fn png_bytes(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut out = Cursor::new(Vec::new());
    // `image-0.25.10/src/codecs/png.rs:631`: fast deflate, as design §2.3 assumes.
    PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive)
        .write_image(rgba, w, h, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("png: {e}"))?;
    Ok(out.into_inner())
}

/// The pixels of `r` as tight rows of `channels` (4: RGBA, 3: RGB).
fn crop(frame: &[u8], frame_w: u32, r: Rect, channels: usize) -> Vec<u8> {
    let stride = frame_w as usize * 4;
    let mut out = Vec::with_capacity(r.w as usize * r.h as usize * channels);
    for y in r.y..r.y + r.h {
        let start = y as usize * stride + r.x as usize * 4;
        let row = frame.get(start..start + r.w as usize * 4).unwrap_or_default();
        if channels == 4 {
            out.extend_from_slice(row);
        } else {
            out.extend(row.as_chunks::<4>().0.iter().flat_map(|px| [px[0], px[1], px[2]]));
        }
    }
    out
}

/// The page draws rectangles over the last frame: every pixel is opaque (the UI covers the
/// whole pane; a compositor may leave alpha below 255 where it blended).
fn force_opaque(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pieces_land_back_to_back_in_the_payload() {
        let (w, h) = (300u32, 300u32);
        let frame: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, (i % 7) as u8, 9, 0]).collect();
        let small = Rect { x: 1, y: 2, w: 3, h: 2 };
        let big = Rect { x: 0, y: 0, w: 300, h: 300 };
        let pieces = [Piece { rect: small, codec: Codec::Rgba }, Piece { rect: big, codec: Codec::Jpeg }, Piece { rect: big, codec: Codec::Png }];
        let (rects, payload) = encode_pieces(&frame, w, &pieces).unwrap();
        assert_eq!(rects[0].length, 3 * 2 * 4);
        let first_px = (2 * w + 1) as usize * 4;
        assert_eq!(&payload[..4], &[frame[first_px], frame[first_px + 1], 9, 255], "raw rows, alpha 255");
        assert_eq!(rects[1].offset, rects[0].length);
        assert_eq!(&payload[rects[1].offset as usize..][..2], &[0xFF, 0xD8], "a JPEG file");
        assert_eq!(&payload[rects[2].offset as usize + 1..][..3], b"PNG");
        assert_eq!(rects[2].offset + rects[2].length, payload.len() as u32);
    }
}
