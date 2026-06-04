//! Synthetic RGB24 patterns for integration tests and `stream --test-cycle`.

pub const CYCLE_COUNT: u32 = 10;

/// Distinct RGB24 frame for `cycle % 10` (solid fill + tag strip on row 0).
pub fn test_cycle_frame(width: u32, height: u32, cycle: u32) -> Vec<u8> {
    let cycle = cycle % CYCLE_COUNT;
    let (r, g, b) = cycle_palette(cycle);
    let w = width as usize;
    let h = height as usize;
    let mut buf = vec![0u8; w * h * 3];
    for px in buf.chunks_exact_mut(3) {
        px[0] = r;
        px[1] = g;
        px[2] = b;
    }
    let bar_w = w.min(40 + cycle as usize * 4);
    for x in 0..bar_w {
        let o = x * 3;
        buf[o] = (cycle.wrapping_mul(23) + 40) as u8;
        buf[o + 1] = (cycle.wrapping_mul(37) + 40) as u8;
        buf[o + 2] = (cycle.wrapping_mul(53) + 40) as u8;
    }
    buf
}

pub fn cycle_palette(cycle: u32) -> (u8, u8, u8) {
    [
        (210, 20, 20),
        (20, 210, 20),
        (20, 20, 210),
        (210, 210, 20),
        (210, 20, 210),
        (20, 210, 210),
        (140, 60, 210),
        (210, 140, 60),
        (60, 210, 140),
        (180, 180, 180),
    ][(cycle % CYCLE_COUNT) as usize]
}

pub fn frame_mean_rgb(rgb: &[u8]) -> (u8, u8, u8) {
    let n = (rgb.len() / 3).max(1) as u64;
    let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
    for px in rgb.chunks_exact(3) {
        r += px[0] as u64;
        g += px[1] as u64;
        b += px[2] as u64;
    }
    ((r / n) as u8, (g / n) as u8, (b / n) as u8)
}

fn color_distance(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let dr = a.0 as i32 - b.0 as i32;
    let dg = a.1 as i32 - b.1 as i32;
    let db = a.2 as i32 - b.2 as i32;
    (dr * dr + dg * dg + db * db) as u32
}

/// Match decoded RGB to nearest cycle index (lossy H.264 safe).
pub fn decoded_cycle_from_rgb(rgb: &[u8]) -> u32 {
    let mean = frame_mean_rgb(rgb);
    (0..CYCLE_COUNT)
        .min_by_key(|c| color_distance(mean, cycle_palette(*c)))
        .unwrap()
}
