//! Public boot identity and allocation report.
use crate::heap;
use core::fmt::Write as _;
/// Boot line naming the node and its heap cost. Shows only the public destination.
pub fn describe_node(node: Option<&retinue::node::Node<32, 8, 4>>, out: &mut [u8; 64]) -> usize {
    let mut text = radio_face::Text::<64>::empty();
    match node {
        Some(node) => {
            let dest = node.destination();
            let bytes = dest.as_slice();
            let _ = write!(
                &mut text,
                "node={:02x}{:02x}{:02x}{:02x} heap={}/{} highwater={}\r\n",
                bytes[0],
                bytes[1],
                bytes[2],
                bytes[3],
                heap::used(),
                heap::HEAP_SIZE,
                heap::high_water(),
            );
        }
        None => {
            let _ = write!(&mut text, "node=unavailable\r\n");
        }
    }
    let source = text.as_str().as_bytes();
    let len = source.len().min(out.len());
    out[..len].copy_from_slice(&source[..len]);
    len
}
pub fn publish_fault(status: &mut radio_face::LocalStatus, code: u8, message: &'static str) {
    status.radio = radio_face::RadioState::Fault;
    status.fault = Some(radio_face::Fault {
        code,
        message: radio_face::Text::from_truncated(message),
    });
    crate::ui::publish(*status, radio_face::LedSignal::Idle);
}

pub fn publish_online(status: &mut radio_face::LocalStatus) {
    status.radio = radio_face::RadioState::Online;
    status.fault = None;
    crate::ui::publish(*status, radio_face::LedSignal::Idle);
}
