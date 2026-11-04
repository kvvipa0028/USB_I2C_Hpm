//! Small transport seam: keep queue ownership/cancellation in the session code.

use std::time::Duration;

use nusb::transfer::{Buffer, Bulk, Completion, In, Out};

use crate::proto;

const EP_OUT: u8 = 0x01;
const EP_IN: u8 = 0x81;

pub(super) trait Transport {
    fn submit_in(&mut self, reply_max: usize);
    /// Return the padded length that must be reported by OUT completion.
    fn submit_out(&mut self, request: Vec<u8>) -> usize;
    fn wait_in(&mut self, timeout: Duration) -> Option<Completion>;
    fn wait_out(&mut self, timeout: Duration) -> Option<Completion>;
    fn pending_in(&self) -> usize;
    fn pending_out(&self) -> usize;
    fn cancel_all(&mut self);
}

pub(super) struct UsbTransport {
    _iface: nusb::Interface,
    out: nusb::Endpoint<Bulk, Out>,
    inp: nusb::Endpoint<Bulk, In>,
}

impl UsbTransport {
    pub(super) fn new(iface: nusb::Interface) -> anyhow::Result<Self> {
        let out = iface.endpoint::<Bulk, Out>(EP_OUT)?;
        let inp = iface.endpoint::<Bulk, In>(EP_IN)?;
        Ok(Self {
            _iface: iface,
            out,
            inp,
        })
    }
}

impl Transport for UsbTransport {
    fn submit_in(&mut self, reply_max: usize) {
        self.inp.submit(
            self.inp
                .allocate(proto::in_len(reply_max, self.inp.max_packet_size())),
        );
    }

    fn submit_out(&mut self, request: Vec<u8>) -> usize {
        let request = proto::pad_for_mps(request, self.out.max_packet_size());
        let len = request.len();
        self.out.submit(Buffer::from(request));
        len
    }

    fn wait_in(&mut self, timeout: Duration) -> Option<Completion> {
        self.inp.wait_next_complete(timeout)
    }

    fn wait_out(&mut self, timeout: Duration) -> Option<Completion> {
        self.out.wait_next_complete(timeout)
    }

    fn pending_in(&self) -> usize {
        self.inp.pending()
    }

    fn pending_out(&self) -> usize {
        self.out.pending()
    }

    fn cancel_all(&mut self) {
        self.out.cancel_all();
        self.inp.cancel_all();
    }
}
