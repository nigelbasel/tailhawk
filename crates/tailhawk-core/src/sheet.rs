//! The atlas sheet on the GPU — one texture, one slot per glyph.
//!
//! [`crate::atlas`] decides which slot a glyph gets and [`crate::raster`] produces its pixels; this
//! is where those pixels land. The two are deliberately separate: the allocator is portable
//! arithmetic with no device in it, and this is a texture with no policy in it.

use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};

use crate::Result;

/// Bytes per texel in every format a sheet uses. Both the mono and the colour sheet are 8-bit
/// four-channel; only the channel *meaning* differs, so one constant covers the row pitch.
const BYTES_PER_TEXEL: u32 = 4;

pub struct Sheet {
    texture: ID3D11Texture2D,
    srv: ID3D11ShaderResourceView,
    width: u16,
    height: u16,
}

impl Sheet {
    /// The monochrome sheet: RGB are the three ClearType coverages and A is their average, which is
    /// what lets one sheet serve both subpixel and greyscale rendering at no extra memory (G4).
    pub fn mono(device: &ID3D11Device, width: u16, height: u16) -> Result<Self> {
        Self::new(device, width, height, DXGI_FORMAT_R8G8B8A8_UNORM)
    }

    fn new(device: &ID3D11Device, width: u16, height: u16, format: DXGI_FORMAT) -> Result<Self> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width as u32,
            Height: height as u32,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            // DEFAULT with UpdateSubresource rather than DYNAMIC with Map: a dynamic texture must
            // be mapped WRITE_DISCARD, which throws away every glyph already resident. The whole
            // point of the atlas is that they stay.
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
        let texture = texture.expect("texture out param is set on success");
        let mut srv = None;
        unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut srv))? };
        Ok(Self {
            texture,
            srv: srv.expect("srv out param is set on success"),
            width,
            height,
        })
    }

    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    pub fn srv(&self) -> &ID3D11ShaderResourceView {
        &self.srv
    }

    /// Writes one glyph's pixels into its slot. `rgba` must be tightly packed at `w * 4` bytes per
    /// row, which is what [`crate::raster::Rasteriser::rasterise`] produces.
    ///
    /// Returns without drawing if the rectangle would leave the sheet. A partial write would
    /// silently corrupt whichever glyph occupies the neighbouring texels, and an out-of-range
    /// `D3D11_BOX` is undefined rather than an error.
    pub fn upload(
        &self,
        context: &ID3D11DeviceContext,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        rgba: &[u8],
    ) -> bool {
        let row_pitch = w as u32 * BYTES_PER_TEXEL;
        if w == 0
            || h == 0
            || x as u32 + w as u32 > self.width as u32
            || y as u32 + h as u32 > self.height as u32
            || rgba.len() as u32 != row_pitch * h as u32
        {
            return false;
        }
        let box_ = D3D11_BOX {
            left: x as u32,
            top: y as u32,
            front: 0,
            right: (x + w) as u32,
            bottom: (y + h) as u32,
            back: 1,
        };
        unsafe {
            context.UpdateSubresource(
                &self.texture,
                0,
                Some(&box_),
                rgba.as_ptr().cast(),
                row_pitch,
                0,
            );
        }
        true
    }
}
