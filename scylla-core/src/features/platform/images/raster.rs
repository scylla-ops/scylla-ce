//! Makes the square PNG that the server keeps from an uploaded image.

use std::io::Cursor;

use image::codecs::png::{CompressionType, FilterType as RowFilter, PngEncoder};
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageError, ImageFormat, ImageReader, Limits};
use scylla_domain::platform::images::Image;
use scylla_domain::DomainError;

/// The most memory that the decode of one upload takes, in bytes: a square of
/// `Image::MAX_SOURCE_SIDE` pixels with 4 bytes for each pixel, and a margin.
const MAX_ALLOC: u64 = 80 * 1024 * 1024;

/// The formats that the server reads. An SVG has no magic bytes, so it is not one of them.
const FORMATS: [ImageFormat; 3] = [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP];

/// Reads an upload, turns it as its orientation tells, cuts the center square, scales it to
/// `Image::SIDE` pixels and writes it as a PNG. Only the first frame of an animation stays.
pub(super) fn square_png(upload: &[u8]) -> Result<Vec<u8>, DomainError> {
    // Read the format from the bytes, and refuse each other format.
    let format = image::guess_format(upload).map_err(|_| DomainError::NotAnImage)?;
    if !FORMATS.contains(&format) {
        return Err(DomainError::NotAnImage);
    }

    // Decode the image in the limits of its size and of the memory.
    let mut reader = ImageReader::with_format(Cursor::new(upload), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(Image::MAX_SOURCE_SIDE);
    limits.max_image_height = Some(Image::MAX_SOURCE_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(refusal)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut picture = DynamicImage::from_decoder(decoder).map_err(refusal)?;
    picture.apply_orientation(orientation);

    // Cut the center square, scale it, and write it as a PNG.
    let square = picture
        .resize_to_fill(Image::SIDE, Image::SIDE, FilterType::Lanczos3)
        .into_rgba8();
    let mut png = Vec::new();
    let encoder =
        PngEncoder::new_with_quality(&mut png, CompressionType::Best, RowFilter::Adaptive);
    square
        .write_with_encoder(encoder)
        .map_err(|_| DomainError::NotAnImage)?;
    Ok(png)
}

/// Gives the reason of a failed decode: an image above the limits, or bytes that are not an
/// image of the format.
fn refusal(error: ImageError) -> DomainError {
    match error {
        ImageError::Limits(_) => DomainError::ImageTooLarge {
            max: Image::MAX_SOURCE_SIDE,
        },
        _ => DomainError::NotAnImage,
    }
}
