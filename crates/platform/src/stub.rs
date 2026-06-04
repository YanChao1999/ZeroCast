use anyhow::Result;

pub struct StubCapturer {
    width: u32,
    height: u32,
    frame_index: u64,
}

impl StubCapturer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            frame_index: 0,
        }
    }

    pub fn set_output_size(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    pub fn capture_frame(&mut self) -> Result<Vec<u8>> {
        let idx = self.frame_index;
        self.frame_index = self.frame_index.wrapping_add(1);
        let mut buf = vec![0u8; (self.width * self.height * 3) as usize];
        let r = ((idx & 0xff) as u8).wrapping_mul(3);
        let g = (((idx >> 8) & 0xff) as u8).wrapping_mul(7);
        let b = (((idx >> 16) & 0xff) as u8).wrapping_mul(11);
        for px in 0..(self.width * self.height) {
            let i = (px * 3) as usize;
            buf[i] = r;
            buf[i + 1] = g;
            buf[i + 2] = b;
        }
        Ok(buf)
    }
}
