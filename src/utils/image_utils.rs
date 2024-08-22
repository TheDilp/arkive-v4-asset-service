use image::DynamicImage;

pub fn encode_lossy_webp(img: DynamicImage) -> Vec<u8> {
    let img = img.to_rgba8();
    let (width, height) = img.dimensions();
    webp::Encoder::new(&*img, webp::PixelLayout::Rgba, width, height)
        .encode(1.0)
        .to_vec()
}
