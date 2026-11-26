//! Unit tests of the square PNG of an upload.

use std::io::Cursor;

use image::{ImageFormat, Rgba, RgbaImage};
use scylla_domain::platform::images::Image;
use scylla_domain::DomainError;

use super::raster::square_png;

/// Writes an image of `width` by `height` pixels in a format. The left half is red and the right
/// half is blue.
fn picture(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
    let image = RgbaImage::from_fn(width, height, |x, _| {
        if x < width / 2 {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([0, 0, 255, 255])
        }
    });
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .to_rgb8()
        .write_to(&mut bytes, format)
        .unwrap();
    bytes.into_inner()
}

/// Reads a PNG that the server made.
fn read(png: &[u8]) -> RgbaImage {
    let image = image::load_from_memory_with_format(png, ImageFormat::Png).unwrap();
    image.into_rgba8()
}

/// A PNG, a JPEG and a WebP each give a square PNG of `Image::SIDE` pixels.
#[test]
fn square_png_reads_png_jpeg_and_webp() {
    for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
        // given
        let upload = picture(40, 30, format);

        // when
        let png = square_png(&upload).unwrap();

        // then
        assert_eq!(
            image::guess_format(&png).unwrap(),
            ImageFormat::Png,
            "{format:?}"
        );
        let square = read(&png);
        assert_eq!(
            square.dimensions(),
            (Image::SIDE, Image::SIDE),
            "{format:?}"
        );
    }
}

/// A wide image keeps its center: the two halves stay on the two sides.
#[test]
fn square_png_keeps_the_center_of_a_wide_image() {
    // given
    let upload = picture(300, 100, ImageFormat::Png);

    // when
    let square = read(&square_png(&upload).unwrap());

    // then
    let left = square.get_pixel(10, Image::SIDE / 2);
    let right = square.get_pixel(Image::SIDE - 10, Image::SIDE / 2);
    assert_eq!(left.0, [255, 0, 0, 255]);
    assert_eq!(right.0, [0, 0, 255, 255]);
}

/// An SVG, a GIF and bytes that are not an image are refused.
#[test]
fn square_png_refuses_another_format() {
    // given
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
    let gif = [b"GIF89a".as_slice(), &[1, 0, 1, 0, 0, 0, 0, b';']].concat();
    let text = b"not an image";
    let mut broken = picture(40, 30, ImageFormat::Png);
    broken.truncate(60);

    // when
    let results = [&svg[..], &gif[..], &text[..], &broken[..]].map(square_png);

    // then
    for result in results {
        assert_eq!(result, Err(DomainError::NotAnImage));
    }
}

/// An image with more than `Image::MAX_SOURCE_SIDE` pixels on a side is refused before its
/// decode.
#[test]
fn square_png_refuses_an_image_above_the_size_limit() {
    // given
    let upload = picture(Image::MAX_SOURCE_SIDE + 1, 1, ImageFormat::Png);

    // when
    let result = square_png(&upload);

    // then
    assert_eq!(
        result,
        Err(DomainError::ImageTooLarge {
            max: Image::MAX_SOURCE_SIDE
        })
    );
}
