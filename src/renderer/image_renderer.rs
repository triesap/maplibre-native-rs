use std::f64::consts::PI;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::path::Path;

use cxx::UniquePtr;
use image::{ImageBuffer, Rgba};

use crate::renderer::bridge::ffi;
use crate::renderer::MapDebugOptions;

/// A rendered map image.
///
/// The image is stored as RGBA pixel data using the `image` crate.
/// Use [`as_image`](Image::as_image) to access the underlying `ImageBuffer` for all image operations.
///
/// # Example
///
/// ```no_run
/// # fn foo() {
/// use maplibre_native::{ImageRendererBuilder, Image};
///
/// let renderer = ImageRendererBuilder::new()
///     .with_size(512, 512)
///     .build_static_renderer();
///
/// renderer.load_style_from_url(&"https://demotiles.maplibre.org/style.json".parse().unwrap());
/// let image: Image = renderer.render_static(0.0, 0.0, 0.0, 0.0, 0.0).unwrap();
///
/// // Access the underlying ImageBuffer for all operations
/// let img_buffer = image.as_image();
/// println!("Image dimensions: {}x{}", img_buffer.width(), img_buffer.height());
/// img_buffer.save("map.png").unwrap();
/// # }
/// ```
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Image(ImageBuffer<Rgba<u8>, Vec<u8>>);

impl Image {
    /// Create an Image from raw RGBA data
    pub(crate) fn from_raw(bytes: &[u8]) -> Option<Self> {
        // Parse dimensions from first 8 bytes
        if bytes.len() < 8 {
            return None;
        }

        let width = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let height = u32::from_ne_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let data = bytes[8..].to_vec();
        ImageBuffer::from_vec(width, height, data).map(Image)
    }

    /// Get access to the underlying image buffer.
    /// Use this to perform any image operations using the `image` crate.
    #[must_use]
    pub fn as_image(&self) -> &ImageBuffer<Rgba<u8>, Vec<u8>> {
        &self.0
    }
}

/// Internal state type to render a static map image.
#[derive(Debug)]
pub struct Static;
/// Internal state type to render a map tile.
#[derive(Debug)]
pub struct Tile;

/// Map projection mode for static rendering.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MapProjectionType {
    /// Render in standard Web Mercator.
    #[default]
    Mercator,
    /// Render in software-projected globe mode.
    Globe,
}

/// Configuration options for a tile server.
pub struct ImageRenderer<S> {
    pub(crate) instance: UniquePtr<ffi::MapRenderer>,
    pub(crate) _marker: PhantomData<S>,
    pub(crate) style_specified: bool,
    pub(crate) map_projection: MapProjectionType,
}

impl<S> Debug for ImageRenderer<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageRenderer")
            .field("style_specified", &self.style_specified)
            .finish_non_exhaustive()
    }
}

impl<S> ImageRenderer<S> {
    /// Set the style URL for the map.
    pub fn load_style_from_url(&mut self, url: &url::Url) -> &mut Self {
        self.style_specified = true;
        ffi::MapRenderer_getStyle_loadURL(self.instance.pin_mut(), url.as_ref());
        self
    }

    /// Load the style from the specified path.
    ///
    /// The style will be loaded from the path, but won't be refreshed automatically if the file changes
    ///
    /// # Errors
    /// Returns an error if the path is not a valid file.
    pub fn load_style_from_path(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<&mut Self, std::io::Error> {
        let path = path.as_ref();
        if !path.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Path {} is not a file", path.display()),
            ));
        }
        let Some(path) = path.to_str() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Path {} is not valid UTF-8", path.display()),
            ));
        };
        self.style_specified = true;
        ffi::MapRenderer_getStyle_loadURL(self.instance.pin_mut(), &format!("file://{path}"));
        Ok(self)
    }

    /// Set debug visualization flags for the map renderer.
    pub fn set_debug_flags(&mut self, flags: MapDebugOptions) -> &mut Self {
        ffi::MapRenderer_setDebugFlags(self.instance.pin_mut(), flags);
        self
    }

    /// Sets map projection mode.
    pub fn set_projection(&mut self, projection: MapProjectionType) -> &mut Self {
        self.map_projection = projection;
        self
    }

    /// Returns map projection mode.
    #[must_use]
    pub fn projection(&self) -> MapProjectionType {
        self.map_projection
    }
}

impl ImageRenderer<Static> {
    /// Render the map as a static [`Image`] where the camera can be freely controlled.
    ///
    /// # Errors
    /// Returns an error if
    /// - the style has not been specified via either [`load_style_from_path`](Self::load_style_from_path) or [`load_style_from_url`](Self::load_style_from_url).
    pub fn render_static(
        &mut self,
        lat: f64,
        lon: f64,
        zoom: f64,
        bearing: f64,
        pitch: f64,
    ) -> Result<Image, RenderingError> {
        if !self.style_specified {
            return Err(RenderingError::StyleNotSpecified);
        }

        if self.map_projection == MapProjectionType::Globe {
            ffi::MapRenderer_setCamera(self.instance.pin_mut(), lat, lon, zoom, 0.0, 0.0);
        } else {
            ffi::MapRenderer_setCamera(self.instance.pin_mut(), lat, lon, zoom, bearing, pitch);
        }
        let data = ffi::MapRenderer_render(self.instance.pin_mut());
        let bytes = data.as_bytes();

        let mut image = Image::from_raw(bytes).ok_or(RenderingError::InvalidImageData)?;
        if self.map_projection == MapProjectionType::Globe {
            image = apply_globe_projection(&image, lat, lon, zoom, bearing, pitch);
        }
        Ok(image)
    }
}

impl ImageRenderer<Tile> {
    /// Render a top-down tile of the map as a static [`Image`].
    ///
    /// # Errors
    /// Returns an error if
    /// - the style has not been specified via either [`load_style_from_path`](Self::load_style_from_path) or [`load_style_from_url`](Self::load_style_from_url).
    pub fn render_tile(&mut self, zoom: u8, x: u32, y: u32) -> Result<Image, RenderingError> {
        if !self.style_specified {
            return Err(RenderingError::StyleNotSpecified);
        }

        let (lat, lon) = coords_to_lat_lon(f64::from(zoom), x, y);
        ffi::MapRenderer_setCamera(self.instance.pin_mut(), lat, lon, f64::from(zoom), 0.0, 0.0);

        let data = ffi::MapRenderer_render(self.instance.pin_mut());
        let bytes = data.as_bytes();
        let image = Image::from_raw(bytes).ok_or(RenderingError::InvalidImageData)?;
        Ok(image)
    }
}

#[allow(clippy::cast_precision_loss)]
fn coords_to_lat_lon(zoom: f64, x: u32, y: u32) -> (f64, f64) {
    // https://github.com/oldmammuth/slippy_map_tilenames/blob/058678480f4b50b622cda7a48b98647292272346/src/lib.rs#L114
    let zz = 2_f64.powf(zoom);
    let lng = (f64::from(x) + 0.5) / zz * 360_f64 - 180_f64;
    let lat = ((PI * (1_f64 - 2_f64 * (f64::from(y) + 0.5) / zz)).sinh())
        .atan()
        .to_degrees();
    (lat, lng)
}

fn apply_globe_projection(
    image: &Image,
    center_lat: f64,
    center_lon: f64,
    zoom: f64,
    bearing: f64,
    pitch: f64,
) -> Image {
    let source = image.as_image();
    let width = source.width();
    let height = source.height();
    let cx = f64::from(width) * 0.5;
    let cy = f64::from(height) * 0.5;
    let radius = f64::from(width.min(height)) * 0.5;

    let center_lat = center_lat.clamp(-MAX_MERCATOR_LAT, MAX_MERCATOR_LAT);
    let center_lon = wrap_longitude(center_lon);

    let bearing_rad = bearing.to_radians();
    let pitch_rad = pitch.to_radians();
    let center_lat_rad = center_lat.to_radians();
    let center_lon_rad = center_lon.to_radians();

    let center_world_x = longitude_to_world_x(center_lon, zoom);
    let center_world_y = latitude_to_world_y(center_lat, zoom);

    let projected = ImageBuffer::from_fn(width, height, |x, y| {
        let nx = (f64::from(x) + 0.5 - cx) / radius;
        let ny = (f64::from(y) + 0.5 - cy) / radius;
        let radius_sq = nx * nx + ny * ny;
        if radius_sq > 1.0 {
            return Rgba([0, 0, 0, 0]);
        }

        let nz = (1.0 - radius_sq).sqrt();
        let mut v = (nx, -ny, nz);

        v = rotate_z(v, bearing_rad);
        v = rotate_x(v, pitch_rad);
        v = rotate_x(v, -center_lat_rad);
        v = rotate_y(v, center_lon_rad);

        let lat =
            v.1.asin()
                .to_degrees()
                .clamp(-MAX_MERCATOR_LAT, MAX_MERCATOR_LAT);
        let lon = wrap_longitude(v.0.atan2(v.2).to_degrees());

        let world_x = longitude_to_world_x(lon, zoom);
        let world_y = latitude_to_world_y(lat, zoom);
        let sample_x = world_x - center_world_x + cx;
        let sample_y = world_y - center_world_y + cy;

        sample_image(source, sample_x, sample_y)
    });

    Image(projected)
}

fn rotate_x(v: (f64, f64, f64), angle: f64) -> (f64, f64, f64) {
    let (x, y, z) = v;
    let cos = angle.cos();
    let sin = angle.sin();
    (x, y * cos - z * sin, y * sin + z * cos)
}

fn rotate_y(v: (f64, f64, f64), angle: f64) -> (f64, f64, f64) {
    let (x, y, z) = v;
    let cos = angle.cos();
    let sin = angle.sin();
    (x * cos + z * sin, y, -x * sin + z * cos)
}

fn rotate_z(v: (f64, f64, f64), angle: f64) -> (f64, f64, f64) {
    let (x, y, z) = v;
    let cos = angle.cos();
    let sin = angle.sin();
    (x * cos - y * sin, x * sin + y * cos, z)
}

fn sample_image(image: &ImageBuffer<Rgba<u8>, Vec<u8>>, x: f64, y: f64) -> Rgba<u8> {
    let wrapped_x = wrap_pixel(x, f64::from(image.width()));
    let clamped_y = y.clamp(0.0, f64::from(image.height().saturating_sub(1)));
    let px = wrapped_x.floor() as u32;
    let py = clamped_y.floor() as u32;
    *image.get_pixel(px, py)
}

fn wrap_pixel(x: f64, size: f64) -> f64 {
    if size <= 0.0 {
        return 0.0;
    }
    x.rem_euclid(size)
}

fn wrap_longitude(lon: f64) -> f64 {
    let wrapped = (lon + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped == -180.0 {
        180.0
    } else {
        wrapped
    }
}

fn longitude_to_world_x(lon: f64, zoom: f64) -> f64 {
    let world_size = 512.0 * 2f64.powf(zoom.max(0.0));
    (wrap_longitude(lon) + 180.0) / 360.0 * world_size
}

fn latitude_to_world_y(lat: f64, zoom: f64) -> f64 {
    let world_size = 512.0 * 2f64.powf(zoom.max(0.0));
    let clamped = lat.clamp(-MAX_MERCATOR_LAT, MAX_MERCATOR_LAT);
    let lat_rad = clamped.to_radians();
    let y = (1.0 - ((lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / PI)) * 0.5;
    y * world_size
}

const MAX_MERCATOR_LAT: f64 = 85.051_128_78;

/// Errors that can occur during map rendering operations.
#[derive(thiserror::Error, Debug)]
pub enum RenderingError {
    /// Style must be specified before rendering can occur.
    #[error("Style must be specified to render a tile")]
    StyleNotSpecified,
    /// The renderer returned invalid or corrupted image data.
    #[error("Invalid image data received from renderer")]
    InvalidImageData,
}

#[cfg(test)]
mod tests {
    use super::{apply_globe_projection, Image};
    use image::{ImageBuffer, Rgba};

    #[test]
    fn globe_projection_masks_outer_pixels() {
        let source = Image(ImageBuffer::from_pixel(64, 64, Rgba([255, 255, 255, 255])));
        let projected = apply_globe_projection(&source, 0.0, 0.0, 1.5, 0.0, 0.0);
        let image = projected.as_image();

        assert_eq!(image.get_pixel(0, 0)[3], 0);
        assert_eq!(image.get_pixel(63, 0)[3], 0);
        assert_eq!(image.get_pixel(0, 63)[3], 0);
        assert_eq!(image.get_pixel(63, 63)[3], 0);
    }

    #[test]
    fn globe_projection_keeps_center_visible() {
        let source = Image(ImageBuffer::from_pixel(64, 64, Rgba([17, 34, 51, 255])));
        let projected = apply_globe_projection(&source, 0.0, 0.0, 1.5, 0.0, 0.0);
        let image = projected.as_image();
        let center = image.get_pixel(32, 32);
        assert_eq!(center[0], 17);
        assert_eq!(center[1], 34);
        assert_eq!(center[2], 51);
        assert_eq!(center[3], 255);
    }
}
