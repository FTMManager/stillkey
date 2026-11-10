//! Step 6: the graphics mode firmware selected. Nothing is drawn.

use stillkey_bootinfo::{FB_FORMAT_BGRX, FB_FORMAT_RGBX, FRAMEBUFFER_VIRT, Framebuffer};
use uefi::boot::{self, OpenProtocolAttributes, OpenProtocolParams};
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};

/// The current mode, if it has a linear 32-bit framebuffer that makes sense.
pub fn capture() -> Option<Framebuffer> {
    let handle = boot::get_handle_for_protocol::<GraphicsOutput>().ok()?;
    // SAFETY: `GetProtocol` leaves the protocol with the firmware driver that
    // owns it. The loader only reads the current mode and the framebuffer
    // address, changes nothing, and closes the protocol when this returns.
    let mut gop = unsafe {
        boot::open_protocol::<GraphicsOutput>(
            OpenProtocolParams { handle, agent: boot::image_handle(), controller: None },
            OpenProtocolAttributes::GetProtocol,
        )
    }
    .ok()?;
    let gop = gop.get_mut()?;

    let mode = gop.current_mode_info();
    let format = match mode.pixel_format() {
        PixelFormat::Rgb => FB_FORMAT_RGBX,
        PixelFormat::Bgr => FB_FORMAT_BGRX,
        // A bit-mask format or no linear framebuffer at all: serial only.
        _ => return None,
    };
    let (width, height) = mode.resolution();
    let stride = mode.stride();
    let mut buffer = gop.frame_buffer();
    let record = Framebuffer {
        phys_base: buffer.as_mut_ptr() as u64,
        size: buffer.size() as u64,
        virt_base: FRAMEBUFFER_VIRT,
        width: u32::try_from(width).ok()?,
        height: u32::try_from(height).ok()?,
        stride_pixels: u32::try_from(stride).ok()?,
        format,
    };
    // The rule the kernel applies before it writes a pixel (docs/02 section
    // 6). A mode that fails it is not handed over: serial only.
    record.is_well_formed().then_some(record)
}
