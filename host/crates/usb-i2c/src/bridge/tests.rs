//! Software queue model, not a hardware/USB-driver test. It deliberately keeps
//! a timeout's transfer pending and preserves device replies across reconnects.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use nusb::transfer::{Buffer, Completion, TransferError};

use super::*;
use crate::proto::MAX_XFER;

#[derive(Debug)]
enum Outcome {
    Complete,
    Timeout,
    Error(TransferError),
    Short(usize),
}

#[derive(Default)]
struct State {
    requests: Vec<Vec<u8>>,
    replies: VecDeque<Vec<u8>>,
    in_outcomes: VecDeque<Outcome>,
    out_outcomes: VecDeque<Outcome>,
    in_sizes: Vec<usize>,
    allocated_in_sizes: Vec<usize>,
    packet_size: usize,
    coalesce_full_packets: bool,
    partial_in: Vec<u8>,
    slots: Option<FirmwareSlots>,
    in_waits: Vec<Duration>,
    out_waits: Vec<Duration>,
    pending_in: bool,
    pending_out: Option<usize>,
    cancelled: bool,
    cancellation_count: usize,
    drops: usize,
    hold_cancellation: bool,
    wait_out_cleanup: bool,
    omit_replies: bool,
    status: u8,
}

type SharedState = Rc<RefCell<State>>;
struct Model(SharedState);

impl Drop for Model {
    fn drop(&mut self) {
        let mut s = self.0.borrow_mut();
        s.drops += 1;
        // Releasing the model's endpoint ownership is not rollback: leave
        // device replies intact, including replies to failed/timed-out writes.
        s.pending_in = false;
        s.pending_out = None;
    }
}

impl Transport for Model {
    fn submit_in(&mut self, reply_max: usize) {
        let mut s = self.0.borrow_mut();
        assert!(!s.pending_in, "a second IN must never be queued");
        s.in_sizes.push(reply_max);
        let allocated = proto::in_len(reply_max, s.packet_size);
        s.allocated_in_sizes.push(allocated);
        s.pending_in = true;
    }

    fn submit_out(&mut self, request: Vec<u8>) -> usize {
        let mut s = self.0.borrow_mut();
        assert!(s.pending_out.is_none(), "a second OUT must never be queued");
        assert!(s.pending_in, "arm IN before OUT");
        let request = proto::pad_for_mps(request, s.packet_size);
        let len = request.len();
        s.pending_out = Some(len);
        if let Some(slots) = s.slots.as_mut() {
            slots.pending_out = Some(request.clone());
        } else if !s.omit_replies {
            let data = if request[0] == CMD_PING {
                vec![1, b'f', b'w']
            } else if s.status == 0 {
                vec![0x55; u16::from_le_bytes([request[6], request[7]]) as usize]
            } else {
                Vec::new()
            };
            let status = if request[0] == CMD_PING { 0 } else { s.status };
            s.replies
                .push_back(reply(request[0], request[1], status, &data));
        }
        s.requests.push(request);
        len
    }

    fn wait_in(&mut self, timeout: Duration) -> Option<Completion> {
        let mut s = self.0.borrow_mut();
        assert!(s.pending_in, "wait on an empty IN queue");
        s.in_waits.push(timeout);
        if s.cancelled {
            if s.hold_cancellation {
                return None;
            }
            s.pending_in = false;
            return Some(completion(Vec::new(), Err(TransferError::Cancelled)));
        }
        match s.in_outcomes.pop_front().unwrap_or(Outcome::Complete) {
            Outcome::Timeout => None,
            Outcome::Error(error) => {
                s.pending_in = false;
                Some(completion(Vec::new(), Err(error)))
            }
            Outcome::Complete => {
                let bytes = if let Some(slots) = s.slots.as_mut() {
                    slots.receive()?
                } else if s.coalesce_full_packets {
                    receive_packets(&mut s)?
                } else {
                    s.replies.pop_front()?
                };
                assert!(bytes.len() <= *s.allocated_in_sizes.last().unwrap());
                s.pending_in = false;
                Some(completion(bytes, Ok(())))
            }
            Outcome::Short(len) => {
                s.pending_in = false;
                Some(completion(vec![0; len], Ok(())))
            }
        }
    }

    fn wait_out(&mut self, timeout: Duration) -> Option<Completion> {
        let mut s = self.0.borrow_mut();
        let len = s.pending_out.expect("wait on an empty OUT queue");
        s.out_waits.push(timeout);
        if s.cancelled {
            if s.wait_out_cleanup {
                std::thread::sleep(timeout);
            }
            if s.hold_cancellation {
                return None;
            }
            s.pending_out = None;
            return Some(completion(Vec::new(), Err(TransferError::Cancelled)));
        }
        match s.out_outcomes.pop_front().unwrap_or(Outcome::Complete) {
            Outcome::Timeout => None,
            Outcome::Error(error) => {
                s.pending_out = None;
                Some(completion(Vec::new(), Err(error)))
            }
            Outcome::Complete => {
                s.pending_out = None;
                if let Some(slots) = s.slots.as_mut() {
                    slots.accept_out();
                }
                Some(completion(vec![0; len], Ok(())))
            }
            Outcome::Short(actual) => {
                s.pending_out = None;
                Some(completion(vec![0; actual], Ok(())))
            }
        }
    }

    fn pending_in(&self) -> usize {
        usize::from(self.0.borrow().pending_in)
    }

    fn pending_out(&self) -> usize {
        usize::from(self.0.borrow().pending_out.is_some())
    }

    fn cancel_all(&mut self) {
        let mut s = self.0.borrow_mut();
        s.cancelled = true;
        s.cancellation_count += 1;
    }
}

// A second, bounded model for PING ordering: exactly one firmware TX slot,
// one received/processing RX slot and one completed host IN. Processing can
// overwrite an unread TX rather than appending to a lossless reply queue.
#[derive(Default)]
struct FirmwareSlots {
    tx: Option<Vec<u8>>,
    rx: Option<Vec<u8>>,
    completed_in: Option<Vec<u8>>,
    pending_out: Option<Vec<u8>>,
    deliver_before_overwrite: bool,
    overwrites: usize,
}

impl FirmwareSlots {
    fn deliver_tx(&mut self) {
        if self.completed_in.is_none() {
            self.completed_in = self.tx.take();
        }
    }

    fn execute_rx(&mut self) {
        if let Some(request) = self.rx.take() {
            let data = if request[0] == CMD_PING {
                vec![1, b'f', b'w']
            } else {
                vec![0x55; u16::from_le_bytes([request[6], request[7]]) as usize]
            };
            self.overwrites += usize::from(self.tx.is_some());
            self.tx = Some(reply(request[0], request[1], 0, &data));
        }
    }

    fn accept_out(&mut self) {
        if self.deliver_before_overwrite {
            self.deliver_tx();
        }
        // The old RX must execute/rearm OUT before the new OUT is accepted.
        self.execute_rx();
        self.rx = self.pending_out.take();
    }

    fn receive(&mut self) -> Option<Vec<u8>> {
        if self.deliver_before_overwrite {
            self.deliver_tx();
        }
        if self.completed_in.is_none() {
            self.execute_rx();
            self.deliver_tx();
        }
        self.completed_in.take()
    }

    fn is_empty(&self) -> bool {
        self.tx.is_none()
            && self.rx.is_none()
            && self.completed_in.is_none()
            && self.pending_out.is_none()
    }
}

// Optional USB packet model: a full packet does not terminate IN. A short
// packet or the requested receive length does, even across protocol replies.
fn receive_packets(s: &mut State) -> Option<Vec<u8>> {
    let capacity = *s.allocated_in_sizes.last().unwrap();
    let mut bytes = std::mem::take(&mut s.partial_in);
    while let Some(frame) = s.replies.pop_front() {
        let take = frame.len().min(capacity - bytes.len());
        bytes.extend_from_slice(&frame[..take]);
        if take < frame.len() {
            s.replies.push_front(frame[take..].to_vec());
        }
        if bytes.len() == capacity || take == 0 || take % s.packet_size != 0 {
            return Some(bytes);
        }
    }
    s.partial_in = bytes;
    None
}

fn completion(bytes: Vec<u8>, status: Result<(), TransferError>) -> Completion {
    Completion {
        actual_len: bytes.len(),
        buffer: Buffer::from(bytes),
        status,
    }
}

fn reply(cmd: u8, seq: u8, status: u8, data: &[u8]) -> Vec<u8> {
    let mut bytes = vec![cmd, seq, status, 0, 0, 0, 0, 0];
    bytes.extend_from_slice(data);
    bytes
}

fn session() -> (Session<Model>, SharedState) {
    let state = Rc::new(RefCell::new(State {
        packet_size: 512,
        ..State::default()
    }));
    (
        Session {
            transport: Some(Model(state.clone())),
            seq: 0,
        },
        state,
    )
}

fn assert_clean(state: &SharedState) {
    let s = state.borrow();
    assert!(!s.pending_in);
    assert!(s.pending_out.is_none());
}

fn assert_invalidated(bridge: &mut Session<Model>, state: &SharedState) {
    assert!(!bridge.is_usable());
    let requests = state.borrow().requests.len();
    assert!(bridge
        .i2c(0x50, &[1], 0)
        .unwrap_err()
        .to_string()
        .contains("reconnect"));
    assert!(bridge
        .ping_line()
        .unwrap_err()
        .to_string()
        .contains("reconnect"));
    assert_eq!(
        state.borrow().requests.len(),
        requests,
        "invalid session submitted more work"
    );
    assert_eq!(state.borrow().cancellation_count, 1);
    assert_eq!(
        state.borrow().drops,
        1,
        "failed session must release transport ownership"
    );
    assert_clean(state);
}

#[test]
fn successful_requests_consume_both_queues_once() {
    let (mut bridge, state) = session();
    assert!(bridge.ping_line().unwrap().contains("protocol 1"));
    assert_eq!(bridge.i2c(0x50, &[0x12], 2).unwrap(), [0x55, 0x55]);
    assert!(bridge.is_usable());
    assert_eq!(state.borrow().requests.len(), 2);
    assert_eq!(state.borrow().cancellation_count, 0);
    assert_clean(&state);
}

#[test]
fn only_exact_legacy_unsupported_capabilities_response_falls_back() {
    let (mut bridge, state) = session();
    state.borrow_mut().omit_replies = true;
    state
        .borrow_mut()
        .replies
        .push_back(reply(CMD_CAPABILITIES, 1, 3, &[]));
    assert_eq!(bridge.capabilities().unwrap(), Capabilities::legacy());
    assert!(bridge.is_usable());
    assert_eq!(state.borrow().requests, [vec![2, 1, 0, 0, 0, 0, 0, 0]]);
    assert_clean(&state);
}

#[test]
fn malformed_or_missing_capabilities_invalidates_without_fallback_or_retry() {
    let mut nonzero_hpm = reply(CMD_CAPABILITIES, 1, 3, &[]);
    nonzero_hpm[4] = 1;
    let mut nonzero_reserved = reply(CMD_CAPABILITIES, 1, 3, &[]);
    nonzero_reserved[3] = 1;
    for malformed in [
        reply(CMD_CAPABILITIES, 0, 3, &[]),
        reply(CMD_I2C, 1, 3, &[]),
        reply(CMD_CAPABILITIES, 1, 3, &[0]),
        reply(CMD_CAPABILITIES, 1, 0, &[]),
        reply(CMD_CAPABILITIES, 1, 2, &[]),
        vec![CMD_CAPABILITIES, 1, 3],
        nonzero_hpm,
        nonzero_reserved,
        vec![],
    ] {
        let (mut bridge, state) = session();
        state.borrow_mut().omit_replies = true;
        if !malformed.is_empty() {
            state.borrow_mut().replies.push_back(malformed);
        }
        assert!(bridge.capabilities().is_err());
        assert_eq!(state.borrow().requests.len(), 1);
        assert_invalidated(&mut bridge, &state);
    }
}

#[test]
fn advertised_extension_is_parsed_and_port_transfer_runs_once() {
    let (mut bridge, state) = session();
    state.borrow_mut().omit_replies = true;
    state.borrow_mut().replies.push_back(reply(
        CMD_CAPABILITIES,
        1,
        0,
        &[
            1, 2, 5, 0, 0, 1, 0, 0, 0xa0, 0x86, 1, 0, 0x10, 0x27, 0, 0, 0x40, 0x42, 0x0f, 0,
        ],
    ));
    let caps = bridge.capabilities().unwrap();
    let request = TransferRequest {
        port: 1,
        addr: 0x50,
        wdata: vec![0x12],
        rlen: 2,
        speed_hz: 400_000,
    };
    caps.validate_transfer(&request).unwrap();
    state
        .borrow_mut()
        .replies
        .push_back(reply(CMD_I2C_PORT, 2, 0, &[0x11, 0x22]));
    assert_eq!(bridge.i2c_port(&request).unwrap(), [0x11, 0x22]);
    assert_eq!(state.borrow().requests.len(), 2);
    assert_eq!(state.borrow().requests[1][0], CMD_I2C_PORT);
    assert_eq!(state.borrow().requests[1][8], 1);
    assert!(bridge.is_usable());
    assert_clean(&state);
}

#[test]
fn port_failure_preserves_no_replay_and_session_rules() {
    let request = TransferRequest {
        port: 1,
        addr: 0x50,
        wdata: vec![0, 1],
        rlen: 0,
        speed_hz: 100_000,
    };
    for status in [1, 2] {
        let (mut bridge, state) = session();
        state.borrow_mut().status = status;
        assert!(bridge.i2c_port(&request).is_err());
        assert!(bridge.is_usable());
        assert_eq!(state.borrow().requests.len(), 1);
    }
    let (mut bridge, state) = session();
    state.borrow_mut().in_outcomes.push_back(Outcome::Timeout);
    let error = bridge.i2c_port(&request).unwrap_err().to_string();
    assert!(error.contains("may have executed and was not retried"));
    assert_eq!(state.borrow().requests.len(), 1);
    assert_invalidated(&mut bridge, &state);
}

fn batch_requests(rlen0: usize, rlen1: usize) -> [TransferRequest; 2] {
    [
        TransferRequest {
            port: 0,
            addr: 0x50,
            wdata: vec![0],
            rlen: rlen0,
            speed_hz: 100_000,
        },
        TransferRequest {
            port: 1,
            addr: 0x50,
            wdata: vec![0],
            rlen: rlen1,
            speed_hz: 100_000,
        },
    ]
}

fn batch_reply(seq: u8, first: &[u8], second: &[u8], second_status: u8) -> Vec<u8> {
    let mut data = vec![2, 0];
    for (port, status, bytes) in [(0u8, 0u8, first), (1, second_status, second)] {
        data.extend_from_slice(&[port, status]);
        data.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        data.extend_from_slice(&(if status == 1 { 2003u32 } else { 0 }).to_le_bytes());
        data.extend_from_slice(bytes);
    }
    let mut frame = reply(CMD_I2C_BATCH, seq, 0, &data);
    if frame.len().is_multiple_of(64) {
        frame.push(0);
    }
    frame
}

#[test]
fn whole_batch_rejection_preserves_session_and_carries_execution_certainty() {
    let (mut bridge, state) = session();
    state.borrow_mut().omit_replies = true;
    state
        .borrow_mut()
        .replies
        .push_back(reply(CMD_I2C_BATCH, 1, 3, &[]));
    let error = bridge.batch(&batch_requests(0, 0)).unwrap_err();
    assert!(error.is::<BatchRejectedBeforeExecution>());
    assert!(bridge.is_usable());
    assert_eq!(state.borrow().requests.len(), 1);
    assert_eq!(state.borrow().cancellation_count, 0);
    assert_clean(&state);
}

#[test]
fn batch_partial_failure_is_a_confirmed_result_without_replay() {
    let (mut bridge, state) = session();
    state.borrow_mut().omit_replies = true;
    state
        .borrow_mut()
        .replies
        .push_back(batch_reply(1, &[0xaa], &[], 1));
    let outcomes = bridge.batch(&batch_requests(1, 20)).unwrap();
    assert_eq!(outcomes[0].data, [0xaa]);
    assert_eq!(
        (outcomes[1].status, outcomes[1].hpm_stat),
        (Status::Nack, 2003)
    );
    assert!(bridge.is_usable());
    assert_eq!(state.borrow().requests.len(), 1);
    assert_clean(&state);
}

#[test]
fn batch_invalid_second_result_invalidates_without_publishing_partial_success() {
    let mut wrong_port = batch_reply(1, &[0xaa], &[0xbb], 0);
    wrong_port[HDR + 2 + 8 + 1] = 0;
    let mut unknown_status = batch_reply(1, &[0xaa], &[0xbb], 0);
    unknown_status[HDR + 2 + 8 + 1 + 1] = 99;
    for malformed in [wrong_port, unknown_status, batch_reply(1, &[0xaa], &[], 0)] {
        let (mut bridge, state) = session();
        state.borrow_mut().omit_replies = true;
        state.borrow_mut().replies.push_back(malformed);
        assert!(bridge.batch(&batch_requests(1, 1)).is_err());
        assert_eq!(state.borrow().requests.len(), 1);
        assert_invalidated(&mut bridge, &state);
    }
}

#[test]
fn full_speed_partial_batch_uses_short_packet_padding_not_the_maximum_in_size() {
    let (mut bridge, state) = session();
    {
        let mut s = state.borrow_mut();
        s.packet_size = 64;
        s.coalesce_full_packets = true;
        s.omit_replies = true;
        let frame = batch_reply(1, &[0x55; 38], &[], 1);
        assert_eq!(frame.len(), 65); // unpadded response would stall at exactly one packet
        s.replies.push_back(frame);
    }
    let outcomes = bridge.batch(&batch_requests(38, 100)).unwrap();
    assert_eq!(outcomes[0].data.len(), 38);
    assert_eq!(outcomes[1].status, Status::Nack);
    assert_eq!(state.borrow().allocated_in_sizes, [192]);
    assert_clean(&state);

    let (mut bridge, state) = session();
    {
        let mut s = state.borrow_mut();
        s.packet_size = 64;
        s.coalesce_full_packets = true;
        s.omit_replies = true;
        let mut frame = batch_reply(1, &[0x55; 38], &[], 1);
        frame.pop();
        s.replies.push_back(frame);
    }
    assert!(bridge.batch(&batch_requests(38, 100)).is_err());
    assert_invalidated(&mut bridge, &state);
}

#[test]
fn reconnect_discards_maximum_and_exact_packet_stale_batch_responses() {
    for packet_size in [64, 512] {
        for stale in [
            batch_reply(0, &[0xaa; 240], &[0xbb; 240], 0),
            batch_reply(0, &[0xaa; 38], &[], 1),
        ] {
            let (mut bridge, state) = session();
            {
                let mut s = state.borrow_mut();
                s.packet_size = packet_size;
                s.coalesce_full_packets = true;
                s.replies.push_back(stale);
            }
            bridge.synchronize().unwrap();
            assert!(bridge.is_usable());
            assert!(state.borrow().in_sizes.iter().all(|&n| n == MAX_FRAME));
            assert!(state.borrow().requests.iter().all(|r| r[0] == CMD_PING));
            assert_clean(&state);
        }
    }
}

#[test]
fn firmware_errors_leave_a_usable_session_without_automatic_retry() {
    for (status, name) in [
        (1, "Nack"),
        (2, "Timeout"),
        (3, "BadRequest"),
        (4, "I2cError"),
    ] {
        let (mut bridge, state) = session();
        state.borrow_mut().status = status;
        let error = bridge.i2c(0x50, &[0x12], 0).unwrap_err().to_string();
        assert!(error.contains(name));
        assert!(!error.contains("unusable"));
        assert!(bridge.is_usable());
        assert_eq!(state.borrow().requests.len(), 1);
        assert_clean(&state);
        state.borrow_mut().status = 0;
        assert_eq!(bridge.i2c(0x50, &[], 1).unwrap(), [0x55]);
        assert_eq!(state.borrow().cancellation_count, 0);
    }
}

#[test]
fn in_timeout_with_late_reply_cancels_and_denies_reuse() {
    let (mut bridge, state) = session();
    state.borrow_mut().in_outcomes.push_back(Outcome::Timeout);
    let error = bridge.i2c(0x50, &[0x12, 0x34], 0).unwrap_err().to_string();
    assert!(error.contains("timeout waiting for reply"));
    assert!(error.contains("may have executed and was not retried"));
    assert!(error.contains("cancellation cannot undo"));
    assert_eq!(state.borrow().requests.len(), 1);
    assert_eq!(
        state.borrow().replies.len(),
        1,
        "cancelling USB must not undo a device reply"
    );
    assert_invalidated(&mut bridge, &state);
}

#[test]
fn missing_reply_also_invalidates_without_replay() {
    let (mut bridge, state) = session();
    state.borrow_mut().omit_replies = true;
    assert!(bridge
        .i2c(0x50, &[1], 0)
        .unwrap_err()
        .to_string()
        .contains("timeout"));
    assert_eq!(state.borrow().requests.len(), 1);
    assert_invalidated(&mut bridge, &state);
}

#[test]
fn out_timeout_cancels_and_drains_both_queues() {
    let (mut bridge, state) = session();
    state.borrow_mut().out_outcomes.push_back(Outcome::Timeout);
    let error = bridge.i2c(0x50, &[1], 0).unwrap_err().to_string();
    assert!(error.contains("timeout sending request"));
    assert!(error.contains("may have executed"));
    assert_eq!(state.borrow().requests.len(), 1);
    assert_eq!(state.borrow().out_waits.len(), 2); // failed wait + cancellation
    assert_eq!(state.borrow().in_waits.len(), 1); // cancellation, no normal read
    assert_invalidated(&mut bridge, &state);
}

#[test]
fn out_errors_clean_the_armed_in_transfer() {
    for error in [
        TransferError::Disconnected,
        TransferError::Stall,
        TransferError::Cancelled,
    ] {
        let (mut bridge, state) = session();
        state
            .borrow_mut()
            .out_outcomes
            .push_back(Outcome::Error(error));
        let message = bridge.i2c(0x50, &[1], 0).unwrap_err().to_string();
        assert!(message.contains("sending request"));
        assert!(message.contains("may have executed"));
        assert_eq!(state.borrow().out_waits.len(), 1);
        assert_eq!(state.borrow().in_waits.len(), 1);
        assert_invalidated(&mut bridge, &state);
    }
}

#[test]
fn in_errors_invalidate_even_when_out_completed() {
    let (mut bridge, state) = session();
    state
        .borrow_mut()
        .in_outcomes
        .push_back(Outcome::Error(TransferError::Disconnected));
    assert!(bridge
        .i2c(0x50, &[], 1)
        .unwrap_err()
        .to_string()
        .contains("receiving reply"));
    assert_invalidated(&mut bridge, &state);
}

#[test]
fn incomplete_out_completion_is_not_success() {
    let (mut bridge, state) = session();
    state
        .borrow_mut()
        .out_outcomes
        .push_back(Outcome::Short(HDR));
    assert!(bridge
        .i2c(0x50, &[1], 0)
        .unwrap_err()
        .to_string()
        .contains("short USB request"));
    assert_invalidated(&mut bridge, &state);
}

#[test]
fn stale_reply_in_established_session_fails_closed_without_retry() {
    for stale in [reply(CMD_I2C, 0, 0, &[]), reply(CMD_PING, 1, 0, &[1])] {
        let (mut bridge, state) = session();
        state.borrow_mut().replies.push_back(stale);
        // Reserve enough space that a stale PING fits in this I2C receive.
        assert!(bridge
            .i2c(0x50, &[1], 1)
            .unwrap_err()
            .to_string()
            .contains("expected cmd"));
        assert_eq!(state.borrow().requests.len(), 1);
        assert_invalidated(&mut bridge, &state);
    }
}

#[test]
fn short_reply_and_incorrect_data_length_invalidate() {
    let (mut bridge, state) = session();
    state
        .borrow_mut()
        .in_outcomes
        .push_back(Outcome::Short(HDR - 1));
    assert!(bridge
        .i2c(0x50, &[1], 0)
        .unwrap_err()
        .to_string()
        .contains("short reply"));
    assert_invalidated(&mut bridge, &state);

    let (mut bridge, state) = session();
    state
        .borrow_mut()
        .replies
        .push_back(reply(CMD_I2C, 1, 0, &[1]));
    assert!(bridge
        .i2c(0x50, &[], 2)
        .unwrap_err()
        .to_string()
        .contains("expected 2 bytes"));
    assert_invalidated(&mut bridge, &state);
}

#[test]
fn invalid_request_does_not_touch_usb_or_poison_session() {
    let (mut bridge, state) = session();
    assert!(bridge.i2c(0x80, &[1], 0).is_err());
    assert!(bridge.i2c(0x50, &[], 0).is_err());
    assert!(bridge.i2c(0x50, &[0; MAX_XFER + 1], 0).is_err());
    assert!(bridge.is_usable());
    assert!(state.borrow().requests.is_empty());
    assert!(state.borrow().in_sizes.is_empty());
    assert_eq!(bridge.i2c(0x50, &[], 1).unwrap(), [0x55]);
}

#[test]
fn cancellation_that_never_completes_has_one_shared_bounded_wait() {
    let (mut bridge, state) = session();
    {
        let mut s = state.borrow_mut();
        s.out_outcomes.push_back(Outcome::Timeout);
        s.hold_cancellation = true;
        s.wait_out_cleanup = true;
    }
    let start = Instant::now();
    let error = bridge.i2c(0x50, &[1], 0).unwrap_err().to_string();
    assert!(error.contains("cancellation is still pending"));
    assert!(start.elapsed() < Duration::from_secs(2));
    {
        let s = state.borrow();
        assert_eq!(s.out_waits.len(), 2);
        assert!(s.out_waits[1] <= CLEANUP_TIMEOUT);
        assert_eq!(
            s.in_waits,
            [Duration::ZERO],
            "IN must not start a new cleanup budget"
        );
    }
    assert_invalidated(&mut bridge, &state);
}

#[test]
fn reconnect_discards_a_full_sized_stale_i2c_reply_using_only_ping() {
    let (mut bridge, state) = session();
    state
        .borrow_mut()
        .replies
        .push_back(reply(CMD_I2C, 1, 0, &[0xAB; MAX_XFER]));
    bridge.synchronize().unwrap();
    assert!(bridge.is_usable());
    let s = state.borrow();
    assert_eq!(s.requests.len(), 3);
    assert!(s.requests.iter().all(|r| r[0] == CMD_PING));
    assert!(s.in_sizes.iter().all(|&len| len == MAX_FRAME));
    assert!(s.replies.is_empty());
    drop(s);
    assert_clean(&state);
}

#[test]
fn colliding_stale_ping_requires_a_second_distinct_ping() {
    let (mut bridge, state) = session();
    state
        .borrow_mut()
        .replies
        .push_back(reply(CMD_PING, 1, 0, &[1]));
    bridge.synchronize().unwrap();
    let s = state.borrow();
    assert_eq!(
        s.requests.iter().map(|r| r[1]).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(s.in_waits.len(), 4); // stale 1, fresh 1, fresh 2, fresh 3
    assert!(s.replies.is_empty());
    drop(s);
    assert_clean(&state);
}

#[test]
fn reconnect_does_not_replay_a_failed_i2c_write() {
    let (mut bridge, state) = session();
    state.borrow_mut().in_outcomes.push_back(Outcome::Timeout);
    assert!(bridge.i2c(0x50, &[0xAB], 0).is_err());
    assert_invalidated(&mut bridge, &state);
    // A new transport/session, but the device can still have the old reply.
    state.borrow_mut().cancelled = false;
    let mut reopened = Session {
        transport: Some(Model(state.clone())),
        seq: 0,
    };
    reopened.synchronize().unwrap();
    assert_eq!(
        state
            .borrow()
            .requests
            .iter()
            .filter(|r| r[0] == CMD_I2C)
            .count(),
        1
    );
    assert_eq!(reopened.i2c(0x50, &[], 1).unwrap(), [0x55]);
    assert_eq!(
        state
            .borrow()
            .requests
            .iter()
            .filter(|r| r[0] == CMD_I2C)
            .count(),
        2
    );
    assert_clean(&state);
}

#[test]
fn synchronization_rejects_missing_replies_and_limits_stale_frames() {
    let (mut bridge, state) = session();
    state.borrow_mut().omit_replies = true;
    assert!(bridge
        .synchronize()
        .unwrap_err()
        .to_string()
        .contains("timeout"));
    assert_eq!(state.borrow().requests.len(), 1);
    assert_invalidated(&mut bridge, &state);

    let (mut bridge, state) = session();
    state
        .borrow_mut()
        .replies
        .extend((0..MAX_SYNC_REPLIES).map(|_| reply(CMD_I2C, 0, 0, &[])));
    assert!(bridge
        .synchronize()
        .unwrap_err()
        .to_string()
        .contains("too many stale replies"));
    assert_eq!(state.borrow().in_waits.len(), MAX_SYNC_REPLIES);
    assert!(state.borrow().requests.iter().all(|r| r[0] == CMD_PING));
    assert_invalidated(&mut bridge, &state);
}

#[test]
fn synchronization_fails_closed_after_an_out_error() {
    let (mut bridge, state) = session();
    state
        .borrow_mut()
        .out_outcomes
        .push_back(Outcome::Error(TransferError::Stall));
    assert!(bridge.synchronize().is_err());
    assert_eq!(state.borrow().requests.len(), 1);
    assert_invalidated(&mut bridge, &state);
}

#[test]
fn invalid_ping_or_unsupported_protocol_never_establishes_session() {
    for data in [vec![], vec![2], vec![1; PING_DATA_MAX + 1]] {
        let (mut bridge, state) = session();
        state
            .borrow_mut()
            .replies
            .push_back(reply(CMD_PING, 1, 0, &[1]));
        state
            .borrow_mut()
            .replies
            .push_back(reply(CMD_PING, 2, 0, &data));
        assert!(bridge.synchronize().is_err());
        assert_invalidated(&mut bridge, &state);
    }
}

#[test]
fn full_speed_exact_packet_replies_use_the_exact_receive_size() {
    // With no terminating ZLP, asking for even one extra packet would leave
    // these healthy replies pending. Keep each normal IN sized to its request.
    let (mut bridge, state) = session();
    state.borrow_mut().packet_size = 64;
    state.borrow_mut().coalesce_full_packets = true;
    for rlen in [56, 120, 184, 248] {
        assert_eq!(bridge.i2c(0x50, &[], rlen).unwrap(), vec![0x55; rlen]);
        assert_eq!(*state.borrow().in_sizes.last().unwrap(), HDR + rlen);
        assert_eq!(
            *state.borrow().allocated_in_sizes.last().unwrap(),
            HDR + rlen
        );
        assert_clean(&state);
    }
    assert!(bridge.is_usable());
}

#[test]
fn full_speed_padded_out_completion_is_checked_against_padded_length() {
    let (mut bridge, state) = session();
    state.borrow_mut().packet_size = 64;
    bridge.i2c(0x50, &[0xAA; 56], 0).unwrap();
    assert_eq!(state.borrow().requests[0].len(), 65);
    assert!(bridge.is_usable());
    assert_clean(&state);
}

#[test]
fn initial_drain_recovers_coalesced_full_speed_stale_replies() {
    for rlen in [56, 120, 184, 248, MAX_XFER] {
        let (mut bridge, state) = session();
        {
            let mut s = state.borrow_mut();
            s.packet_size = 64;
            s.coalesce_full_packets = true;
            s.replies
                .push_back(reply(CMD_I2C, 99, 0, &vec![0xAA; rlen]));
        }
        bridge.synchronize().unwrap();
        {
            let s = state.borrow();
            assert_eq!(s.requests.len(), 3);
            assert!(s.requests.iter().all(|r| r[0] == CMD_PING));
            assert!(s.replies.is_empty());
            assert!(s.partial_in.is_empty());
        }
        assert_eq!(bridge.i2c(0x50, &[], 56).unwrap(), [0x55; 56]);
        assert_clean(&state);
    }
}

#[test]
fn initial_drain_prevents_two_old_ping_collisions_from_establishing_session() {
    for sequences in [[1, 2], [2, 3]] {
        let (mut bridge, state) = session();
        {
            let mut s = state.borrow_mut();
            s.packet_size = 64;
            s.coalesce_full_packets = true;
            for seq in sequences {
                s.replies.push_back(reply(CMD_PING, seq, 0, &[1]));
            }
        }
        bridge.synchronize().unwrap();
        {
            let s = state.borrow();
            assert_eq!(
                s.requests.iter().map(|r| r[1]).collect::<Vec<_>>(),
                [1, 2, 3]
            );
            assert!(
                s.replies.is_empty(),
                "all old replies and fresh PINGs must be consumed"
            );
            assert_eq!(s.in_waits.len(), 5);
        }
        assert_eq!(bridge.i2c(0x50, &[], 1).unwrap(), [0x55]);
        assert_clean(&state);
    }
}

#[test]
fn warmup_handles_delayed_rx_execution_and_tx_overwrite() {
    for sequences in [[1, 2], [2, 3]] {
        for deliver_before_overwrite in [false, true] {
            let (mut bridge, state) = session();
            state.borrow_mut().slots = Some(FirmwareSlots {
                tx: Some(reply(CMD_PING, sequences[0], 0, &[1])),
                rx: Some(proto::encode_ping(sequences[1])),
                deliver_before_overwrite,
                ..FirmwareSlots::default()
            });
            bridge.synchronize().unwrap();
            {
                let s = state.borrow();
                assert_eq!(s.requests.len(), 3);
                assert!(s.requests.iter().all(|r| r[0] == CMD_PING));
                let slots = s.slots.as_ref().unwrap();
                assert!(
                    slots.is_empty(),
                    "successful setup must leave no old or fresh PING work"
                );
                if !deliver_before_overwrite {
                    assert!(slots.overwrites > 0, "exercise loss of an unread TX reply");
                }
            }
            assert_eq!(bridge.i2c(0x50, &[], 1).unwrap(), [0x55]);
            assert!(state.borrow().slots.as_ref().unwrap().is_empty());
            assert_clean(&state);
        }
    }
}

#[test]
fn initial_drain_discards_fragmented_old_i2c_tails() {
    for rlen in [64, 120, 184, 248, MAX_XFER] {
        let (mut bridge, state) = session();
        {
            let mut s = state.borrow_mut();
            s.packet_size = 64;
            s.coalesce_full_packets = true;
            // The previous host receive consumed the first 64-byte packet
            // before cancellation. The remaining data has no protocol header.
            let old = reply(CMD_I2C, 99, 0, &vec![0xAA; rlen]);
            s.replies.push_back(old[64..].to_vec());
        }
        bridge.synchronize().unwrap();
        assert!(state.borrow().replies.is_empty());
        assert!(state.borrow().partial_in.is_empty());
        assert_eq!(bridge.i2c(0x50, &[], 1).unwrap(), [0x55]);
        assert_clean(&state);
    }
}

#[test]
fn lost_confirmation_after_initial_drain_fails_closed() {
    let (mut bridge, state) = session();
    {
        let mut s = state.borrow_mut();
        s.packet_size = 64;
        s.omit_replies = true;
        // Initial IN includes a full stale packet and the initial short PING.
        // The later confirmation is lost, as can happen on a broken link or
        // with the firmware's shared TX buffer. Recovery must not assume FIFO.
        let mut combined = reply(CMD_I2C, 99, 0, &[0xAA; 56]);
        combined.extend_from_slice(&reply(CMD_PING, 1, 0, &[1, b'f', b'w']));
        s.replies.push_back(combined);
    }
    let error = bridge.synchronize().unwrap_err().to_string();
    assert!(error.contains("timeout waiting for reply"));
    assert!(error.contains("reconnect"));
    assert_eq!(state.borrow().requests.len(), 2);
    assert!(state.borrow().requests.iter().all(|r| r[0] == CMD_PING));
    assert_invalidated(&mut bridge, &state);
}

#[test]
fn unknown_or_malformed_status_is_not_a_normal_i2c_failure() {
    for stale in [reply(CMD_I2C, 1, 99, &[]), reply(CMD_I2C, 1, 1, &[0xAA])] {
        let (mut bridge, state) = session();
        state.borrow_mut().replies.push_back(stale);
        assert!(bridge
            .i2c(0x50, &[], 1)
            .unwrap_err()
            .to_string()
            .contains("invalid I2C status"));
        assert_invalidated(&mut bridge, &state);
    }
}
