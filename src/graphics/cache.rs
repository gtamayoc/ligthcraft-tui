//! Memory-budgeted dual-tier LRU preview cache.
//! Eliminates memory leaks and UI stalls during rapid photo navigation.

use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;
use image::DynamicImage;
use lru::LruCache;
use ratatui::layout::Rect;
use ratatui_image::picker::ProtocolType;
use ratatui_image::protocol::Protocol;

/// Hashable protocol kind for cache key identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheProtocolKind {
    Halfblocks,
    Sixel,
    Kitty,
    Iterm2,
}

impl From<ProtocolType> for CacheProtocolKind {
    fn from(p: ProtocolType) -> Self {
        match p {
            ProtocolType::Halfblocks => Self::Halfblocks,
            ProtocolType::Sixel => Self::Sixel,
            ProtocolType::Kitty => Self::Kitty,
            ProtocolType::Iterm2 => Self::Iterm2,
        }
    }
}

/// Unique composite cache key for rendered terminal protocols.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PreviewCacheKey {
    pub photo_id: String,
    pub recipe_hash: u64,
    pub target_cells: (u16, u16),
    pub crop_region: Option<[u32; 4]>,
    pub protocol: CacheProtocolKind,
}

/// Pre-rendered cached protocol instance with area and tracked memory size.
#[derive(Clone)]
pub struct CachedProtocol {
    pub protocol: Arc<Protocol>,
    pub render_area: Rect,
    pub byte_size: usize,
}

impl fmt::Debug for CachedProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachedProtocol")
            .field("render_area", &self.render_area)
            .field("byte_size", &self.byte_size)
            .finish()
    }
}

/// Configuration settings for the preview LRU cache.
#[derive(Debug, Clone)]
pub struct PreviewCacheConfig {
    /// Maximum allowed memory footprint in bytes (default: 64 MB).
    pub max_memory_bytes: usize,
    /// Maximum number of distinct protocol frames to retain (default: 100).
    pub max_frames: usize,
}

impl Default for PreviewCacheConfig {
    fn default() -> Self {
        Self {
            max_memory_bytes: 64 * 1024 * 1024,
            max_frames: 100,
        }
    }
}

/// Dual-tier LRU cache tracking both pre-rendered terminal protocols (Tier 1)
/// and downscaled master images (Tier 2).
pub struct PreviewCache {
    config: PreviewCacheConfig,
    protocol_lru: LruCache<PreviewCacheKey, CachedProtocol>,
    image_lru: LruCache<String, (Arc<DynamicImage>, usize)>,
    current_bytes: usize,
}

impl PreviewCache {
    pub fn new(config: PreviewCacheConfig) -> Self {
        let cap = NonZeroUsize::new(config.max_frames).unwrap_or(NonZeroUsize::new(50).unwrap());
        Self {
            config,
            protocol_lru: LruCache::new(cap),
            image_lru: LruCache::new(NonZeroUsize::new(20).unwrap()),
            current_bytes: 0,
        }
    }

    /// Retrieve a pre-rendered protocol from Tier 1 cache.
    pub fn get_protocol(&mut self, key: &PreviewCacheKey) -> Option<CachedProtocol> {
        self.protocol_lru.get(key).cloned()
    }

    /// Store a pre-rendered protocol into Tier 1 cache, updating memory tracking and evicting if necessary.
    pub fn insert_protocol(
        &mut self,
        key: PreviewCacheKey,
        protocol: Protocol,
        render_area: Rect,
    ) {
        let byte_size = Self::estimate_protocol_bytes(&protocol, render_area);
        let cached = CachedProtocol {
            protocol: Arc::new(protocol),
            render_area,
            byte_size,
        };

        if let Some(old) = self.protocol_lru.put(key, cached) {
            self.current_bytes = self.current_bytes.saturating_sub(old.byte_size);
        }
        self.current_bytes += byte_size;
        self.evict_if_needed();
    }

    /// Retrieve a decoded master image from Tier 2 cache.
    pub fn get_source_image(&mut self, photo_id: &str, recipe_hash: u64) -> Option<Arc<DynamicImage>> {
        let key = format!("{}:{:016x}", photo_id, recipe_hash);
        self.image_lru.get(&key).map(|(img, _)| Arc::clone(img))
    }

    /// Store a decoded master image into Tier 2 cache, returning an Arc reference.
    pub fn insert_source_image(
        &mut self,
        photo_id: &str,
        recipe_hash: u64,
        img: DynamicImage,
    ) -> Arc<DynamicImage> {
        let key = format!("{}:{:016x}", photo_id, recipe_hash);
        let byte_size = (img.width() as usize) * (img.height() as usize) * 4;
        let arc_img = Arc::new(img);

        if let Some((_, old_size)) = self.image_lru.put(key, (Arc::clone(&arc_img), byte_size)) {
            self.current_bytes = self.current_bytes.saturating_sub(old_size);
        }
        self.current_bytes += byte_size;
        self.evict_if_needed();

        arc_img
    }

    /// Invalidate all cache entries associated with a specific photo ID.
    pub fn evict_photo(&mut self, photo_id: &str) {
        let matching_proto_keys: Vec<PreviewCacheKey> = self
            .protocol_lru
            .iter()
            .filter(|(k, _)| k.photo_id == photo_id)
            .map(|(k, _)| k.clone())
            .collect();

        for key in matching_proto_keys {
            if let Some(popped) = self.protocol_lru.pop(&key) {
                self.current_bytes = self.current_bytes.saturating_sub(popped.byte_size);
            }
        }

        let prefix = format!("{}:", photo_id);
        let matching_img_keys: Vec<String> = self
            .image_lru
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(k, _)| k.clone())
            .collect();

        for key in matching_img_keys {
            if let Some((_, popped_size)) = self.image_lru.pop(&key) {
                self.current_bytes = self.current_bytes.saturating_sub(popped_size);
            }
        }
    }

    /// Evict oldest entries until memory usage is under budget and count is within bounds.
    fn evict_if_needed(&mut self) {
        while self.current_bytes > self.config.max_memory_bytes {
            // First evict from protocol LRU
            if let Some((_, popped)) = self.protocol_lru.pop_lru() {
                self.current_bytes = self.current_bytes.saturating_sub(popped.byte_size);
                continue;
            }
            // Next evict from image LRU
            if let Some((_, (_, popped_size))) = self.image_lru.pop_lru() {
                self.current_bytes = self.current_bytes.saturating_sub(popped_size);
                continue;
            }
            break;
        }
    }

    /// Estimate memory size in bytes occupied by a Protocol instance for given render area.
    pub fn estimate_protocol_bytes(proto: &Protocol, area: Rect) -> usize {
        let cells = area.width as usize * area.height as usize;
        match proto {
            Protocol::Halfblocks(_) => std::mem::size_of::<Protocol>() + (cells * 48),
            Protocol::Sixel(s) => std::mem::size_of::<Protocol>() + s.data.len(),
            Protocol::Kitty(_) => std::mem::size_of::<Protocol>() + (cells * 8).max(64 * 1024),
            Protocol::ITerm2(it) => std::mem::size_of::<Protocol>() + it.data.len(),
        }
    }

    /// Current total allocated memory in bytes across both cache tiers.
    pub fn current_memory_bytes(&self) -> usize {
        self.current_bytes
    }

    /// Number of cached protocol frames in Tier 1.
    pub fn protocol_count(&self) -> usize {
        self.protocol_lru.len()
    }

    /// Number of cached source images in Tier 2.
    pub fn image_count(&self) -> usize {
        self.image_lru.len()
    }

    /// Clear all cached protocols and images, resetting memory tracker to zero.
    pub fn clear(&mut self) {
        self.protocol_lru.clear();
        self.image_lru.clear();
        self.current_bytes = 0;
    }
}
