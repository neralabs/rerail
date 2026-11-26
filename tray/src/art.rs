//! The tray's pictures: the circles on the menu's status line and repository rows, and the badge drawn onto the icon.

use std::io::Cursor;

/// An image as straight (not premultiplied) 8-bit RGBA, row by row.
#[derive(Clone, PartialEq, Debug)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

const GREEN: [u8; 3] = [0x34, 0xC7, 0x59];
const RED: [u8; 3] = [0xFF, 0x3B, 0x30];
const AMBER: [u8; 3] = [0xFF, 0x9F, 0x0A];
/// macOS's system grey, which reads as "not running" in light and dark menus alike.
const GREY: [u8; 3] = [0x8E, 0x8E, 0x93];
const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];
const BLACK: [u8; 3] = [0, 0, 0];

/// The status circle's diameter, as a share of its image's edge: 10 pt of macOS's 18 pt item image, about the size of
/// Docker Desktop's, centred in the same image, so the status text lines up with the other items' text.
const CIRCLE: f32 = 10.0 / 18.0;
/// The badge's diameter, as a share of the shorter side of what the icon draws.
const BADGE: f32 = 0.55;
/// How far the badge reaches past the lower right corner of what the icon draws, as a share of its radius, so it covers
/// the end of the glyph's stroke there; the canvas's edge stops it.
const OVERHANG: f32 = 0.25;
/// The clear ring around the badge, in pixels, so the badge stands apart from the glyph under it.
const KNOCK_OUT: f32 = 1.0;

/// The status line's icon: a filled circle, green when the daemon is healthy and red when not, on transparency.
pub fn status_circle(healthy: bool, size: u32) -> Rgba {
    circle(if healthy { GREEN } else { RED }, size)
}

/// A repository row's icon: the status line's circle, green where the repository's rules run and grey where not.
pub fn repository_circle(running: bool, size: u32) -> Rgba {
    circle(if running { GREEN } else { GREY }, size)
}

/// A filled circle of `colour`, [`CIRCLE`] of its image's edge across, on transparency.
fn circle(colour: [u8; 3], size: u32) -> Rgba {
    let mut image = Rgba {
        width: size,
        height: size,
        pixels: vec![0; (size * size * 4) as usize],
    };
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

/// The tray icon: the plain base image when all is good; when something needs attention, the base with a "!" badge in
/// its lower right corner, cleared around by a thin transparent ring.
///
/// In colour the badge is an amber disc with a white "!". As a template (macOS tints it, so only alpha counts) the disc
/// is black, with the "!" cut out of it.
pub fn icon(base: &Rgba, attention: bool, template: bool) -> Rgba {
    let mut icon = base.clone();
    if !attention {
        return icon;
    }
    let (cx, cy, radius) = badge_circle(base);
    let colour = if template { BLACK } else { AMBER };
    icon.each_pixel(|x, y, pixel| {
        erase(pixel, disc(cx, cy, radius + KNOCK_OUT, x, y));
        paint(pixel, colour, disc(cx, cy, radius, x, y));
        let mark = exclamation(cx, cy, radius, x, y);
        if template {
            erase(pixel, mark);
        } else {
            paint(pixel, WHITE, mark);
        }
    });
    icon
}

/// The badge's centre and radius on `base`: at the lower right corner of what the image draws (its pixels that are not
/// fully transparent), so a glyph padded on its canvas gets a badge at its own size and corner.
fn badge_circle(base: &Rgba) -> (f32, f32, f32) {
    let (mut left, mut top, mut right, mut bottom) = (base.width, base.height, 0, 0);
    for (at, pixel) in base.pixels.chunks_exact(4).enumerate() {
        if pixel[3] > 0 {
            let (x, y) = (at as u32 % base.width, at as u32 / base.width);
            (left, top) = (left.min(x), top.min(y));
            (right, bottom) = (right.max(x + 1), bottom.max(y + 1));
        }
    }
    if right <= left || bottom <= top {
        (left, top, right, bottom) = (0, 0, base.width, base.height);
    }
    let radius = (right - left).min(bottom - top) as f32 * BADGE / 2.0;
    let reach =
        |edge: u32, canvas: u32| (edge as f32 + radius * OVERHANG).min(canvas as f32) - radius;
    (reach(right, base.width), reach(bottom, base.height), radius)
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
    const GREY: [u8; 4] = [0x8E, 0x8E, 0x93, 255];
    const AMBER: [u8; 3] = [0xFF, 0x9F, 0x0A];
    const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];

    fn pixel(image: &Rgba, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * image.width + x) * 4) as usize;
        image.pixels[at..at + 4].try_into().unwrap()
    }

    /// Every pixel of `image` inside a disc of `radius` around the centre of the badge drawn on `base`.
    fn badge_disc(base: &Rgba, image: &Rgba, radius: f32) -> Vec<[u8; 4]> {
        let (cx, cy, _) = badge_circle(base);
        let mut seen = Vec::new();
        for y in 0..image.height {
            for x in 0..image.width {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
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
    fn repository_circle_is_green_or_grey_in_the_centre() {
        assert_eq!(pixel(&repository_circle(true, 32), 16, 16), GREEN);
        assert_eq!(pixel(&repository_circle(false, 32), 16, 16), GREY);
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
    fn status_circle_is_10_of_18_parts_across() {
        // 36 px is macOS's 18 pt item image at 2x, so the disc is 20 px, 10 pt, across: a radius of 10 px.
        let circle = status_circle(true, 36);
        assert_eq!(pixel(&circle, 18, 18), GREEN);
        // A pixel 8.5 px from the centre is inside; one 11.5 px away is past the radius and its pixel of anti-aliasing.
        assert_eq!(pixel(&circle, 26, 18), GREEN);
        assert_eq!(pixel(&circle, 29, 18)[3], 0);
        assert_eq!(pixel(&circle, 18, 29)[3], 0);
    }

    #[test]
    fn badge_sits_in_the_lower_right() {
        let base = solid(32, 32, [10, 20, 30, 255]);
        let with_badge = icon(&base, true, false);
        assert_eq!(
            pixel(&with_badge, 20, 24),
            [AMBER[0], AMBER[1], AMBER[2], 255]
        );
        assert_eq!(pixel(&with_badge, 0, 0), [10, 20, 30, 255]);
        assert_eq!(pixel(&with_badge, 31, 0), [10, 20, 30, 255]);
        assert_eq!(pixel(&with_badge, 0, 31), [10, 20, 30, 255]);
    }

    #[test]
    fn badge_follows_the_glyph_not_the_canvas() {
        // A 29 x 32 px glyph box, from (4, 6) to (33, 38), on a transparent 37 x 44 px canvas.
        let mut base = solid(37, 44, [0, 0, 0, 0]);
        for y in 6..38 {
            for x in 4..33 {
                let at = ((y * 37 + x) * 4) as usize;
                base.pixels[at..at + 4].copy_from_slice(&[10, 20, 30, 255]);
            }
        }
        let with_badge = icon(&base, true, false);
        let amber = [AMBER[0], AMBER[1], AMBER[2], 255];
        // The badge is 55% of the box's 29 px width across, and reaches 2 px past the box's lower right corner, over
        // the end of whatever the glyph draws there: its centre is at (27, 32). Left of its "!", the disc.
        assert_eq!(pixel(&with_badge, 24, 34), amber);
        assert_eq!(pixel(&with_badge, 33, 32), amber);
        // The canvas's own corner and the padding beside the box stay as they were.
        assert_eq!(pixel(&with_badge, 36, 43), [0, 0, 0, 0]);
        assert_eq!(pixel(&with_badge, 35, 20), [0, 0, 0, 0]);
        // Above the badge and its clear ring, the box is untouched.
        assert_eq!(pixel(&with_badge, 28, 21), [10, 20, 30, 255]);
    }

    #[test]
    fn all_good_is_the_plain_icon() {
        let colour = decode_png(include_bytes!("../assets/icon.png"));
        assert_eq!(icon(&colour, false, false), colour);
        let template = decode_png(include_bytes!("../assets/icon-template.png"));
        assert_eq!(icon(&template, false, true), template);
    }

    #[test]
    fn attention_badge_is_amber_with_a_mark() {
        let with_badge = icon(&solid(32, 32, [10, 20, 30, 255]), true, false);
        let disc = badge_disc(&solid(32, 32, [10, 20, 30, 255]), &with_badge, 5.0);
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
    fn template_badge_stays_monochrome() {
        let base = decode_png(include_bytes!("../assets/icon-template.png"));
        let attention = icon(&base, true, true);
        assert!(
            attention.pixels.chunks(4).all(|p| p[..3] == [0, 0, 0]),
            "a template pixel has colour"
        );
        assert_ne!(attention.pixels, base.pixels);
        // The badge is a solid disc with its "!" cut out.
        assert!(
            badge_disc(&base, &attention, 5.0)
                .iter()
                .any(|p| p[3] == 255)
        );
        assert!(badge_disc(&base, &attention, 5.0).iter().any(|p| p[3] == 0));
    }

    #[test]
    fn badge_leaves_a_clear_ring_around_its_disc() {
        let with_badge = icon(&solid(44, 44, [0, 0, 0, 255]), true, true);
        // Leftwards along the row through the badge's centre (31.9, 31.9), from inside its disc and clear of its "!":
        // the opaque disc, a clear gap, then the opaque base.
        let alphas: Vec<u8> = (0..=27)
            .rev()
            .map(|x| pixel(&with_badge, x, 31)[3])
            .collect();
        assert_eq!(alphas[0], 255);
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
            (30, 44, 30 * 44 * 4)
        );
        // The Rerail mark is 24 px tall, 12 pt at 2x like the menu bar's solid icons (Wi-Fi, Docker's), centred in the
        // 22 pt bar: what is more than faintly drawn runs from row 10 to row 33.
        let drawn: Vec<u32> = (0..template.height)
            .filter(|&y| {
                (0..template.width)
                    .any(|x| template.pixels[((y * template.width + x) * 4 + 3) as usize] > 32)
            })
            .collect();
        assert_eq!((drawn.first(), drawn.last()), (Some(&10), Some(&33)));
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
