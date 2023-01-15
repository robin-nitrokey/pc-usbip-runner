use apdu_dispatch::{dispatch::ApduDispatch, Data};
use interchange::Channel;
use usb_device::bus::{UsbBus, UsbBusAllocator};
use usbd_ccid::Ccid;

pub fn setup<'a, B: UsbBus>(
    bus_allocator: &'a UsbBusAllocator<B>,
    contact: &'a Channel<Data, Data>,
    contactless: &'a Channel<Data, Data>,
) -> (Ccid<'a, B, 3072>, ApduDispatch<'a>) {
    let (ccid_rq, ccid_rp) = contact.split().unwrap();
    let ccid = Ccid::new(bus_allocator, ccid_rq, None);
    let apdu_dispatch = ApduDispatch::new(ccid_rp, contactless.split().unwrap().1);
    (ccid, apdu_dispatch)
}
