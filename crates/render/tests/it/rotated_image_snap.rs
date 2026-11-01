//! Contributor-original synthetic pages (#5530): a shrunk, interpolated image on a page turned by
//! `/Rotate` is drawn exactly as on the upright page, turned. The crop box and scale put every
//! page edge on a whole device pixel, so a turn is a pure permutation of pixels and the turned
//! render, turned back, must match the upright one. The image itself sits at fractional
//! coordinates: an upright image there is snapped to whole pixels and shrunk once, and before the
//! fix a turned one was not, so it was shrunk to a rounded-up size and resampled a second time at
//! a fractional scale and offset (most of its pixels off, by up to 200 grey levels).
use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RenderedPage};
use std::sync::Arc;

/// A PDF of the given objects (object 1 the catalog), with a cross-reference table.
fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer << /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    bytes
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut bytes = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(b"\nendstream");
    bytes
}

/// Image size in samples, and a busy grey pattern with edges both ways, as a scan has.
const W: usize = 400;
const H: usize = 500;

fn samples() -> Vec<u8> {
    (0..H).flat_map(|y| (0..W).map(move |x| ((x * 7 + y * 13) % 256) as u8 ^ if (x / 3 + y / 5) % 2 == 0 { 0 } else { 0x80 })).collect()
}

/// A page whose crop box sits inside a larger media box, as in the report, with the image drawn
/// at fractional coordinates inside it.
fn page(rotate: u32, interpolate: bool) -> Vec<u8> {
    let image = stream(
        &format!("/Type /XObject /Subtype /Image /Width {W} /Height {H} /ColorSpace /DeviceGray /BitsPerComponent 8 /Interpolate {interpolate}"),
        &samples(),
    );
    pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 794 1224] /CropBox [114 510 680 1210] /Rotate {rotate} /Contents 4 0 R /Resources << /XObject << /Im 5 0 R >> >> >>"
        )
        .into_bytes(),
        stream("", b"q 500.3 0 0 600.7 150.15 560.45 cm /Im Do Q"),
        image,
    ])
}

/// At half a pixel per point every crop-box edge, upright or turned, is a whole device pixel,
/// and the 400 × 500 image is shrunk to about 250 × 300 pixels.
fn render(rotate: u32, interpolate: bool) -> RenderedPage {
    let mut renderer = PageRenderer::new(Arc::new(page(rotate, interpolate)), RenderConfig::default());
    let out = renderer.render(RenderRequest { page: 0, scale: 0.5, ..Default::default() });
    assert!(out.error.is_none(), "{:?}", out.error);
    out
}

fn grey(out: &RenderedPage, x: u32, y: u32) -> u8 {
    out.rgba[((y * out.width + x) * 4) as usize]
}

/// The largest grey difference between the turned render and the upright one turned the same way.
fn turned_difference(rotate: u32, interpolate: bool) -> u8 {
    let upright = render(0, interpolate);
    let turned = render(rotate, interpolate);
    let (w, h) = (upright.width, upright.height);
    // `/Rotate` turns the page clockwise.
    let expected = |x: u32, y: u32| match rotate {
        90 => grey(&upright, y, h - 1 - x),
        180 => grey(&upright, w - 1 - x, h - 1 - y),
        _ => grey(&upright, w - 1 - y, x),
    };
    let (tw, th) = if rotate == 180 { (w, h) } else { (h, w) };
    assert_eq!((turned.width, turned.height), (tw, th), "/Rotate {rotate}");
    let mut worst = 0;
    for y in 0..th {
        for x in 0..tw {
            worst = worst.max(grey(&turned, x, y).abs_diff(expected(x, y)));
        }
    }
    worst
}

#[test]
fn a_shrunk_image_on_a_turned_page_is_the_upright_render_turned() {
    for interpolate in [true, false] {
        for rotate in [90, 180, 270] {
            let worst = turned_difference(rotate, interpolate);
            assert!(worst <= 1, "/Rotate {rotate}, /Interpolate {interpolate}: off by up to {worst} grey levels");
        }
    }
}
