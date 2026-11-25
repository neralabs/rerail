//! The tray's pictures: the status circle on the menu's first line, and the badge drawn onto the icon.

use std::io::Cursor;

/// An image as straight (not premultiplied) 8-bit RGBA, row by row.
#[derive(Clone, PartialEq, Debug)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// The icon's badge: a plain dot when all is good, a circle with "!" when something needs attention.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Badge {
    Good,
    Attention,
}

const GREEN: [u8; 3] = [0x34, 0xC7, 0x59];
const RED: [u8; 3] = [0xFF, 0x3B, 0x30];
const AMBER: [u8; 3] = [0xFF, 0x9F, 0x0A];
const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];
const BLACK: [u8; 3] = [0, 0, 0];

/// The status circle's diameter, as a share of its image's edge.
const CIRCLE: f32 = 0.75;
/// The badge's diameter, as a share of the icon's edge.
const BADGE: f32 = 0.45;
/// The clear ring around the badge, in pixels, so the badge stands apart from the glyph under it.
const KNOCK_OUT: f32 = 1.0;

/// The status line's icon: a filled circle, green when the daemon is healthy and red when not, on transparency.
pub fn status_circle(healthy: bool, size: u32) -> Rgba {
    let mut image = Rgba {
        width: size,
        height: size,
        pixels: vec![0; (size * size * 4) as usize],
    };
    let colour = if healthy { GREEN } else { RED };
    let (centre, radius) = (size as f32 / 2.0, size as f32 * CIRCLE / 2.0);
    image.each_pixel(|x, y, pixel| paint(pixel, colour, disc(centre, centre, radius, x, y)));
    image
}

/// Decodes one of the icons built into the binary, which are 8-bit RGBA. A test decodes each of them, so the
/// panics here cannot reach a user.
pub fn decode_png(bytes: &[u8]) -> Rgba {
    let mut reader = png::Decoder::new(Cursor::new(bytes))
        .read_info()
        .expect("an embedded icon is a PNG");
    let mut pixels = vec![
        0;
        reader
            .output_buffer_size()
            .expect("an embedded icon fits in memory")
    ];
    let info = reader
        .next_frame(&mut pixels)
        .expect("an embedded icon decodes");
    assert_eq!(
        (info.color_type, info.bit_depth),
        (png::ColorType::Rgba, png::BitDepth::Eight),
        "an embedded icon is 8-bit RGBA"
    );
    pixels.truncate(info.buffer_size());
    Rgba {
        width: info.width,
        height: info.height,
        pixels,
    }
}

/// The icon with its badge in the lower right corner, cleared around by a thin transparent ring.
///
/// In colour, the good badge is a green dot and the attention badge an amber disc with a white "!". As a template
/// (macOS tints it, so only alpha counts) both are black, and the attention badge's "!" is cut out of its disc.
pub fn badged(base: &Rgba, badge: Badge, template: bool) -> Rgba {
    let mut icon = base.clone();
    let radius = base.width.min(base.height) as f32 * BADGE / 2.0;
    let (cx, cy) = (base.width as f32 - radius, base.height as f32 - radius);
    let colour = match (template, badge) {
        (true, _) => BLACK,
        (false, Badge::Good) => GREEN,
        (false, Badge::Attention) => AMBER,
    };
    icon.each_pixel(|x, y, pixel| {
        erase(pixel, disc(cx, cy, radius + KNOCK_OUT, x, y));
        paint(pixel, colour, disc(cx, cy, radius, x, y));
        if badge == Badge::Attention {
            let mark = exclamation(cx, cy, radius, x, y);
            if template {
                erase(pixel, mark);
            } else {
                paint(pixel, WHITE, mark);
            }
        }
    });
    icon
}

impl Rgba {
    /// The pixels as ARGB32 in network byte order (alpha first), which is what a StatusNotifierItem icon takes.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn to_argb(&self) -> Vec<u8> {
        self.pixels
            .chunks_exact(4)
            .flat_map(|rgba| [rgba[3], rgba[0], rgba[1], rgba[2]])
            .collect()
    }

    /// The image as a PNG file, which is what a dbusmenu item's icon takes.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn to_png(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // Writing to memory fails only on dimensions the tray's own images never have.
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&self.pixels))
            .expect("an image the tray drew encodes");
        bytes
    }

    /// Calls `f` with each pixel's centre and its four bytes.
    fn each_pixel(&mut self, mut f: impl FnMut(f32, f32, &mut [u8])) {
        let width = self.width as usize;
        for (at, pixel) in self.pixels.chunks_exact_mut(4).enumerate() {
            f((at % width) as f32 + 0.5, (at / width) as f32 + 0.5, pixel);
        }
    }
}

/// How much of the pixel centred on (x, y) a disc covers, from 0 to 1, anti-aliased over one pixel of its edge.
fn disc(cx: f32, cy: f32, radius: f32, x: f32, y: f32) -> f32 {
    (radius + 0.5 - (x - cx).hypot(y - cy)).clamp(0.0, 1.0)
}

/// How much of the pixel centred on (x, y) a rectangle covers, from 0 to 1.
fn rectangle(left: f32, top: f32, right: f32, bottom: f32, x: f32, y: f32) -> f32 {
    let overlap =
        |low: f32, high: f32, at: f32| ((at + 0.5).min(high) - (at - 0.5).max(low)).max(0.0);
    overlap(left, right, x) * overlap(top, bottom, y)
}

/// The coverage of a "!" in a badge of `radius` around (cx, cy): a vertical bar over a dot.
fn exclamation(cx: f32, cy: f32, radius: f32, x: f32, y: f32) -> f32 {
    let half_width = radius * 0.16;
    let bar = rectangle(
        cx - half_width,
        cy - radius * 0.6,
        cx + half_width,
        cy + radius * 0.16,
        x,
        y,
    );
    bar.max(disc(cx, cy + radius * 0.48, radius * 0.18, x, y))
}

/// Draws `colour` over the pixel at `coverage`, in straight alpha.
fn paint(pixel: &mut [u8], colour: [u8; 3], coverage: f32) {
    if coverage <= 0.0 {
        return;
    }
    let below = f32::from(pixel[3]) / 255.0 * (1.0 - coverage);
    let alpha = coverage + below;
    for (channel, &value) in pixel.iter_mut().zip(&colour) {
        *channel =
            ((f32::from(value) * coverage + f32::from(*channel) * below) / alpha).round() as u8;
    }
    pixel[3] = (alpha * 255.0).round() as u8;
}

/// Clears the pixel towards transparent by `coverage`.
fn erase(pixel: &mut [u8], coverage: f32) {
    pixel[3] = (f32::from(pixel[3]) * (1.0 - coverage)).round() as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argb_from_rgba_reorders_channels() {
        let image = Rgba {
            width: 2,
            height: 1,
            pixels: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        assert_eq!(image.to_argb(), [4, 1, 2, 3, 8, 5, 6, 7]);
    }

    #[test]
    fn png_round_trips() {
        let image = status_circle(true, 16);
        assert_eq!(decode_png(&image.to_png()), image);
    }

    const GREEN: [u8; 4] = [0x34, 0xC7, 0x59, 255];
    const RED: [u8; 4] = [0xFF, 0x3B, 0x30, 255];
    const AMBER: [u8; 3] = [0xFF, 0x9F, 0x0A];
    const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];

    fn pixel(image: &Rgba, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * image.width + x) * 4) as usize;
        image.pixels[at..at + 4].try_into().unwrap()
    }

    /// Every pixel inside a disc of `radius` around the badge's centre, in a square icon.
    fn badge_disc(image: &Rgba, radius: f32) -> Vec<[u8; 4]> {
        let centre = image.width as f32 - image.width as f32 * BADGE / 2.0;
        let mut seen = Vec::new();
        for y in 0..image.height {
            for x in 0..image.width {
                let (dx, dy) = (x as f32 + 0.5 - centre, y as f32 + 0.5 - centre);
                if (dx * dx + dy * dy).sqrt() <= radius {
                    seen.push(pixel(image, x, y));
                }
            }
        }
        seen
    }

    fn solid(width: u32, height: u32, colour: [u8; 4]) -> Rgba {
        Rgba {
            width,
            height,
            pixels: colour.repeat((width * height) as usize),
        }
    }

    #[test]
    fn status_circle_is_green_or_red_in_the_centre() {
        assert_eq!(pixel(&status_circle(true, 32), 16, 16), GREEN);
        assert_eq!(pixel(&status_circle(false, 32), 16, 16), RED);
    }

    #[test]
    fn status_circle_is_transparent_in_the_corner() {
        let circle = status_circle(true, 32);
        assert_eq!(
            (circle.width, circle.height, circle.pixels.len()),
            (32, 32, 32 * 32 * 4)
        );
        assert_eq!(pixel(&circle, 0, 0)[3], 0);
    }

    #[test]
    fn badge_sits_in_the_lower_right() {
        let base = solid(32, 32, [10, 20, 30, 255]);
        let icon = badged(&base, Badge::Good, false);
        assert_eq!(pixel(&icon, 25, 25), GREEN);
        assert_eq!(pixel(&icon, 0, 0), [10, 20, 30, 255]);
        assert_eq!(pixel(&icon, 31, 0), [10, 20, 30, 255]);
        assert_eq!(pixel(&icon, 0, 31), [10, 20, 30, 255]);
    }

    #[test]
    fn attention_badge_is_amber_with_a_mark() {
        let icon = badged(&solid(32, 32, [10, 20, 30, 255]), Badge::Attention, false);
        let disc = badge_disc(&icon, 5.0);
        assert!(
            disc.contains(&[AMBER[0], AMBER[1], AMBER[2], 255]),
            "no amber in the badge"
        );
        assert!(
            disc.contains(&[WHITE[0], WHITE[1], WHITE[2], 255]),
            "no white mark in the badge"
        );
    }

    #[test]
    fn template_badges_stay_monochrome() {
        let base = decode_png(include_bytes!("../assets/icon-template.png"));
        let good = badged(&base, Badge::Good, true);
        let attention = badged(&base, Badge::Attention, true);
        for icon in [&good, &attention] {
            assert!(
                icon.pixels.chunks(4).all(|p| p[..3] == [0, 0, 0]),
                "a template pixel has colour"
            );
        }
        assert_ne!(good.pixels, attention.pixels);
        // The good badge is a solid disc; the attention badge has its "!" cut out of the disc.
        assert!(badge_disc(&good, 6.0).iter().all(|p| p[3] == 255));
        assert!(badge_disc(&attention, 6.0).iter().any(|p| p[3] == 0));
    }

    #[test]
    fn badge_leaves_a_clear_ring_around_its_disc() {
        let icon = badged(&solid(44, 44, [0, 0, 0, 255]), Badge::Good, true);
        // Up and left along the diagonal from the badge's centre: the opaque disc, a clear gap, then the opaque base.
        let alphas: Vec<u8> = (0..=34).rev().map(|at| pixel(&icon, at, at)[3]).collect();
        let disc_ends = alphas.iter().position(|&a| a < 255).unwrap();
        let base_starts = disc_ends + alphas[disc_ends..].iter().position(|&a| a == 255).unwrap();
        assert!(
            alphas[disc_ends..base_starts].iter().any(|&a| a < 128),
            "no gap between badge and base: {alphas:?}"
        );
    }

    #[test]
    fn embedded_icons_decode_at_their_sizes() {
        let template = decode_png(include_bytes!("../assets/icon-template.png"));
        assert_eq!(
            (template.width, template.height, template.pixels.len()),
            (44, 44, 44 * 44 * 4)
        );
        assert!(template.pixels.chunks(4).all(|p| p[..3] == [0, 0, 0]));
        assert!(template.pixels.chunks(4).any(|p| p[3] == 255));
        let colour = decode_png(include_bytes!("../assets/icon.png"));
        assert_eq!(
            (colour.width, colour.height, colour.pixels.len()),
            (32, 32, 32 * 32 * 4)
        );
        assert!(colour.pixels.chunks(4).all(|p| p[3] == 255));
    }
}
