//! Generate the 1024x1024 source PNG that `tauri icon` expands into every
//! platform-specific icon format. Run with:
//!
//! ```text
//! cargo run --example gen_icon
//! pnpm tauri icon src-tauri/icons/source.png
//! ```
//!
//! The output is an ASCII-pixel-art flame: a pure upward triangle rendered
//! on a 32x32 grid and scaled up 32x with nearest-neighbor, giving chunky
//! block-art edges that read well at both the 32x32 and 256x256 sizes the
//! bundler targets.
//!
//! Palette matches the in-app theme (`#0a0a0a` bg, `#00b3ff` accent,
//! `#5de0ff` hot tip) so the icon reads as part of BlueFlame's identity
//! in the taskbar, dock, and alt-tab switcher.

use image::{ImageBuffer, Rgba, RgbaImage};

const SIZE: u32 = 1024;
const GRID: u32 = 32;
const CELL: u32 = SIZE / GRID; // 32px per ASCII cell

const BG: Rgba<u8> = Rgba([0x0a, 0x0a, 0x0a, 0xff]);
const ACCENT: Rgba<u8> = Rgba([0x00, 0xb3, 0xff, 0xff]);
const ACCENT_BRIGHT: Rgba<u8> = Rgba([0x5d, 0xe0, 0xff, 0xff]);

fn main() -> anyhow::Result<()> {
    let grid = build_flame_grid();

    let mut img: RgbaImage = ImageBuffer::from_pixel(SIZE, SIZE, BG);
    for (gy, row) in grid.iter().enumerate() {
        for (gx, &on) in row.iter().enumerate() {
            if !on {
                continue;
            }
            // Hot tip at the top fades to the main accent at the base.
            let t = gy as f32 / (GRID - 1) as f32;
            let color = lerp(ACCENT_BRIGHT, ACCENT, t);
            fill_cell(&mut img, gx as u32, gy as u32, color);
        }
    }

    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("icons");
    std::fs::create_dir_all(&out_dir)?;
    let path = out_dir.join("source.png");
    img.save(&path)?;
    println!("wrote {}", path.display());
    println!("now run: pnpm tauri icon src-tauri/icons/source.png");
    Ok(())
}

/// A centered upward-pointing triangle with a symmetric flicker tip.
/// Using a 32x32 ASCII grid keeps the shape chunky and readable at 32x32.
fn build_flame_grid() -> [[bool; 32]; 32] {
    let mut grid = [[false; 32]; 32];
    let center = 16i32;

    // Tip: single 2-wide pixel at row 4.
    for x in (center - 1)..(center + 1) {
        grid[4][x as usize] = true;
    }

    // Body: widen by 1 each side every 2 rows so the edges stay crisp when
    // scaled up and the triangle feels bold rather than spiky.
    for (offset, row) in grid.iter_mut().enumerate().take(29).skip(5) {
        let half = (((offset - 4) / 2) + 1) as i32;
        let x0 = (center - half).max(0) as usize;
        let x1 = (center + half).min(32) as usize;
        for cell in row.iter_mut().take(x1).skip(x0) {
            *cell = true;
        }
    }

    // Flat base to anchor the shape.
    for base_row in &mut [29usize, 30usize] {
        for cell in grid[*base_row].iter_mut().take(30).skip(2) {
            *cell = true;
        }
    }

    grid
}

fn fill_cell(img: &mut RgbaImage, gx: u32, gy: u32, color: Rgba<u8>) {
    let x0 = gx * CELL;
    let y0 = gy * CELL;
    for py in y0..(y0 + CELL) {
        for px in x0..(x0 + CELL) {
            img.put_pixel(px, py, color);
        }
    }
}

fn lerp(a: Rgba<u8>, b: Rgba<u8>, t: f32) -> Rgba<u8> {
    let t = t.clamp(0.0, 1.0);
    Rgba([
        (a[0] as f32 * (1.0 - t) + b[0] as f32 * t) as u8,
        (a[1] as f32 * (1.0 - t) + b[1] as f32 * t) as u8,
        (a[2] as f32 * (1.0 - t) + b[2] as f32 * t) as u8,
        0xff,
    ])
}
