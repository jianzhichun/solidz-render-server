//! The UI's file pickers, answered by the host (design §1.8): every "save to file" and "open a
//! file" the UI asks for becomes an `ask_save` / `ask_open`, and its picker future
//! (`crates/ui-egui/src/file_picker.rs:14`, polled by `file_picker::tick` every pass, `:123-128`)
//! resolves when the host's `answer` comes. A picker program (`pick_program`, the slicer) is
//! not offered: `None` answers as a cancel (`file_picker.rs:73-80`). Everything else a desktop host offers (show in
//! folder, downloads, updates, keychain, agent HTTP, slicer hand-off) stays `None`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

// `crates/ui-egui/src/lib.rs:201` re-exports the private module's type.
use solidcraft_ui_egui::{FilePickerFuture, Services};

use crate::domain::wire::ServerMsg;

/// The asks waiting for the host, and the ones a pass has just handed to the UI.
#[derive(Default)]
struct Table {
    next: u32,
    waiting: HashMap<u32, Slot>,
    /// Answers the UI took this pass: `(ask, a path was given)`; `ask_done` follows the pass.
    delivered: Vec<(u32, bool)>,
}

#[derive(Default)]
struct Slot {
    answer: Option<Option<String>>,
    waker: Option<Waker>,
}

/// The asks of this process. Lives on the frame loop's thread with the UI (`Services` holds
/// closures that are not `Send`, `crates/ui-egui/src/lib.rs:487-540`).
#[derive(Clone, Default)]
pub struct Asks {
    table: Rc<RefCell<Table>>,
}

impl Asks {
    /// The `Services` whose pickers ask the host through `send`.
    pub fn services(&self, send: Rc<dyn Fn(ServerMsg)>) -> Services {
        let (open, file, save) = (self.clone(), self.clone(), self.clone());
        let (send_open, send_file, send_save) = (send.clone(), send.clone(), send);
        Services {
            pick_open: Some(Box::new(move |exts: &[&str]| open.ask(&*send_open, |ask| ServerMsg::AskOpen { ask, exts: owned(exts) }))),
            // One kind of file (the agent chat's attachments): the host offers the same chooser.
            pick_file: Some(Box::new(move |_label: &str, exts: &[&str]| file.ask(&*send_file, |ask| ServerMsg::AskOpen { ask, exts: owned(exts) }))),
            pick_save: Some(Box::new(move |name: &str, exts: &[&str]| {
                save.ask(&*send_save, |ask| ServerMsg::AskSave { ask, name: name.to_string(), exts: owned(exts) })
            })),
            ..Services::default()
        }
    }

    /// The host answered `ask` (`None`: cancelled). An answer to an ask that is not waiting
    /// (answered twice, or never asked) is ignored.
    pub fn answer(&self, ask: u32, path: Option<String>) {
        let waker = {
            let mut table = self.table.borrow_mut();
            // Already answered but not yet taken by the UI's poll: the first answer stands.
            let Some(slot) = table.waiting.get_mut(&ask).filter(|s| s.answer.is_none()) else { return };
            slot.answer = Some(path);
            slot.waker.take()
        };
        // `file_picker::Repaint` (`file_picker.rs:22-28`): the context repaints. Outside the borrow.
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// The answers the UI took since the last call, for `ask_done` after the pass that ran them.
    pub fn take_delivered(&self) -> Vec<(u32, bool)> {
        std::mem::take(&mut self.table.borrow_mut().delivered)
    }

    fn ask(&self, send: &dyn Fn(ServerMsg), msg: impl FnOnce(u32) -> ServerMsg) -> FilePickerFuture {
        let ask = {
            let mut table = self.table.borrow_mut();
            table.next += 1;
            let ask = table.next;
            table.waiting.insert(ask, Slot::default());
            ask
        };
        send(msg(ask));
        Box::pin(Answer { ask, table: self.table.clone() })
    }
}

fn owned(exts: &[&str]) -> Vec<String> {
    exts.iter().map(|e| e.trim_start_matches('.').to_string()).collect()
}

/// One ask's picker future.
struct Answer {
    ask: u32,
    table: Rc<RefCell<Table>>,
}

impl Future for Answer {
    type Output = Option<String>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<String>> {
        let mut table = self.table.borrow_mut();
        let Some(slot) = table.waiting.get_mut(&self.ask) else { return Poll::Ready(None) };
        match slot.answer.take() {
            Some(path) => {
                table.waiting.remove(&self.ask);
                table.delivered.push((self.ask, path.is_some()));
                Poll::Ready(path)
            }
            None => {
                slot.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An ask goes to the host at once; its future waits until the answer, which is reported
    /// as delivered once (for `ask_done`); a second answer changes nothing.
    #[test]
    fn a_save_ask_waits_for_the_hosts_answer() {
        let asks = Asks::default();
        let sent = Rc::new(RefCell::new(Vec::new()));
        let log = sent.clone();
        let services = asks.services(Rc::new(move |m: ServerMsg| log.borrow_mut().push(m)));
        let mut future = (services.pick_save.as_ref().unwrap())("part.step", &["step", ".stp"]);
        assert_eq!(*sent.borrow(), vec![ServerMsg::AskSave { ask: 1, name: "part.step".into(), exts: vec!["step".into(), "stp".into()] }]);
        let mut cx = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut cx).is_pending());
        asks.answer(1, Some("/staging/part.step".into()));
        asks.answer(1, None);
        assert_eq!(future.as_mut().poll(&mut cx), Poll::Ready(Some("/staging/part.step".into())));
        assert_eq!(asks.take_delivered(), vec![(1, true)]);
        assert!(asks.take_delivered().is_empty());
    }
}
