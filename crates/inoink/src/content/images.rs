//! Bounded image download and normalization.

use crate::error::{Error, Result};
use image::{DynamicImage, ImageFormat, ImageReader};
use std::io::Cursor;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use url::Url;

#[derive(Clone, Debug)]
pub struct ImageLimits {
    /// Largest compressed download.
    pub max_bytes: usize,
    /// Largest decoded size in pixels.
    pub max_pixels: u64,
    /// Larger images are downscaled to fit.
    pub max_width: u32,
    pub max_height: u32,
    pub max_redirects: usize,
    /// Transient failures are retried up to this many attempts in total.
    pub attempts: usize,
}

impl Default for ImageLimits {
    fn default() -> Self {
        ImageLimits {
            max_bytes: 16 * 1024 * 1024,
            max_pixels: 32_000_000,
            max_width: 1800,
            max_height: 2400,
            max_redirects: 5,
            attempts: 3,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Fetched {
    pub status: u16,
    pub content_type: Option<String>,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

/// One HTTP GET without following redirects and without any credentials.
pub trait Fetcher: Send + Sync {
    fn get(&self, url: &Url, max_bytes: usize) -> Result<Fetched>;
}

pub fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            v.is_loopback()
                || v.is_private()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_multicast()
                || v.octets()[0] == 0
                || (v.octets()[0] == 100 && (v.octets()[1] & 0xc0) == 64) // 100.64/10 CGNAT
        }
        IpAddr::V6(v) => {
            if let Some(m) = v.to_ipv4_mapped() {
                return is_private_ip(&IpAddr::V4(m));
            }
            v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || (v.segments()[0] & 0xfe00) == 0xfc00
                || (v.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// True for URLs that must never be fetched: non-http(s), local names and private addresses.
pub fn is_blocked_url(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return true;
    }
    match url.host() {
        None => true,
        Some(url::Host::Ipv4(ip)) => is_private_ip(&IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => is_private_ip(&IpAddr::V6(ip)),
        Some(url::Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            d == "localhost"
                || d.ends_with(".localhost")
                || d.ends_with(".local")
                || d.ends_with(".internal")
                || d.ends_with(".lan")
                || d.ends_with(".home.arpa")
                || !d.contains('.')
        }
    }
}

/// Follows redirects manually, re-validating every hop (redirect-based SSRF).
pub fn fetch_following(f: &dyn Fetcher, start: &Url, limits: &ImageLimits, cancel: &AtomicBool) -> Result<Fetched> {
    let mut url = start.clone();
    for _ in 0..=limits.max_redirects {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Retryable("cancelled".into()));
        }
        if is_blocked_url(&url) {
            return Err(Error::InvalidRequest("blocked address".into()));
        }
        let r = f.get(&url, limits.max_bytes)?;
        match r.status {
            200..=299 => return Ok(r),
            301 | 302 | 303 | 307 | 308 => {
                let loc = r.location.ok_or_else(|| Error::InvalidRequest("redirect without location".into()))?;
                url = url.join(&loc).map_err(|_| Error::InvalidRequest("bad redirect".into()))?;
            }
            408 | 425 | 429 | 500..=599 => return Err(Error::Retryable(format!("status {}", r.status))),
            s => return Err(Error::InvalidRequest(format!("status {s}"))),
        }
    }
    Err(Error::InvalidRequest("too many redirects".into()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Normalized {
    pub bytes: Vec<u8>,
    /// `png` or `jpg`.
    pub ext: &'static str,
    pub width: u32,
    pub height: u32,
}

/// Validates by magic bytes and size limits, then keeps PNG/JPEG as they are (unless too large)
/// and converts GIF/WebP to PNG (first frame). Everything else is rejected.
pub fn normalize_image(bytes: &[u8], limits: &ImageLimits) -> Result<Normalized> {
    let format = image::guess_format(bytes).map_err(|_| Error::Unsupported("unrecognized image format".into()))?;
    if !matches!(format, ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP) {
        return Err(Error::Unsupported("unsupported image format".into()));
    }
    let reader = ImageReader::with_format(Cursor::new(bytes), format);
    let (w, h) = reader.into_dimensions().map_err(|e| Error::Data(format!("bad image: {e}")))?;
    if w == 0 || h == 0 || (w as u64) * (h as u64) > limits.max_pixels {
        return Err(Error::Unsupported("image is too large".into()));
    }
    let direct = matches!(format, ImageFormat::Png | ImageFormat::Jpeg);
    if direct && w <= limits.max_width && h <= limits.max_height {
        // Make sure it actually decodes before we publish it.
        decode(bytes, format)?;
        let ext = if format == ImageFormat::Png { "png" } else { "jpg" };
        return Ok(Normalized { bytes: bytes.to_vec(), ext, width: w, height: h });
    }
    let mut img = decode(bytes, format)?;
    if w > limits.max_width || h > limits.max_height {
        img = img.resize(limits.max_width, limits.max_height, image::imageops::FilterType::Triangle);
    }
    let (ow, oh) = (img.width(), img.height());
    let mut out = Vec::new();
    if format == ImageFormat::Jpeg {
        let rgb = DynamicImage::ImageRgb8(img.to_rgb8());
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85);
        rgb.write_with_encoder(enc).map_err(|e| Error::Data(format!("encode failed: {e}")))?;
        Ok(Normalized { bytes: out, ext: "jpg", width: ow, height: oh })
    } else {
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).map_err(|e| Error::Data(format!("encode failed: {e}")))?;
        Ok(Normalized { bytes: out, ext: "png", width: ow, height: oh })
    }
}

fn decode(bytes: &[u8], format: ImageFormat) -> Result<DynamicImage> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|e| Error::Data(format!("bad image: {e}")))
}

#[derive(Debug, PartialEq, Eq)]
pub enum DownloadError {
    /// Will not succeed on retry (blocked, unsupported, too large, 4xx).
    Permanent(String),
    /// Exhausted retries on network/server trouble; worth retrying later.
    Transient(String),
    Cancelled,
}

/// Downloads and normalizes one image with bounded retries.
pub fn download_image(
    f: &dyn Fetcher,
    url: &Url,
    limits: &ImageLimits,
    cancel: &AtomicBool,
    sleep: &dyn Fn(Duration),
) -> std::result::Result<Normalized, DownloadError> {
    let mut last = String::from("failed");
    for attempt in 0..limits.attempts.max(1) {
        if cancel.load(Ordering::Relaxed) {
            return Err(DownloadError::Cancelled);
        }
        match fetch_following(f, url, limits, cancel) {
            Ok(r) => {
                if r.body.len() > limits.max_bytes {
                    return Err(DownloadError::Permanent("image is too large".into()));
                }
                return normalize_image(&r.body, limits).map_err(|e| DownloadError::Permanent(e.to_string()));
            }
            Err(Error::Offline) => {
                last = "offline".into();
            }
            Err(Error::Retryable(m)) if m == "cancelled" => return Err(DownloadError::Cancelled),
            Err(Error::Retryable(m)) | Err(Error::UnknownOutcome(m)) => {
                last = m;
            }
            Err(e) => return Err(DownloadError::Permanent(e.to_string())),
        }
        if attempt + 1 < limits.attempts {
            sleep(Duration::from_millis(500 * (1 << attempt)));
        }
    }
    Err(DownloadError::Transient(last))
}

/// A resolver that refuses to connect to private addresses, closing the DNS-rebinding gap that
/// name-based checks leave open.
#[derive(Debug, Default)]
struct GuardedResolver {
    inner: ureq::unversioned::resolver::DefaultResolver,
}

impl ureq::unversioned::resolver::Resolver for GuardedResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> std::result::Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let all = self.inner.resolve(uri, config, timeout)?;
        let mut out = self.empty();
        for a in all.iter() {
            if !is_private_ip(&a.ip()) {
                out.push(*a);
            }
        }
        if out.is_empty() {
            return Err(ureq::Error::HostNotFound);
        }
        Ok(out)
    }
}

/// Real fetcher: 10 s connect, 30 s total, no redirects, no credentials, public addresses only.
pub struct UreqFetcher {
    agent: ureq::Agent,
}

impl UreqFetcher {
    pub fn new(user_agent: &str) -> UreqFetcher {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(user_agent)
            .build();
        let agent = ureq::Agent::with_parts(config, ureq::unversioned::transport::DefaultConnector::default(), GuardedResolver::default());
        UreqFetcher { agent }
    }
}

impl Fetcher for UreqFetcher {
    fn get(&self, url: &Url, max_bytes: usize) -> Result<Fetched> {
        let mut resp = self.agent.get(url.as_str()).call().map_err(|e| match e {
            ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => Error::Offline,
            ureq::Error::Timeout(_) => Error::Retryable("the site did not answer in time".into()),
            ureq::Error::Io(e) => Error::Retryable(format!("network error ({})", e.kind())),
            other => Error::Retryable(other.to_string().chars().take(60).collect()),
        })?;
        let status = resp.status().as_u16();
        let content_type = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).map(String::from);
        let location = resp.headers().get("location").and_then(|v| v.to_str().ok()).map(String::from);
        let body = if (200..300).contains(&status) {
            let b = resp
                .body_mut()
                .with_config()
                .limit(max_bytes as u64 + 1)
                .read_to_vec()
                .map_err(|_| Error::Retryable("read failed".into()))?;
            if b.len() > max_bytes {
                return Err(Error::Unsupported("download is too large".into()));
            }
            b
        } else {
            Vec::new()
        };
        Ok(Fetched { status, content_type, location, body })
    }
}
