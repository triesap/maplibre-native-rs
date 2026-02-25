//! Integration tests for projection behavior in static rendering mode.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use maplibre_native::{ImageRendererBuilder, MapProjectionType};

fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("projection test lock should be available")
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn alpha_at_corners(image: &image::ImageBuffer<image::Rgba<u8>, Vec<u8>>) -> [u8; 4] {
    let max_x = image.width().saturating_sub(1);
    let max_y = image.height().saturating_sub(1);
    [
        image.get_pixel(0, 0)[3],
        image.get_pixel(max_x, 0)[3],
        image.get_pixel(0, max_y)[3],
        image.get_pixel(max_x, max_y)[3],
    ]
}

#[test]
fn mercator_projection_keeps_full_coverage() {
    let _guard = test_lock();

    let mut renderer = ImageRendererBuilder::new()
        .with_size(
            NonZeroU32::new(128).unwrap(),
            NonZeroU32::new(128).unwrap(),
        )
        .with_projection(MapProjectionType::Mercator)
        .build_static_renderer();

    renderer
        .load_style_from_path(fixture_path("test-style.json"))
        .unwrap();
    let rendered = renderer.render_static(0.0, 0.0, 1.5, 0.0, 0.0).unwrap();
    assert_eq!(alpha_at_corners(rendered.as_image()), [255, 255, 255, 255]);
}

#[test]
fn globe_projection_applies_round_mask() {
    let _guard = test_lock();

    let mut renderer = ImageRendererBuilder::new()
        .with_size(
            NonZeroU32::new(128).unwrap(),
            NonZeroU32::new(128).unwrap(),
        )
        .with_projection(MapProjectionType::Globe)
        .build_static_renderer();

    renderer
        .load_style_from_path(fixture_path("test-style.json"))
        .unwrap();
    let rendered = renderer.render_static(0.0, 0.0, 1.5, 0.0, 0.0).unwrap();
    let corners = alpha_at_corners(rendered.as_image());
    assert_eq!(corners, [0, 0, 0, 0]);
    assert_eq!(rendered.as_image().get_pixel(64, 64)[3], 255);
}

#[test]
fn projection_setter_updates_output_mode() {
    let _guard = test_lock();

    let mut renderer = ImageRendererBuilder::new()
        .with_size(
            NonZeroU32::new(128).unwrap(),
            NonZeroU32::new(128).unwrap(),
        )
        .with_projection(MapProjectionType::Mercator)
        .build_static_renderer();

    renderer
        .load_style_from_path(fixture_path("test-style.json"))
        .unwrap();
    let mercator = renderer.render_static(0.0, 0.0, 1.5, 0.0, 0.0).unwrap();
    assert_eq!(alpha_at_corners(mercator.as_image()), [255, 255, 255, 255]);

    renderer.set_projection(MapProjectionType::Globe);
    assert_eq!(renderer.projection(), MapProjectionType::Globe);
    let globe = renderer.render_static(0.0, 0.0, 1.5, 0.0, 0.0).unwrap();
    assert_eq!(alpha_at_corners(globe.as_image()), [0, 0, 0, 0]);
}
