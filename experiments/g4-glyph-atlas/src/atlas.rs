//! Two atlas textures and a fixed-slot LRU cache over them.
//!
//! **Why fixed slots rather than a shelf packer.** A log grid is monospace, so every glyph fits a
//! cell box (wide glyphs take two horizontally adjacent slots). That makes eviction a slot swap
//! with no repacking and no rasterisation of innocent bystanders — which is what G4's "eviction
//! does not stall a frame" criterion actually needs. A shelf packer has to reset the whole sheet
//! when it fills, re-rasterising every glyph still on screen; that variant is measured in
//! `grid.rs` for comparison.

use std::collections::HashMap;

use windows::core::Result;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_SAMPLE_DESC};

pub struct AtlasTexture {
    pub texture: ID3D11Texture2D,
    pub srv: ID3D11ShaderResourceView,
    pub bytes_per_pixel: u32,
}

impl AtlasTexture {
    pub fn new(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        format: DXGI_FORMAT,
        bytes_per_pixel: u32,
    ) -> Result<Self> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
        let texture = texture.expect("out param set on success");
        let mut srv = None;
        unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut srv))? };
        Ok(Self {
            texture,
            srv: srv.expect("out param set on success"),
            bytes_per_pixel,
        })
    }

    /// Upload one glyph's pixels into a sub-rect. `data` is tightly packed at
    /// `w * bytes_per_pixel` per row.
    pub fn upload(&self, ctx: &ID3D11DeviceContext, x: u32, y: u32, w: u32, h: u32, data: &[u8]) {
        if w == 0 || h == 0 {
            return;
        }
        let row_pitch = w * self.bytes_per_pixel;
        debug_assert_eq!(data.len() as u32, row_pitch * h);
        let box_ = D3D11_BOX {
            left: x,
            top: y,
            front: 0,
            right: x + w,
            bottom: y + h,
            back: 1,
        };
        unsafe {
            ctx.UpdateSubresource(
                &self.texture,
                0,
                Some(&box_),
                data.as_ptr() as *const _,
                row_pitch,
                0,
            );
        }
    }
}

/// What a cached glyph needs for drawing, in atlas texel space.
#[derive(Copy, Clone, Debug)]
pub struct GlyphEntry {
    pub atlas_x: u32,
    pub atlas_y: u32,
    pub width: u32,
    pub height: u32,
    /// Offset from the pen origin to the top-left of the rasterised box.
    pub left: i32,
    pub top: i32,
    pub colour: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct GlyphKey {
    pub face: u16,
    pub glyph: u16,
    /// Font size in 1/4 px, so the key stays hashable.
    pub size_q: u16,
}

/// How a full atlas picks its victim. G4 measures both, because the choice turns out to decide
/// whether eviction stalls the frame.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Policy {
    /// Scan every slot for the oldest. O(capacity) per miss — the obvious implementation.
    ScanLru,
    /// Intrusive doubly-linked LRU list. O(1) per miss and per touch.
    ListLru,
}

/// Fixed-slot atlas. Every slot is `slot_w * slot_h` and holds exactly one glyph, so eviction never
/// repacks and never has to find adjacent runs. `slot_w` must therefore cover the widest glyph
/// accepted; wider glyphs are refused and counted rather than clipped.
pub struct SlotAtlas {
    pub cols: u32,
    pub rows: u32,
    pub policy: Policy,
    occupant: Vec<Option<GlyphKey>>,
    used: Vec<u64>,
    /// Cursor for the cold-start free sweep, so filling an empty atlas stays O(1) amortised.
    next_free: u32,
    filled: u32,

    // Intrusive LRU list over slot indices. `u32::MAX` is the null link.
    prev: Vec<u32>,
    next: Vec<u32>,
    head: u32,
    tail: u32,
}

const NIL: u32 = u32::MAX;

pub struct Allocation {
    pub col: u32,
    pub row: u32,
    pub slot: u32,
    /// The key displaced to make room, if any. The caller must drop it from its map.
    pub evicted: Option<GlyphKey>,
}

impl SlotAtlas {
    pub fn new(width: u32, height: u32, slot_w: u32, slot_h: u32, policy: Policy) -> Self {
        let cols = width / slot_w;
        let rows = height / slot_h;
        let n = (cols * rows) as usize;
        Self {
            cols,
            rows,
            policy,
            occupant: vec![None; n],
            used: vec![0; n],
            next_free: 0,
            filled: 0,
            prev: vec![NIL; n],
            next: vec![NIL; n],
            head: NIL,
            tail: NIL,
        }
    }

    pub fn capacity(&self) -> u32 {
        self.cols * self.rows
    }

    pub fn occupied(&self) -> u32 {
        self.filled
    }

    fn unlink(&mut self, s: u32) {
        let (p, n) = (self.prev[s as usize], self.next[s as usize]);
        if p != NIL {
            self.next[p as usize] = n;
        } else if self.head == s {
            self.head = n;
        }
        if n != NIL {
            self.prev[n as usize] = p;
        } else if self.tail == s {
            self.tail = p;
        }
        self.prev[s as usize] = NIL;
        self.next[s as usize] = NIL;
    }

    /// Move to the most-recently-used end (the tail).
    fn push_back(&mut self, s: u32) {
        self.prev[s as usize] = self.tail;
        self.next[s as usize] = NIL;
        if self.tail != NIL {
            self.next[self.tail as usize] = s;
        } else {
            self.head = s;
        }
        self.tail = s;
    }

    /// Allocate one slot, evicting the least-recently-used if the sheet is full.
    ///
    /// Slots touched during `current_frame` are never evicted — a glyph already drawn this frame
    /// must survive, or the frame would corrupt itself.
    pub fn alloc(&mut self, current_frame: u64) -> Option<Allocation> {
        let total = self.capacity();
        if total == 0 {
            return None;
        }

        if self.filled < total {
            // Cold path: take the next unoccupied slot.
            for step in 0..total {
                let s = (self.next_free + step) % total;
                if self.occupant[s as usize].is_none() {
                    self.next_free = (s + 1) % total;
                    return Some(Allocation {
                        col: s % self.cols,
                        row: s / self.cols,
                        slot: s,
                        evicted: None,
                    });
                }
            }
        }

        let victim = match self.policy {
            Policy::ListLru => {
                let s = self.head;
                if s == NIL || self.used[s as usize] >= current_frame {
                    return None;
                }
                s
            }
            Policy::ScanLru => {
                let mut best: Option<(u64, u32)> = None;
                for s in 0..total {
                    let u = self.used[s as usize];
                    if u >= current_frame {
                        continue;
                    }
                    if best.map_or(true, |(b, _)| u < b) {
                        best = Some((u, s));
                    }
                }
                best?.1
            }
        };

        let evicted = self.occupant[victim as usize].take();
        if evicted.is_some() {
            self.filled -= 1;
        }
        self.unlink(victim);
        Some(Allocation {
            col: victim % self.cols,
            row: victim / self.cols,
            slot: victim,
            evicted,
        })
    }

    pub fn place(&mut self, slot: u32, key: GlyphKey, frame: u64) {
        if self.occupant[slot as usize].is_none() {
            self.filled += 1;
        }
        self.occupant[slot as usize] = Some(key);
        self.used[slot as usize] = frame;
        self.unlink(slot);
        self.push_back(slot);
    }

    pub fn touch(&mut self, slot: u32, frame: u64) {
        self.used[slot as usize] = frame;
        if self.policy == Policy::ListLru {
            self.unlink(slot);
            self.push_back(slot);
        }
    }

    pub fn slot_of(&self, col: u32, row: u32) -> u32 {
        row * self.cols + col
    }
}

/// Counters G4 reports on.
#[derive(Default, Debug, Clone, Copy)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    /// DirectWrite rasterisation. Unavoidable on a miss, and nothing to do with eviction policy.
    pub raster_ns: u64,
    pub upload_ns: u64,
    /// Slot bookkeeping only — the part that eviction policy is actually responsible for.
    pub alloc_ns: u64,
    pub failed: u64,
    /// Glyphs with no ink (space, and CJK codepoints absent from the face). Cached as blanks so
    /// they are not re-rasterised every frame.
    pub blanks: u64,
}

pub struct GlyphCache {
    pub map: HashMap<GlyphKey, GlyphEntry>,
    pub slots: SlotAtlas,
    pub stats: CacheStats,
}

impl GlyphCache {
    pub fn new(width: u32, height: u32, slot_w: u32, slot_h: u32, policy: Policy) -> Self {
        Self {
            map: HashMap::new(),
            slots: SlotAtlas::new(width, height, slot_w, slot_h, policy),
            stats: CacheStats::default(),
        }
    }

    /// Drop every cached glyph. Used to give each measured eviction policy a cold start.
    pub fn reset(&mut self, width: u32, height: u32, slot_w: u32, slot_h: u32, policy: Policy) {
        self.map.clear();
        self.slots = SlotAtlas::new(width, height, slot_w, slot_h, policy);
    }
}
