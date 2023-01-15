use ctaphid_dispatch::{
    dispatch::Dispatch,
    types::{Request, Response},
};
use interchange::Channel;
use usb_device::bus::{UsbBus, UsbBusAllocator};
use usbd_ctaphid::CtapHid;

pub fn setup<'a, B: UsbBus>(
    bus_allocator: &'a UsbBusAllocator<B>,
    channel: &'a Channel<Request, Response>,
) -> (CtapHid<'a, B>, Dispatch<'a>) {
    let (ctaphid_rq, ctaphid_rp) = channel.split().unwrap();
    let ctaphid = CtapHid::new(bus_allocator, ctaphid_rq, 0u32)
        .implements_ctap1()
        .implements_ctap2()
        .implements_wink();
    let ctaphid_dispatch = Dispatch::new(ctaphid_rp);
    (ctaphid, ctaphid_dispatch)
}
