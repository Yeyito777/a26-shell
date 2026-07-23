use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat};

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args()
        .nth(1)
        .ok_or("usage: a26-shellshot OUTPUT.ppm")?;
    let (conn, screen_number) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_number];
    // GetImage on the root drawable captures the visible screen including child
    // windows. Capturing the topmost child alone breaks as soon as a small
    // override-redirect surface (keyboard or volume OSD) is present.
    let window = screen.root;
    let geometry = conn.get_geometry(window)?.reply()?;
    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            window,
            0,
            0,
            geometry.width,
            geometry.height,
            u32::MAX,
        )?
        .reply()?;

    let expected = usize::from(geometry.width) * usize::from(geometry.height) * 4;
    if image.data.len() != expected {
        return Err(format!(
            "expected 32-bit XRGB data ({expected} bytes), received {}",
            image.data.len()
        )
        .into());
    }

    let mut writer = BufWriter::new(File::create(output)?);
    write!(writer, "P6\n{} {}\n255\n", geometry.width, geometry.height)?;
    for bytes in image.data.chunks_exact(4) {
        // The exact A26 Xorg target is little-endian depth-24 XRGB8888.
        writer.write_all(&[bytes[2], bytes[1], bytes[0]])?;
    }
    writer.flush()?;
    Ok(())
}
