use std::sync::Arc;

use gpui_kit::RenderImage;
use objc2_core_foundation::{CFData, CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGDataProvider, CGImageAlphaInfo, CGPDFBox,
    CGPDFDocument, CGPDFPage,
};

pub struct Page {
    pub image: Arc<RenderImage>,
    pub width: u32,
    pub height: u32,
}

fn document(bytes: &[u8]) -> Result<CFRetained<CGPDFDocument>, String> {
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("PDF exceeds 64 MiB".into());
    }
    let data = CFData::from_bytes(bytes);
    let provider = CGDataProvider::with_cf_data(Some(&data)).ok_or("Cannot read PDF data")?;
    let document = CGPDFDocument::with_provider(Some(&provider)).ok_or("Invalid PDF")?;
    if CGPDFDocument::is_encrypted(Some(&document)) {
        return Err("Encrypted PDF is not supported".into());
    }
    Ok(document)
}

pub fn pages(bytes: &[u8]) -> Result<usize, String> {
    let document = document(bytes)?;
    let count = CGPDFDocument::number_of_pages(Some(&document));
    if !(1..=10_000).contains(&count) {
        return Err("PDF must contain between 1 and 10000 pages".into());
    }
    Ok(count)
}

pub fn render(bytes: &[u8], index: usize) -> Result<Page, String> {
    let document = document(bytes)?;
    let page = CGPDFDocument::page(
        Some(&document),
        index.checked_add(1).ok_or("Invalid PDF page")?,
    )
    .ok_or("PDF page does not exist")?;
    let bounds = CGPDFPage::box_rect(Some(&page), CGPDFBox::CropBox);
    let rotated = CGPDFPage::rotation_angle(Some(&page)).rem_euclid(180) != 0;
    let (width, height) = if rotated {
        (bounds.size.height, bounds.size.width)
    } else {
        (bounds.size.width, bounds.size.height)
    };
    if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. {
        return Err("Invalid PDF page size".into());
    }
    let scale = 2_f64
        .min(2400. / width.max(height))
        .min((4_000_000. / (width * height)).sqrt());
    let width = (width * scale).ceil().max(1.) as u32;
    let height = (height * scale).ceil().max(1.) as u32;
    let mut pixels = vec![0_u8; width as usize * height as usize * 4];
    let color = CGColorSpace::new_device_rgb().ok_or("Cannot create PDF color space")?;
    let rect = CGRect::new(
        CGPoint::new(0., 0.),
        CGSize::new(width as f64, height as f64),
    );
    {
        // CoreGraphics borrows this buffer until the context is dropped.
        let context = unsafe {
            CGBitmapContextCreate(
                pixels.as_mut_ptr().cast(),
                width as usize,
                height as usize,
                8,
                width as usize * 4,
                Some(&color),
                CGImageAlphaInfo::PremultipliedLast.0,
            )
        }
        .ok_or("Cannot allocate PDF page bitmap")?;
        CGContext::set_rgb_fill_color(Some(&context), 1., 1., 1., 1.);
        CGContext::fill_rect(Some(&context), rect);
        // Quartz's fitting transform does not enlarge a page beyond its natural size.
        CGContext::scale_ctm(Some(&context), scale, scale);
        let target = CGRect::new(
            CGPoint::new(0., 0.),
            CGSize::new(width as f64 / scale, height as f64 / scale),
        );
        let transform =
            CGPDFPage::drawing_transform(Some(&page), CGPDFBox::CropBox, target, 0, true);
        CGContext::concat_ctm(Some(&context), transform);
        CGContext::draw_pdf_page(Some(&context), Some(&page));
    }
    // GPUI's RenderImage buffers are BGRA, even though image::Frame calls them RGBA.
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let frame = image::Frame::new(
        image::RgbaImage::from_raw(width, height, pixels).ok_or("Invalid PDF bitmap")?,
    );
    Ok(Page {
        image: Arc::new(RenderImage::new(vec![frame])),
        width,
        height,
    })
}

#[cfg(test)]
pub(super) mod tests;
