//! The canvas stays committed until the authority confirms the latest view.
//!
//! The client may ask the authority for a larger room range before it draws
//! a larger frame. A request carries a sequence number and the desired frame;
//! only a matching acknowledgement lets the caller commit.

use super::{adaptive_integer::FrameSelection, camera::FrameSize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewRequest {
    pub seq: u32,
    pub frame: FrameSize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewDecision {
    /// Keep the current canvas and wait for the authority.
    Keep,
    /// Send this request and keep the current canvas until its answer.
    Request(ViewRequest),
    /// The authority answered the latest request: publish this canvas.
    Commit(FrameSelection),
}

#[derive(Default, Debug)]
pub struct ViewSync {
    next: u32,
    pending: Option<ViewRequest>,
}

impl ViewSync {
    /// Decide what to do for `desired`, given the authority's latest `ack`.
    /// A stale answer (an earlier sequence) never commits.
    pub fn poll(
        &mut self,
        current: FrameSize,
        desired: FrameSelection,
        ack: Option<ViewRequest>,
    ) -> ViewDecision {
        if let Some(pending) = self.pending {
            if pending.frame == desired.frame {
                if ack == Some(pending) {
                    return ViewDecision::Commit(desired);
                }
                return ViewDecision::Keep;
            }
        } else if current == desired.frame {
            return ViewDecision::Commit(desired);
        }
        self.next = self.next.wrapping_add(1);
        let request = ViewRequest {
            seq: self.next,
            frame: desired.frame,
        };
        self.pending = Some(request);
        ViewDecision::Request(request)
    }

    /// Send `frame` even when it already matches the local canvas (the
    /// authority may still hold an older room range after loading).
    pub fn request(&mut self, frame: FrameSize) -> ViewRequest {
        self.next = self.next.wrapping_add(1);
        let request = ViewRequest {
            seq: self.next,
            frame,
        };
        self.pending = Some(request);
        request
    }

    /// A request is waiting for its acknowledgement.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn committed(&mut self) {
        self.pending = None;
    }

    pub fn send_failed(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::ScaleMode;

    fn desired(w: u32, h: u32, mode: ScaleMode) -> FrameSelection {
        FrameSelection::for_window(w, h, FrameSize::WIDE, mode)
    }

    fn request(d: ViewDecision) -> ViewRequest {
        let ViewDecision::Request(r) = d else {
            panic!("expected request")
        };
        r
    }

    #[test]
    fn rapid_matrix_changes_publish_only_the_latest_authority_reply() {
        let mut sync = ViewSync::default();
        let current = FrameSize::WIDE;
        let first = desired(1920, 1080, ScaleMode::Integer);
        let old = request(sync.poll(current, first, None));
        let tall = desired(800, 1600, ScaleMode::Integer);
        let latest = request(sync.poll(current, tall, Some(old)));
        assert_eq!(sync.poll(current, tall, Some(old)), ViewDecision::Keep);
        assert_eq!(
            sync.poll(current, tall, Some(latest)),
            ViewDecision::Commit(tall)
        );
        sync.committed();
        let fit = desired(800, 1600, ScaleMode::Fit);
        let back = request(sync.poll(tall.frame, fit, Some(latest)));
        assert_eq!(
            sync.poll(tall.frame, fit, Some(back)),
            ViewDecision::Commit(fit)
        );
    }

    #[test]
    fn returning_to_the_current_canvas_cancels_an_inflight_larger_request() {
        let mut sync = ViewSync::default();
        let current = FrameSize::WIDE;
        let old = request(sync.poll(current, desired(800, 1600, ScaleMode::Integer), None));
        let fit = desired(1920, 1080, ScaleMode::Fit);
        let cancel = request(sync.poll(current, fit, Some(old)));
        assert_ne!(old.seq, cancel.seq);
        assert_eq!(sync.poll(current, fit, Some(old)), ViewDecision::Keep);
        assert_eq!(
            sync.poll(current, fit, Some(cancel)),
            ViewDecision::Commit(fit)
        );
    }
}
