use uefi::{
    boot::{self, OpenProtocolAttributes, OpenProtocolParams, ScopedProtocol},
    proto::console::gop::GraphicsOutput,
};

#[must_use]
pub fn init_gop() -> ScopedProtocol<GraphicsOutput> {
    let gop_handle = boot::get_handle_for_protocol::<GraphicsOutput>()
        .expect("missing graphics output protocol");

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
        )
        .expect("failed to open Graphics Output Protocol")
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
        .expect("no graphics modes available");

    gop.set_mode(&mode).expect("failed to set GOP mode");
    gop // return owned, not a reference
}
