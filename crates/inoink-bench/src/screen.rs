//! What is on the screen: a cheap signature of the framebuffer, sampled a few pixels per cell, and
//! the difference between two signatures.

use std::io;

/// A rectangle in screen pixels, `x1` and `y1` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl Rect {
    pub fn new(x0: u32, y0: u32, x1: u32, y1: u32) -> Rect {
        Rect { x0, y0, x1: x1.max(x0), y1: y1.max(y0) }
    }
    pub fn width(&self) -> u32 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> u32 {
        self.y1 - self.y0
    }
    pub fn contains(&self, o: &Rect) -> bool {
        o.x0 >= self.x0 && o.y0 >= self.y0 && o.x1 <= self.x1 && o.y1 <= self.y1
    }
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn clip(&self, w: u32, h: u32) -> Rect {
        Rect::new(self.x0.min(w), self.y0.min(h), self.x1.min(w), self.y1.min(h))
    }
    /// `x0,y0,x1,y1`
    pub fn parse(s: &str) -> Option<Rect> {
        let v: Vec<u32> = s.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
        match v[..] {
            [x0, y0, x1, y1] if x1 > x0 && y1 > y0 => Some(Rect::new(x0, y0, x1, y1)),
            _ => None,
        }
    }
}

impl std::fmt::Display for Rect {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{},{},{},{}", self.x0, self.y0, self.x1, self.y1)
    }
}

/// One hash per cell of `cell` pixels inside `region`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub region: Rect,
    pub cell: u32,
    pub cells: Vec<u64>,
}

impl Signature {
    pub fn cols(&self) -> u32 {
        self.region.width().div_ceil(self.cell)
    }
    fn cell_rect(&self, i: usize) -> Rect {
        let cols = self.cols();
        let (cx, cy) = (i as u32 % cols, i as u32 / cols);
        let x0 = self.region.x0 + cx * self.cell;
        let y0 = self.region.y0 + cy * self.cell;
        Rect::new(x0, y0, (x0 + self.cell).min(self.region.x1), (y0 + self.cell).min(self.region.y1))
    }
}

/// How much changed between two signatures of the same region, leaving out `ignore`.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub cells: usize,
    /// Changed cells over the cells compared.
    pub fraction: f64,
    pub bbox: Option<Rect>,
}

impl Change {
    pub fn any(&self) -> bool {
        self.cells > 0
    }
}

pub fn diff(a: &Signature, b: &Signature, ignore: &[Rect]) -> Change {
    assert!(a.region == b.region && a.cell == b.cell && a.cells.len() == b.cells.len(), "signatures of different regions");
    let mut cells = 0;
    let mut compared = 0;
    let mut bbox: Option<Rect> = None;
    for (i, (x, y)) in a.cells.iter().zip(&b.cells).enumerate() {
        let r = a.cell_rect(i);
        if ignore.iter().any(|g| g.contains(&r)) {
            continue;
        }
        compared += 1;
        if x != y {
            cells += 1;
            bbox = Some(match bbox {
                None => r,
                Some(b) => Rect::new(b.x0.min(r.x0), b.y0.min(r.y0), b.x1.max(r.x1), b.y1.max(r.y1)),
            });
        }
    }
    Change { cells, fraction: if compared == 0 { 0.0 } else { cells as f64 / compared as f64 }, bbox }
}

pub trait Screen {
    fn size(&self) -> (u32, u32);
    fn signature(&mut self, region: Rect, cell: u32) -> io::Result<Signature>;
}

/// The positions sampled in a cell: four points, a quarter in from each side.
fn samples(x0: u32, y0: u32, x1: u32, y1: u32) -> [(u32, u32); 4] {
    let (w, h) = (x1 - x0, y1 - y0);
    let (qx, qy) = (x0 + w / 4, y0 + h / 4);
    let (tx, ty) = (x0 + (3 * w) / 4, y0 + (3 * h) / 4);
    [(qx, qy), (tx, qy), (qx, ty), (tx, ty)]
}

fn fnv(h: u64, v: u32) -> u64 {
    (h ^ v as u64).wrapping_mul(0x100000001b3)
}

/// Builds a signature from any pixel reader.
pub fn sign(region: Rect, cell: u32, mut pixel: impl FnMut(u32, u32) -> u32) -> Signature {
    let cell = cell.max(1);
    let cols = region.width().div_ceil(cell);
    let rows = region.height().div_ceil(cell);
    let mut cells = Vec::with_capacity((cols * rows) as usize);
    for cy in 0..rows {
        for cx in 0..cols {
            let x0 = region.x0 + cx * cell;
            let y0 = region.y0 + cy * cell;
            let (x1, y1) = ((x0 + cell).min(region.x1), (y0 + cell).min(region.y1));
            let mut h = 0xcbf29ce484222325;
            for (x, y) in samples(x0, y0, x1, y1) {
                h = fnv(h, pixel(x, y));
            }
            cells.push(h);
        }
    }
    Signature { region, cell, cells }
}

/// The Kobo's framebuffer, mapped read only. InoInk draws into it before each display update, so
/// a change here is the moment the new screen is ready (the e-ink refresh follows).
#[cfg(target_os = "linux")]
pub struct Framebuffer {
    ptr: *const u8,
    len: usize,
    /// Where the visible screen starts (the panning offset InoInk draws at).
    start: usize,
    stride: usize,
    bytes: usize,
    width: u32,
    height: u32,
}

#[cfg(target_os = "linux")]
mod sys {
    use std::os::raw::{c_int, c_long, c_void};
    pub const PROT_READ: c_int = 1;
    pub const MAP_SHARED: c_int = 1;
    unsafe extern "C" {
        pub fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, off: c_long) -> *mut c_void;
        pub fn munmap(addr: *mut c_void, len: usize) -> c_int;
    }
}

#[cfg(target_os = "linux")]
impl Framebuffer {
    /// `width` and `height` are the visible screen in its current orientation.
    pub fn open(width: u32, height: u32) -> io::Result<Framebuffer> {
        use std::os::unix::io::AsRawFd;
        let read = |name: &str| std::fs::read_to_string(format!("/sys/class/graphics/fb0/{name}")).map(|s| s.trim().to_string());
        let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, format!("unexpected framebuffer {what}"));
        let stride: usize = read("stride")?.parse().map_err(|_| bad("stride"))?;
        let bpp: usize = read("bits_per_pixel")?.parse().map_err(|_| bad("depth"))?;
        if !matches!(bpp, 8 | 16 | 24 | 32) {
            return Err(bad("depth"));
        }
        let bytes = bpp / 8;
        if stride < width as usize * bytes {
            return Err(bad("stride"));
        }
        let yoffset: usize = read("pan").ok().and_then(|p| p.split(',').nth(1)?.trim().parse().ok()).unwrap_or(0);
        let start = yoffset * stride;
        let file = std::fs::File::open("/dev/fb0")?;
        let len = start + stride * height as usize;
        let ptr = unsafe { sys::mmap(std::ptr::null_mut(), len, sys::PROT_READ, sys::MAP_SHARED, file.as_raw_fd(), 0) };
        if ptr as isize == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Framebuffer { ptr: ptr as *const u8, len, start, stride, bytes, width, height })
    }

    fn pixel(&self, x: u32, y: u32) -> u32 {
        // One aligned word around the pixel: cheap on uncached video memory.
        let off = (self.start + y as usize * self.stride + x as usize * self.bytes) & !3;
        if off + 4 > self.len {
            return 0;
        }
        unsafe { std::ptr::read_volatile(self.ptr.add(off) as *const u32) }
    }
}

#[cfg(target_os = "linux")]
impl Drop for Framebuffer {
    fn drop(&mut self) {
        unsafe {
            sys::munmap(self.ptr as *mut _, self.len);
        }
    }
}

#[cfg(target_os = "linux")]
impl Screen for Framebuffer {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn signature(&mut self, region: Rect, cell: u32) -> io::Result<Signature> {
        let region = region.clip(self.width, self.height);
        Ok(sign(region, cell, |x, y| self.pixel(x, y)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_parsing() {
        assert_eq!(Rect::parse("1, 2,30,40"), Some(Rect::new(1, 2, 30, 40)));
        for bad in ["", "1,2,3", "5,5,5,6", "a,b,c,d", "1,2,3,4,5"] {
            assert_eq!(Rect::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn diff_counts_cells_and_bounds_them() {
        let region = Rect::new(0, 0, 100, 50);
        let mut img = vec![0u32; 100 * 50];
        let a = sign(region, 10, |x, y| img[(y * 100 + x) as usize]);
        assert_eq!(a.cells.len(), 50);
        // A 6-pixel dot inside one cell is seen by the samples a quarter in from each side.
        for y in 22..28 {
            for x in 42..48 {
                img[(y * 100 + x) as usize] = 1;
            }
        }
        let b = sign(region, 10, |x, y| img[(y * 100 + x) as usize]);
        let c = diff(&a, &b, &[]);
        assert_eq!(c.cells, 1);
        assert_eq!(c.bbox, Some(Rect::new(40, 20, 50, 30)));
        assert!((c.fraction - 0.02).abs() < 1e-9);
        // Ignored regions are left out of both the count and the comparison.
        let c = diff(&a, &b, &[Rect::new(40, 20, 50, 30)]);
        assert!(!c.any());
        assert!(!diff(&a, &a, &[]).any());
    }

    #[test]
    fn partial_cells_at_the_edges() {
        let s = sign(Rect::new(5, 5, 28, 12), 10, |x, y| x + y);
        assert_eq!(s.cols(), 3);
        assert_eq!(s.cells.len(), 3);
        assert_eq!(s.cell_rect(2), Rect::new(25, 5, 28, 12));
    }
}
