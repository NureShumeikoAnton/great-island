//! Direct3D for Media Foundation: the device the recording and the player hand to it, and BGRA
//! textures read back into memory. Adapter layer, shared by `record` and `play`.

use windows::{
    Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            Direct3D11::{
                D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG,
                D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
                D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE, D3D11_USAGE_DEFAULT,
                D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11Multithread,
                ID3D11Texture2D,
            },
            Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
        },
        Media::MediaFoundation::{IMFDXGIDeviceManager, MFCreateDXGIDeviceManager},
    },
    core::{Error, Interface, Result},
};

pub fn video_device() -> Result<ID3D11Device> {
    // Video support lets Media Foundation convert and decode frames on the video card;
    // some adapters can't, and the recording then reads frames back.
    let flags = [
        D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    ];
    let mut last = Error::empty();
    for flag in flags {
        match create_device(flag) {
            Ok(device) => {
                // Media Foundation uses the device from its own threads.
                if let Ok(multithread) = device.cast::<ID3D11Multithread>() {
                    unsafe {
                        let _ = multithread.SetMultithreadProtected(true);
                    }
                }
                return Ok(device);
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

pub fn manager(device: &ID3D11Device) -> Result<IMFDXGIDeviceManager> {
    let mut token = 0;
    let mut manager = None;
    unsafe {
        MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
        let manager = manager.ok_or_else(Error::empty)?;
        manager.ResetDevice(device, token)?;
        Ok(manager)
    }
}

/// One video device for every engine and reader of a window. A device of their own each costs
/// the driver memory: three engines took ~65 MB more on Intel Iris Xe (measured).
#[derive(Clone)]
pub struct Card {
    pub device: ID3D11Device,
    pub manager: IMFDXGIDeviceManager,
}

// SAFETY: both are free-threaded. The device is multithread protected (`video_device`), and the
// manager exists to share it between components on different threads: Media Foundation's own
// pipeline takes it from its worker threads and serializes the device through `LockDevice`.
unsafe impl Send for Card {}

impl Card {
    pub fn new() -> Result<Self> {
        let device = video_device()?;
        let manager = manager(&device)?;
        Ok(Self { device, manager })
    }
}

fn create_device(flags: D3D11_CREATE_DEVICE_FLAG) -> Result<ID3D11Device> {
    let mut device = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            flags,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
    }
    device.ok_or_else(Error::empty)
}

/// A BGRA texture to draw into on the video card.
pub fn texture(device: &ID3D11Device, size: (u32, u32)) -> Result<ID3D11Texture2D> {
    create_texture(
        device,
        size,
        D3D11_USAGE_DEFAULT,
        (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        0,
    )
}

/// A BGRA texture the processor can read.
pub fn staging(device: &ID3D11Device, size: (u32, u32)) -> Result<ID3D11Texture2D> {
    create_texture(
        device,
        size,
        D3D11_USAGE_STAGING,
        0,
        D3D11_CPU_ACCESS_READ.0 as u32,
    )
}

fn create_texture(
    device: &ID3D11Device,
    (width, height): (u32, u32),
    usage: D3D11_USAGE,
    bind: u32,
    cpu: u32,
) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: usage,
        BindFlags: bind,
        CPUAccessFlags: cpu,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    texture.ok_or_else(Error::empty)
}

/// The texture's pixels, rows top to bottom, alpha forced to opaque.
pub fn read_back(
    device: &ID3D11Device,
    texture: &ID3D11Texture2D,
    (width, height): (u32, u32),
) -> Result<Vec<u8>> {
    let staging = staging(device, (width, height))?;
    let row = width as usize * 4;
    let mut pixels = vec![0u8; row * height as usize];
    unsafe {
        let context = device.GetImmediateContext()?;
        context.CopyResource(&staging, texture);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        for y in 0..height as usize {
            std::ptr::copy_nonoverlapping(
                (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                pixels.as_mut_ptr().add(y * row),
                row,
            );
        }
        context.Unmap(&staging, 0);
    }
    for alpha in pixels.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }
    Ok(pixels)
}
