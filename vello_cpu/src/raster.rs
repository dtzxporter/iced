use crate::core::image as raster;
use crate::core::{Image, Rectangle, Size};
use crate::graphics;

use rustc_hash::{FxHashMap, FxHashSet};

use std::cell::RefCell;
use std::collections::hash_map;
use std::sync::Arc;

#[derive(Debug)]
pub struct Pipeline {
    cache: RefCell<Cache>,
}

impl Pipeline {
    pub fn new() -> Self {
        Self {
            cache: RefCell::new(Cache::default()),
        }
    }

    pub fn load(&self, handle: &raster::Handle) -> Result<raster::Allocation, raster::Error> {
        let mut cache = self.cache.borrow_mut();
        let image = cache.allocate(handle)?;

        #[allow(unsafe_code)]
        Ok(unsafe {
            raster::allocate(
                handle,
                Size::new(u32::from(image.width()), u32::from(image.height())),
            )
        })
    }

    pub fn dimensions(&self, handle: &raster::Handle) -> Option<Size<u32>> {
        let mut cache = self.cache.borrow_mut();
        let image = cache.allocate(handle).ok()?;

        Some(Size::new(
            u32::from(image.width()),
            u32::from(image.height()),
        ))
    }

    pub fn draw(
        &mut self,
        image: &Image,
        bounds: Rectangle,
        renderer: &mut vello_cpu::RenderContext,
        scale_factor: f32,
    ) {
        let mut cache = self.cache.borrow_mut();

        let Ok(pixmap) = cache.allocate(&image.handle) else {
            return;
        };

        let width = f32::from(pixmap.width());
        let height = f32::from(pixmap.height());
        let width_scale = bounds.width / width;
        let height_scale = bounds.height / height;

        let transform = vello_cpu::kurbo::Affine::translate(vello_cpu::kurbo::Vec2::new(
            -f64::from(width) / 2.0,
            -f64::from(height) / 2.0,
        ))
        .then_rotate(f64::from(image.rotation.0))
        .then_translate(vello_cpu::kurbo::Vec2::new(
            f64::from(width) / 2.0,
            f64::from(height) / 2.0,
        ))
        .then_scale_non_uniform(f64::from(width_scale), f64::from(height_scale))
        .then_translate(vello_cpu::kurbo::Vec2::new(
            f64::from(bounds.x),
            f64::from(bounds.y),
        ))
        .then_scale(f64::from(scale_factor));

        let quality = match image.filter_method {
            raster::FilterMethod::Linear => vello_cpu::peniko::ImageQuality::Medium,
            raster::FilterMethod::Nearest => vello_cpu::peniko::ImageQuality::Low,
        };

        renderer.set_paint(vello_cpu::peniko::Brush::Image(
            vello_cpu::peniko::ImageBrush {
                image: vello_cpu::ImageSource::Pixmap(pixmap),
                sampler: vello_cpu::peniko::ImageSampler::new()
                    .with_quality(quality)
                    .with_alpha(image.opacity),
            },
        ));

        renderer.set_transform(transform);

        renderer.fill_rect(&crate::into_rect(Rectangle {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }));

        renderer.reset_transform();
    }

    pub fn trim_cache(&mut self) {
        self.cache.borrow_mut().trim();
    }
}

#[derive(Debug, Default)]
struct Cache {
    entries: FxHashMap<raster::Id, Option<Entry>>,
    hits: FxHashSet<raster::Id>,
}

impl Cache {
    pub fn allocate(
        &mut self,
        handle: &raster::Handle,
    ) -> Result<Arc<vello_cpu::Pixmap>, raster::Error> {
        let id = handle.id();

        if let hash_map::Entry::Vacant(entry) = self.entries.entry(id) {
            let image = match graphics::image::load(handle) {
                Ok(image) => image,
                Err(error) => {
                    let _ = entry.insert(None);

                    return Err(error);
                }
            };

            let width = image.width() as u16;
            let height = image.height() as u16;

            if width == 0 || height == 0 {
                return Err(raster::Error::Empty);
            }

            let mut buffer = vello_cpu::Pixmap::new(width, height);

            let src = image.as_raw();
            let dst = buffer.data_as_u8_slice_mut();

            for (src, dst) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                let src: [u8; 4] = src.try_into().unwrap();
                let dst: &mut [u8; 4] = dst.try_into().unwrap();

                let [b, g, r, a] = src;

                dst[0] = ((r as u16 * a as u16 + 127) / 255) as u8;
                dst[1] = ((g as u16 * a as u16 + 127) / 255) as u8;
                dst[2] = ((b as u16 * a as u16 + 127) / 255) as u8;
                dst[3] = a;
            }

            let _ = entry.insert(Some(Entry {
                width: image.width(),
                height: image.height(),
                pixels: Arc::new(buffer),
            }));
        }

        let _ = self.hits.insert(id);

        Ok(self
            .entries
            .get(&id)
            .unwrap()
            .as_ref()
            .map(|entry| entry.pixels.clone())
            .expect("Image should be allocated"))
    }

    fn trim(&mut self) {
        self.entries.retain(|key, _| self.hits.contains(key));
        self.hits.clear();
    }
}

#[derive(Debug)]
struct Entry {
    width: u32,
    height: u32,
    pixels: Arc<vello_cpu::Pixmap>,
}
