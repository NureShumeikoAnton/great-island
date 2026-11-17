//! Windows.Graphics.Capture, the part a screenshot and a recording share: a monitor as a capture item,
//! a frame pool with a session on a Direct3D device, and a frame as a texture.

use windows::{
    Foundation::TypedEventHandler,
    Graphics::{
        Capture::{
            Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureAccess,
            GraphicsCaptureAccessKind, GraphicsCaptureItem, GraphicsCaptureSession,
        },
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
        SizeInt32,
    },
    Win32::{
        Graphics::{
            Direct3D11::{ID3D11Device, ID3D11Texture2D},
            Dxgi::IDXGIDevice,
            Gdi::HMONITOR,
        },
        System::WinRT::{
            Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
            Graphics::Capture::IGraphicsCaptureItemInterop,
        },
    },
    core::{IInspectable, Interface, Result},
};

pub fn item(monitor: isize) -> Result<GraphicsCaptureItem> {
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    unsafe { interop.CreateForMonitor(HMONITOR(monitor as *mut _)) }
}

/// The callbacks run on a thread of Windows' own: they only pass the news on.
pub fn open(
    device: &ID3D11Device,
    item: &GraphicsCaptureItem,
    size: SizeInt32,
    on_frame: impl Fn() + Send + 'static,
    on_closed: impl Fn() + Send + 'static,
) -> Result<(Direct3D11CaptureFramePool, GraphicsCaptureSession)> {
    let winrt: IDirect3DDevice =
        unsafe { CreateDirect3D11DeviceFromDXGIDevice(&device.cast::<IDXGIDevice>()?)? }.cast()?;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &winrt,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        2,
        size,
    )?;
    pool.FrameArrived(&TypedEventHandler::new(move |_, _| {
        on_frame();
        Ok(())
    }))?;
    item.Closed(
        &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
            on_closed();
            Ok(())
        }),
    )?;
    let session = pool.CreateCaptureSession(item)?;
    Ok((pool, session))
}

/// Removes the yellow frame Windows draws around the monitor; Windows 10 has no such switch.
pub fn borderless(session: &GraphicsCaptureSession) {
    if GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless)
        .and_then(|op| op.join())
        .is_ok()
    {
        let _ = session.SetIsBorderRequired(false);
    }
}

pub fn texture(frame: &Direct3D11CaptureFrame) -> Result<ID3D11Texture2D> {
    unsafe {
        frame
            .Surface()?
            .cast::<IDirect3DDxgiInterfaceAccess>()?
            .GetInterface()
    }
}
