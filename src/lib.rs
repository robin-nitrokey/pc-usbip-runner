#[cfg(feature = "ccid")]
mod ccid;
#[cfg(feature = "ctaphid")]
mod ctaphid;

use std::{cell::RefCell, fmt::Debug, rc::Rc, thread, time::Duration};

use interchange::{Channel, Interchange};
use trussed::{
    api::{Reply, Request},
    error::Error,
    types::Backends,
    virt::{Platform, StoreProvider},
    ClientImplementation, Service,
};
use usb_device::{
    bus::{UsbBus, UsbBusAllocator},
    device::{UsbDevice, UsbDeviceBuilder, UsbVidPid},
};
use usbip_device::UsbIpBus;

pub type Client<'a, S, B, Bs, const CLIENT_COUNT: usize> =
    ClientImplementation<'a, B, Syscall<Service<'a, Platform<S, B>, Bs, CLIENT_COUNT>>>;

pub struct Options {
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
    pub vid: u16,
    pub pid: u16,
}

impl Options {
    fn vid_pid(&self) -> UsbVidPid {
        UsbVidPid(self.vid, self.pid)
    }
}

pub trait Apps<C: trussed::Client, D> {
    fn new(make_client: impl Fn(&str) -> C, data: D) -> Self;

    #[cfg(feature = "ctaphid")]
    fn with_ctaphid_apps<T>(
        &mut self,
        f: impl FnOnce(&mut [&mut dyn ctaphid_dispatch::app::App]) -> T,
    ) -> T;

    #[cfg(feature = "ccid")]
    fn with_ccid_apps<T>(
        &mut self,
        f: impl FnOnce(&mut [&mut dyn apdu_dispatch::app::App<7609, 7609>]) -> T,
    ) -> T;
}

pub struct Runner<
    S: StoreProvider,
    B: 'static + Debug + PartialEq,
    Bs: Backends<Platform<S, B>>,
    const CLIENT_COUNT: usize,
> {
    store: S,
    options: Options,
    init_platform: Option<Box<dyn Fn(&mut Platform<S, B>)>>,
    interchanges: Interchanges<B, CLIENT_COUNT>,
    backends: Bs,
}

impl<S: StoreProvider + Clone, const CLIENT_COUNT: usize> Runner<S, (), (), CLIENT_COUNT> {
    pub fn new(store: S, options: Options) -> Self {
        Self::with_backends(store, options, ())
    }
}

impl<S, B, Bs, const CLIENT_COUNT: usize> Runner<S, B, Bs, CLIENT_COUNT>
where
    S: StoreProvider + Clone,
    B: 'static + Debug + PartialEq,
    Bs: Backends<Platform<S, B>> + Clone,
{
    pub fn with_backends(store: S, options: Options, backends: Bs) -> Self {
        Self {
            store,
            options,
            init_platform: Default::default(),
            interchanges: Interchanges::new(),
            backends,
        }
    }

    pub fn init_platform<F>(&mut self, f: F) -> &mut Self
    where
        F: Fn(&mut Platform<S, B>) + 'static,
    {
        self.init_platform = Some(Box::new(f));
        self
    }

    pub fn exec<'a, A, D, F>(&'a mut self, make_data: F)
    where
        A: Apps<Client<'a, S, B, Bs, CLIENT_COUNT>, D>,
        F: Fn(&mut Platform<S, B>) -> D,
    {
        self.interchanges = Interchanges::new();

        let mut platform = Platform::new(self.store.clone());
        if let Some(init_platform) = &self.init_platform {
            init_platform(&mut platform);
        }
        let data = make_data(&mut platform);

        // To change IP or port see usbip-device-0.1.4/src/handler.rs:26
        let bus_allocator = UsbBusAllocator::new(UsbIpBus::new());

        #[cfg(feature = "ctaphid")]
        let (mut ctaphid, mut ctaphid_dispatch) =
            ctaphid::setup(&bus_allocator, &self.interchanges.ctaphid);

        #[cfg(feature = "ccid")]
        let (mut ccid, mut apdu_dispatch) = ccid::setup(
            &bus_allocator,
            &self.interchanges.ccid_contact,
            &self.interchanges.ccid_contactless,
        );

        let mut usb_device = build_device(&bus_allocator, &self.options);
        let service = Rc::new(RefCell::new(Service::with_backends(
            platform,
            &self.interchanges.trussed,
            self.backends.clone(),
        )));
        let syscall = Syscall::from(service.clone());
        let mut apps = A::new(
            |id| {
                service
                    .borrow_mut()
                    .try_new_client(id, syscall.clone())
                    .expect("failed to create client")
            },
            data,
        );

        log::info!("Ready for work");
        thread::scope(|s| {
            s.spawn(move || loop {
                thread::sleep(Duration::from_millis(5));
                usb_device.poll(&mut [
                    #[cfg(feature = "ctaphid")]
                    &mut ctaphid,
                    #[cfg(feature = "ccid")]
                    &mut ccid,
                ]);
            });
            loop {
                thread::sleep(Duration::from_millis(5));
                #[cfg(feature = "ctaphid")]
                apps.with_ctaphid_apps(|apps| ctaphid_dispatch.poll(apps));
                #[cfg(feature = "ccid")]
                apps.with_ccid_apps(|apps| apdu_dispatch.poll(apps));
            }
        });
    }
}

fn build_device<'a, B: UsbBus>(
    bus_allocator: &'a UsbBusAllocator<B>,
    options: &'a Options,
) -> UsbDevice<'a, B> {
    let mut usb_builder = UsbDeviceBuilder::new(bus_allocator, options.vid_pid());
    if let Some(manufacturer) = &options.manufacturer {
        usb_builder = usb_builder.manufacturer(manufacturer);
    }
    if let Some(product) = &options.product {
        usb_builder = usb_builder.product(product);
    }
    if let Some(serial_number) = &options.serial_number {
        usb_builder = usb_builder.serial_number(serial_number);
    }
    usb_builder.device_class(0x03).device_sub_class(0).build()
}

struct Interchanges<B: 'static, const CLIENT_COUNT: usize> {
    trussed: Interchange<Request<B>, Result<Reply, Error>, CLIENT_COUNT>,
    #[cfg(feature = "ccid")]
    ccid_contact: Channel<apdu_dispatch::Data, apdu_dispatch::Data>,
    #[cfg(feature = "ccid")]
    ccid_contactless: Channel<apdu_dispatch::Data, apdu_dispatch::Data>,
    #[cfg(feature = "ctaphid")]
    ctaphid: Channel<ctaphid_dispatch::types::Request, ctaphid_dispatch::types::Response>,
}

impl<B: 'static, const CLIENT_COUNT: usize> Interchanges<B, CLIENT_COUNT> {
    fn new() -> Self {
        Self {
            trussed: Interchange::new(),
            #[cfg(feature = "ccid")]
            ccid_contact: Channel::new(),
            #[cfg(feature = "ccid")]
            ccid_contactless: Channel::new(),
            #[cfg(feature = "ctaphid")]
            ctaphid: Channel::new(),
        }
    }
}

pub struct Syscall<T> {
    service: Rc<RefCell<T>>,
}

impl<'a, P: trussed::Platform, B: Backends<P>, const CLIENT_COUNT: usize> trussed::client::Syscall
    for Syscall<Service<'a, P, B, CLIENT_COUNT>>
{
    fn syscall(&mut self) {
        log::debug!("syscall");
        self.service.borrow_mut().process();
    }
}

impl<T> Clone for Syscall<T> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
        }
    }
}

impl<T> From<Rc<RefCell<T>>> for Syscall<T> {
    fn from(service: Rc<RefCell<T>>) -> Self {
        Self { service }
    }
}
