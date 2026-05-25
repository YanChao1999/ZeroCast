use anyhow::Result;

/// Very small capture stub for the prototype.
/// Returns a single RGB24 frame (width*height*3) filled with a color pattern.
pub fn capture_frame(width: u32, height: u32, frame_index: u64) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; (width * height * 3) as usize];
    // simple moving color pattern so frames differ
    let r = ((frame_index & 0xff) as u8).wrapping_mul(3);
    let g = (((frame_index >> 8) & 0xff) as u8).wrapping_mul(7);
    let b = (((frame_index >> 16) & 0xff) as u8).wrapping_mul(11);
    for px in 0..(width * height) {
        let i = (px * 3) as usize;
        buf[i] = r;
        buf[i + 1] = g;
        buf[i + 2] = b;
    }
    Ok(buf)
}
