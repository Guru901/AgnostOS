use uefi::{
    Status,
    boot::{self, OpenProtocolAttributes, OpenProtocolParams, ScopedProtocol},
    proto::console::gop::GraphicsOutput,
};

/// Initializes the UEFI Graphics Output Protocol using the largest available
/// display mode up to 1920×1080.
///
pub fn init_gop() -> uefi::Result<ScopedProtocol<GraphicsOutput>> {
    let gop_handle = boot::get_handle_for_protocol::<GraphicsOutput>()?;

    // SAFETY: `gop_handle` was obtained for `GraphicsOutput`, and this image is
    // the active UEFI agent. `GetProtocol` only borrows the protocol while the
    // returned `ScopedProtocol` keeps that borrow alive.
    let mut gop = unsafe {
        boot::open_protocol::<GraphicsOutput>(
            OpenProtocolParams {
                handle: gop_handle,
                agent: boot::image_handle(),
                controller: None,
            },
            OpenProtocolAttributes::GetProtocol,
        )?
    };

    let mode = gop
        .modes()
        .filter(|mode| {
            let (w, h) = mode.info().resolution();
            w <= 1920 && h <= 1080
        })
        .max_by_key(|mode| {
            let (w, h) = mode.info().resolution();
            w * h
        })
        .ok_or(Status::UNSUPPORTED)?;

    gop.set_mode(&mode)?;
    Ok(gop)
}
