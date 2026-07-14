//! raikiri-vrt — VRT harness (backend-agnostic anyrender rasterizer + tiny-skia PNG encoder).
//!
//! Thin wrapper providing the last-mile of the M1 render pipeline:
//!   `anyrender::PaintScene` write → RGBA8 buffer → PNG bytes.
//!
//! Backend: caller supplies any `anyrender::ImageRenderer` (M1 default =
//! `anyrender_vello_cpu`). Signature is generic over `R: ImageRenderer` so
//! future GPU backends can be swapped without API break.
//!
//! Downstream consumers:
//! - `raikiri` umbrella `html_to_png` (M1 end-to-end pipeline)
//! - hello-world VRT (m1.14)
//! - determinism test (m1.13)
//! - rayon thread-count test (m1.18)
